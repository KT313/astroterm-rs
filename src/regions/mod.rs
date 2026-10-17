//! Step 4 of the frame: `refresh_regions`.
//!
//! Inputs: the caches, the star tables, the selected regions, the simulated time and the render settings. Output:
//! up-to-date group images and region images for every selected region, and the constellation table's
//! simulation. Real work happens per group only when a cache key changed (plan section 4): a group is simulated
//! again when its hold time has expired, its image is drawn again when its key changed, and a region image is
//! merged again when one of its group images was. Zoom changes redraw selected tiles with boosted per-star opacity. Rotation, panning, resize and time inside
//! the holds do not change existing image keys. Images are never dropped: a region that leaves the view keeps its images for its return
//! (owner's decision of 2026-10-12; the memory is bounded by the fixed tile size times the region count).
//!
//! The module holds the simulation caches (`simulation`), the star appearance
//! (`appearance`), the tile geometry (`tiles`) and the image caches (`images`).
mod appearance;
mod content_bounds;
mod filter_summary;
mod images;
mod simulation;
mod tiles;
pub use appearance::{STAR_GLYPH_RADIUS, blend_star, order_star_rows, premultiplied_color, star_opacity, zoom_opacity_boost};
use crate::model::GroupTileKey;
pub use simulation::{Refresh, UNPLACED, rows_wanted};
pub use tiles::TileGeometry;

use crate::time::StepTimes;
use crate::formulas::AstronomicalTime;
use crate::catalog::magnitude_code;
use crate::constants::SPEED_GROUP_COUNT;
use crate::model::{Region, StarTable, SimulationCache, StarOpacities, get_ttl_of_speed_group_ms};
#[cfg(test)]
use crate::model::{Data, RenderOptions};
#[cfg(test)]
use crate::selection::Selected;

/// What one refresh did, for timing and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub simulated: usize,                   // groups whose simulation was refreshed or extended
    pub rendered: usize,                    // group images drawn again (empty prefixes do not count)
    pub merged: usize,                      // region images merged again
}

/// Bring the images of the selected regions and the constellation simulation up to date for this frame.
#[allow(clippy::too_many_arguments)]
pub fn refresh_regions(
    star_regions: &mut [Region],
    constellation_stars: &StarTable,
    constellation_simulation: &mut SimulationCache,
    constellation_last_update_ms: &mut Option<i64>,
    star_opacities: &mut StarOpacities,
    tile_bytes: &mut usize,
    selected_regions: &[usize],
    time_ms: i64,
    fov_degrees: f64,
    magnitude_threshold: f64,
    times: &mut StepTimes,
) -> RefreshReport {
    // Start the refresh counters and read the requested simulation time.
    let mut report = RefreshReport::default();
    let tt = AstronomicalTime::from_time_ms(time_ms).tt_jd;

    // Prepare star opacities for the current zoom, reusing the lookup table when unchanged.
    let zoom_boost = zoom_opacity_boost(fov_degrees);
    times.measure("Regions.Opacity table");
    star_opacities.prepare(zoom_boost);

    // Read the brightness limit and encode it for comparisons with stored star magnitudes.
    let threshold = magnitude_threshold;
    let threshold_code = magnitude_code(threshold);

    // Refresh the constellation cache when needed, covering every endpoint star with the fastest group's TTL.
    times.measure("Regions.Constellation simulation");
    {
        let star_count = constellation_stars.len();
        let fastest_group_index = SPEED_GROUP_COUNT - 1;
        let ttl_ms = get_ttl_of_speed_group_ms(fastest_group_index);

        constellation_simulation.update(
            constellation_stars,
            time_ms,
            tt,
            star_count,
            ttl_ms,
            constellation_last_update_ms,
        )
    };

    // Refresh each selected region, accumulating repeated timing paths within the frame.
    for &region_index in selected_regions {
        // Prepare the sky-to-tile mapping shared by this region's groups.
        times.measure("Regions.Tile geometry");
        let tile = TileGeometry::new(region_index);
        let region = &mut star_regions[region_index];

        for group in &mut region.speed_groups {
            // Find the stars that could be bright enough to draw.
            let table = &group.stars;
            times.measure("Regions.Brightness prefixes");
            let star_count = rows_wanted(table, tt, threshold);

            // Clear the old image if this group has no stars to draw.
            if star_count == 0 {
                *tile_bytes -= group.tile.bytes();
                times.measure("Regions.Group image rendering");
                group.tile.render(GroupTileKey::EMPTY, &tile, table, &[], &[], star_opacities);
                continue;
            }

            // Reuse star samples, or calculate them when missing or expired.
            times.measure("Regions.Simulation caches");
            let refresh = group.update_simulation(time_ms, tt, rows_wanted(&group.stars, tt, threshold));
            if refresh != Refresh::Unchanged {
                report.simulated += 1;
            }
            let table = &group.stars;
            let simulation = &group.simulation;

            // Describe the inputs used to draw this group's image.
            times.measure("Regions.Image cache decisions");
            let image_key = GroupTileKey {
                simulated_at_ms: group.last_update_ms.expect("a simulated group has an epoch"),
                star_count,
                magnitude_threshold: threshold_code,
                zoom_boost,
            };

            // Keep the existing image when those inputs have not changed.
            let image = &mut group.tile;
            times.measure("Regions.Image cache decisions");
            let image_is_current = image.key() == Some(image_key);
            if image_is_current {
                continue;
            }

            // Redraw the image and account for any change in its memory use.
            *tile_bytes -= image.bytes();
            times.measure("Regions.Group image rendering");
            image.render(
                image_key,
                &tile,
                table,
                simulation.directions(),
                simulation.magnitudes(),
                star_opacities,
            );
            *tile_bytes += image.bytes();
            report.rendered += 1;
        }

        // Find this region's group images and its previously merged image.
        let groups = &region.speed_groups;
        let region_image = &mut region.merged_tile;

        // Keep the merged image when all of its group images are unchanged.
        times.measure("Regions.Image cache decisions");
        let region_is_current = region_image.is_current(groups);
        if region_is_current {
            continue;
        }

        // Merge the updated group images and account for their combined image.
        *tile_bytes -= region_image.bytes();
        times.measure("Regions.Region image merging");
        region_image.merge(groups);
        *tile_bytes += region_image.bytes();
        report.merged += 1;
    }

    times.measure("Regions");
    report
}

#[cfg(test)]
mod tests {
    use crate::constants::{CELL_COUNT, SPEED_GROUP_COUNT};
    use crate::model::{RegionTileCache, Texel};
    use super::*;
    use crate::formulas::{J2000, Observer};
    use crate::model::{ProjectionKind, View, ViewCenter};
    use crate::observation::observe;
    use crate::selection::select_regions;

    fn bsc5() -> Data {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap(), &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    fn view(fov_degrees: f64, azimuth: f64) -> View { View { center: ViewCenter::Facing { azimuth, tilt: 0.5 }, projection: ProjectionKind::Stereographic, fov_degrees } }
    fn whole_sky_view(azimuth: f64) -> View { View { center: ViewCenter::Facing { azimuth, tilt: 0.5 }, projection: ProjectionKind::Equidistant, fov_degrees: 360.0 } }
    fn settings(threshold: f64) -> RenderOptions { RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: threshold, dynamic_names: true } }
    fn time(days: f64) -> i64 { crate::formulas::utc_jd_to_time_ms(J2000 + days).unwrap() }

    /// One refresh: observe, select and refresh for the given inputs.
    fn frame(caches: &mut Data, tables: &Data, days: f64, view: &View, threshold: f64) -> (Selected, RefreshReport) {
        let observation = observe(&boston(), true, view, time(days), crate::formulas::evaluate_earth_state(time(days)), &mut crate::time::StepTimes::default());
        let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
        let report = refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &selected.regions, time(days), view.fov_degrees, settings(threshold).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        );
        (selected, report)
    }

    /// The groups of the selected regions whose prefix at `threshold` is not empty, by speed group.
    fn drawn_groups(tables: &Data, selected: &Selected, days: f64, threshold: f64) -> [usize; SPEED_GROUP_COUNT] {
        let mut counts = [0; SPEED_GROUP_COUNT];
        for &region in &selected.regions {
            for (group, table) in tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).enumerate() {
                if rows_wanted(table, crate::formulas::AstronomicalTime::from_time_ms(time(days)).tt_jd, threshold) > 0 { counts[group] += 1; }
            }
        }
        counts
    }
    fn regions_with(tables: &Data, selected: &Selected, days: f64, threshold: f64, group: usize) -> usize {
        selected.regions.iter().filter(|&&region| rows_wanted(tables.star_table(region, group), crate::formulas::AstronomicalTime::from_time_ms(time(days)).tt_jd, threshold) > 0).count()
    }

    #[test]
    fn refresh_uses_the_ttl_stored_on_each_group() {
        let region = 0;
        let direction = TileGeometry::new(region).center();
        let star = crate::model::fixtures::row(direction, 0.0, 2.0, 1);
        let mut data = crate::model::fixtures::layout(&[(region, 0, star)]);
        let selected = Selected { regions: vec![region], whole_sky: false };
        let view = view(180.0, 0.0);
        let settings = settings(6.0);
        let mut times = StepTimes::default();

        // Give this slow group a shorter TTL than the configured default.
        data.star_regions[region].speed_groups[0].ttl_ms = 60_000;
        let start = crate::formulas::utc_jd_to_time_ms(J2000).unwrap();
        let first = refresh_regions(
            &mut data.star_regions, &data.constellation_stars,
            &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
            &mut data.star_opacities, &mut data.tile_bytes,
            &selected.regions, start, view.fov_degrees, settings.magnitude_threshold,
            &mut times,
        );
        assert_eq!(first, RefreshReport { simulated: 1, rendered: 1, merged: 1 });

        // Paused time and times inside the stored TTL reuse all three caches.
        for milliseconds in [0, 30_000] {
            let later = start + milliseconds;
            assert_eq!(refresh_regions(
                &mut data.star_regions, &data.constellation_stars,
                &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
                &mut data.star_opacities, &mut data.tile_bytes,
                &selected.regions, later, view.fov_degrees, settings.magnitude_threshold,
                &mut times,
            ), RefreshReport::default());
        }

        // Crossing that TTL recalculates the group and invalidates its merged region image.
        let expired = start + 61_000;
        let refreshed = refresh_regions(
            &mut data.star_regions, &data.constellation_stars,
            &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
            &mut data.star_opacities, &mut data.tile_bytes,
            &selected.regions, expired, view.fov_degrees, settings.magnitude_threshold,
            &mut times,
        );
        assert_eq!(refreshed, first);
        assert_eq!(data.star_regions[region].speed_groups[0].last_update_ms, Some(expired));
    }

    #[test]
    fn zoom_redraws_tiles_without_resimulation_and_hidden_regions_refresh_on_return() {
        let mut placed = Vec::new();
        for region in [0, 1] {
            placed.push((region, 0, crate::model::fixtures::row(TileGeometry::new(region).center(), 0.0, 6.0, region as u32)));
        }
        let tables = crate::model::fixtures::layout(&placed);
        let mut caches = tables.clone();
        let both = Selected { regions: vec![0, 1], whole_sky: false };
        let wide = view(180.0, 1.0);
        let narrow = view(60.0, 1.0);
        let brightness_changes = zoom_opacity_boost(wide.fov_degrees) != zoom_opacity_boost(narrow.fov_degrees);
        refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &both.regions, time(0.0), wide.fov_degrees, settings(9.0).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        );
        let original = [caches.star_regions[0].merged_tile.clone(), caches.star_regions[1].merged_tile.clone()];
        for region in [0, 1] {
            let selected = Selected { regions: vec![region], whole_sky: false };
            let changed = refresh_regions(
                &mut caches.star_regions, &caches.constellation_stars,
                &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
                &mut caches.star_opacities, &mut caches.tile_bytes,
                &selected.regions, time(0.0), narrow.fov_degrees, settings(9.0).magnitude_threshold,
                &mut crate::time::StepTimes::default(),
            );
            let redraw_count = usize::from(brightness_changes);
            assert_eq!(changed, RefreshReport { simulated: 0, rendered: redraw_count, merged: redraw_count });
            assert_eq!(caches.star_regions[region].speed_groups[0].tile.key().unwrap().zoom_boost, zoom_opacity_boost(60.0));
            assert_eq!(caches.star_regions[region].merged_tile != original[region], brightness_changes);
            if region == 0 { assert_eq!((&caches.star_regions[1].merged_tile), &original[1]); }            // hidden tile retains its old key until selected again
            assert_eq!(refresh_regions(
                &mut caches.star_regions, &caches.constellation_stars,
                &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
                &mut caches.star_opacities, &mut caches.tile_bytes,
                &selected.regions, time(0.0), narrow.fov_degrees, settings(9.0).magnitude_threshold,
                &mut crate::time::StepTimes::default(),
            ), RefreshReport::default());
        }
        let restored = refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &both.regions, time(0.0), wide.fov_degrees, settings(9.0).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        );
        let redraw_count = 2 * usize::from(brightness_changes);
        assert_eq!(restored, RefreshReport { simulated: 0, rendered: redraw_count, merged: redraw_count });
        for (region, image) in original.iter().enumerate() { assert_eq!((&caches.star_regions[region].merged_tile), image); }
        assert_eq!(refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &both.regions, time(0.0), view(360.0, 2.0).fov_degrees, settings(9.0).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        ), RefreshReport::default()); // same boost, no redraw
    }

    #[test]
    fn zoom_is_clamped_per_star_before_overlapping_stars_blend_in_a_tile() {
        let region = 0;
        let tile = TileGeometry::new(region);
        let magnitude = crate::constants::STAR_OPACITY_REFERENCE_MAGNITUDE + 2_f64.log10() / crate::constants::STAR_OPACITY_MAGNITUDE_SCALE; // about half opacity before zoom
        let mut slow = crate::model::fixtures::row(tile.center(), 0.0, magnitude, 1);
        let mut top = crate::model::fixtures::row(tile.center(), 0.0, magnitude, 2);
        slow.display_color = crate::model::StarColor::RedOrange.index();
        top.display_color = crate::model::StarColor::BlueWhite.index();
        let tables = crate::model::fixtures::layout(&[(region, 0, slow), (region, 0, top)]);
        let mut caches = tables.clone();
        let selected = Selected { regions: vec![region], whole_sky: false };
        refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &selected.regions, time(0.0), view(60.0, 1.0).fov_degrees, settings(magnitude + 1.0).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        );
        // Calculate each star's opacity from the configured zoom power before combining their colours.
        let opacity = star_opacity(crate::catalog::decode_magnitude(top.magnitude), zoom_opacity_boost(60.0));
        let mut expected = Texel::default();
        blend_star(&mut expected, premultiplied_color(slow.display_color, opacity), opacity);
        blend_star(&mut expected, premultiplied_color(top.display_color, opacity), opacity);
        let lit: Vec<_> = caches.star_regions[region].merged_tile.texels().iter().filter(|texel| !texel.is_empty()).collect();
        assert_eq!(lit.len(), 4);
        assert!(lit.into_iter().all(|texel| *texel == expected));                           // per-star opacity is applied before any region merging or warp
    }

    #[test]
    fn the_invalidation_matrix_over_the_whole_sky() {
        let tables = bsc5();
        let mut caches = tables.clone();
        let wide = whole_sky_view(1.0);                                                      // the patch covers the whole sky: every non-empty region is selected
        let (selected, first) = frame(&mut caches, &tables, 0.0, &wide, 6.0);
        assert!(selected.whole_sky);
        let drawn = drawn_groups(&tables, &selected, 0.0, 6.0);
        assert_eq!(first.rendered, drawn.iter().sum::<usize>());
        assert_eq!(first.simulated, first.rendered);
        assert_eq!(first.merged, selected.regions.len());                                    // every selected region is merged once, even those with only empty groups
        let snapshot = caches.clone();

        // nothing: the same frame, Earth rotation, panning, and time inside every hold
        for (days, azimuth) in [(0.0, 1.0), (1.0 / 24.0, 1.0), (0.0, 2.5), (0.9 * (get_ttl_of_speed_group_ms(2) as f64) / 86_400_000.0, 1.0)] {
            let (_, report) = frame(&mut caches, &tables, days, &whole_sky_view(azimuth), 6.0);
            assert_eq!(report, RefreshReport::default(), "{days} days, azimuth {azimuth}");
            assert!(caches.star_regions == snapshot.star_regions);
        }

        // the fast hold: only the fast groups and their regions
        let fast_day = (get_ttl_of_speed_group_ms(2) as f64) / 86_400_000.0 + 0.5;
        let (_, fast) = frame(&mut caches, &tables, fast_day, &wide, 6.0);
        assert_eq!((fast.simulated, fast.rendered, fast.merged), (drawn[2], drawn[2], regions_with(&tables, &selected, fast_day, 6.0, 2)));
        assert!(fast.rendered > 0);
        for (region, group, table) in tables.star_tables() {
            if group < 2 && !table.is_empty() { assert_eq!((&caches.star_regions[region].speed_groups[group].tile), (&snapshot.star_regions[region].speed_groups[group].tile)); }
        }

        // the medium hold: medium and fast
        let medium_day = (get_ttl_of_speed_group_ms(1) as f64) / 86_400_000.0 + 0.5;
        let (_, medium) = frame(&mut caches, &tables, medium_day, &wide, 6.0);
        assert_eq!((medium.simulated, medium.rendered), (drawn[1] + drawn[2], drawn[1] + drawn[2]));

        // a deeper threshold: every group with rows, extended where the prefix grew; a shallower one: every group with rows at the shallow threshold
        let (_, deeper) = frame(&mut caches, &tables, medium_day, &wide, 7.0);
        let deep = drawn_groups(&tables, &selected, medium_day, 7.0);
        assert_eq!(deeper.rendered, deep.iter().sum::<usize>());
        let grown = tables.star_tables().filter(|(_, _, table)| rows_wanted(table, crate::formulas::AstronomicalTime::from_time_ms(time(medium_day)).tt_jd, 7.0) > rows_wanted(table, crate::formulas::AstronomicalTime::from_time_ms(time(medium_day)).tt_jd, 6.0)).count();
        assert_eq!(deeper.simulated, grown);                                                 // extended where the prefix grew, nothing re-evaluated
        assert!(grown > 0 && grown < deeper.rendered);
        let (_, shallower) = frame(&mut caches, &tables, medium_day, &wide, 3.0);
        let shallow = drawn_groups(&tables, &selected, medium_day, 3.0);
        assert_eq!((shallower.rendered, shallower.simulated), (shallow.iter().sum::<usize>(), 0));
        assert!(shallower.rendered < deeper.rendered);
        let (_, again) = frame(&mut caches, &tables, medium_day, &wide, 3.0);
        assert_eq!(again, RefreshReport::default());
    }

    #[test]
    fn a_narrow_view_renders_only_regions_that_enter_the_selection_and_keeps_the_rest() {
        let tables = bsc5();
        let mut caches = tables.clone();
        let narrow = view(20.0, 1.0);
        let (before, first) = frame(&mut caches, &tables, 0.0, &narrow, 6.5);
        assert!(!before.whole_sky && first.rendered > 0);
        let kept: Vec<RegionTileCache> = before.regions.iter().map(|&region| caches.star_regions[region].merged_tile.clone()).collect();
        let (after, second) = frame(&mut caches, &tables, 1.0 / 24.0, &narrow, 6.5);        // an hour later: the sky has turned
        let entered: Vec<usize> = after.regions.iter().copied().filter(|region| !before.regions.contains(region)).collect();
        assert!(!entered.is_empty());
        let expected: usize = entered.iter().map(|&region| tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).filter(|table| rows_wanted(table, crate::formulas::AstronomicalTime::from_time_ms(time(0.0)).tt_jd, 6.5) > 0).count()).sum();
        assert_eq!(second.rendered, expected);
        assert_eq!(second.merged, entered.len());
        for (&region, image) in before.regions.iter().zip(&kept) { assert_eq!((&caches.star_regions[region].merged_tile), image); } // regions that left the view keep their images
        let (_, third) = frame(&mut caches, &tables, 1.0 / 24.0, &narrow, 6.5);
        assert_eq!(third, RefreshReport::default());
        let (_, back) = frame(&mut caches, &tables, 0.0, &narrow, 6.5);
        assert_eq!(back, RefreshReport::default());                                          // looking back needs no work at all
    }

    #[test]
    fn the_byte_count_matches_the_images_held_and_every_image_is_one_tile() {
        let tables = bsc5();
        let mut caches = tables.clone();
        for (days, fov, threshold) in [(0.0, 20.0, 6.0), (0.0, 60.0, 6.0), (20.0, 60.0, 3.0), (0.0, 180.0, 6.0)] {
            frame(&mut caches, &tables, days, &view(fov, 1.0), threshold);
            let held: usize = (0..CELL_COUNT).map(|region| caches.star_regions[region].merged_tile.bytes() + (0..SPEED_GROUP_COUNT).map(|group| caches.star_regions[region].speed_groups[group].tile.bytes()).sum::<usize>()).sum();
            assert_eq!(caches.tile_bytes, held);
            for region in 0..CELL_COUNT {
                for texels in [caches.star_regions[region].merged_tile.texels().len()].into_iter().chain((0..SPEED_GROUP_COUNT).map(|group| caches.star_regions[region].speed_groups[group].tile.texels().len())) {
                    assert!(texels == 0 || texels == TileGeometry::texel_count());
                }
            }
        }
    }

    #[test]
    fn a_single_star_tile_holds_the_four_texel_glyph_of_the_old_pixel_renderer() {
        let tables = bsc5();
        let (region, group, table) = tables.star_tables().find(|(_, _, table)| table.len() == 1).unwrap();
        let mut caches = tables.clone();
        let selected = Selected { regions: vec![region], whole_sky: false };
        refresh_regions(
            &mut caches.star_regions, &caches.constellation_stars,
            &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
            &mut caches.star_opacities, &mut caches.tile_bytes,
            &selected.regions, time(0.0), View::default().fov_degrees, settings(9.0).magnitude_threshold,
            &mut crate::time::StepTimes::default(),
        );
        let image = &caches.star_regions[region].speed_groups[group].tile;
        let tile = TileGeometry::new(region);
        let simulation = &caches.star_regions[region].speed_groups[group].simulation;
        let (column, row) = tile.project(simulation.directions()[0]).unwrap();
        let (row, column) = (row.round() as usize, column.round() as usize);                 // the glyph's bottom right texel
        let opacity = caches.star_opacities.of(simulation.magnitudes()[0]);                     // per-star opacity, including the view's boost
        let expected = Texel { rgb: premultiplied_color(table.columns().display_color[0], opacity), opacity };
        let size = TileGeometry::size();
        let lit: Vec<usize> = image.texels().iter().enumerate().filter(|(_, texel)| texel.opacity > 0.0).map(|(index, _)| index).collect();
        assert_eq!(lit, [(row - 1) * size + column - 1, (row - 1) * size + column, row * size + column - 1, row * size + column]);
        for index in lit { assert_eq!(image.texels()[index], expected); }
        assert_eq!(caches.star_regions[region].merged_tile.texels(), image.texels());                 // one group: the merge is the image
    }

    /// Release-mode cost of the images; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_refresh_of_bsc5_at_three_fields() {
        let tables = bsc5();
        for fov in [10.0, 60.0, 180.0] {
            let mut caches = tables.clone();
            let view = view(fov, 1.0);
            let timed = |caches: &mut Data, days: f64, threshold: f64, what: &str| {
                let observation = observe(&boston(), true, &view, time(days), crate::formulas::evaluate_earth_state(time(days)), &mut crate::time::StepTimes::default());
                let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
                let started = std::time::Instant::now();
                let report = refresh_regions(
                    &mut caches.star_regions, &caches.constellation_stars,
                    &mut caches.constellation_simulation, &mut caches.constellation_last_update_ms,
                    &mut caches.star_opacities, &mut caches.tile_bytes,
                    &selected.regions, time(days), view.fov_degrees, settings(threshold).magnitude_threshold,
                    &mut crate::time::StepTimes::default(),
                );
                let elapsed = started.elapsed();
                eprintln!("{fov:>5}° {what:<14} {elapsed:>10.3?}  regions={} rendered={} merged={} simulated={} tile bytes={}", selected.regions.len(), report.rendered, report.merged, report.simulated, caches.tile_bytes);
            };
            timed(&mut caches, 0.0, 6.0, "first frame");
            timed(&mut caches, 1.0 / 24.0, 6.0, "an hour later");
            timed(&mut caches, 11.0, 6.0, "fast hold");
            timed(&mut caches, 11.0, 7.0, "deeper");
            timed(&mut caches, 11.0, 7.0, "steady");
        }
    }
}
