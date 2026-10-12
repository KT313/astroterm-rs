//! Step 4 of the frame: `refresh_regions`.
//!
//! Inputs: the caches, the star tables, the selected regions, the simulated time and the render settings. Output:
//! up-to-date group images and region images for every selected region, and the constellation table's
//! simulation. Real work happens per group only when a cache key changed (plan section 4): a group is simulated
//! again when its hold time has expired, its image is drawn again when its key changed, and a region image is
//! summed again when one of its group images was. Rotation, panning, zoom, resize and simulated time inside the
//! holds do nothing. Images are never dropped: a region that leaves the view keeps its images for its return
//! (owner's decision of 2026-10-12; the memory is bounded by the fixed tile size times the region count).
//!
//! The module holds the speed-group rule (`groups`), the simulation caches (`simulation`), the star appearance
//! (`appearance`), the tile geometry (`tiles`) and the image caches (`images`).
mod appearance;
mod groups;
mod images;
mod simulation;
mod tiles;
pub use appearance::{STAR_GLYPH_RADIUS, StarOpacities, Texel, blend_star, premultiplied_color, star_opacity, zoom_opacity_boost};
pub use groups::{speed_group, speed_group_hold_seconds, drift_bound_for_rate};
pub use images::{GroupImage, ImageKey, RegionImage};
pub use simulation::{GroupSimulation, Simulations, Refresh, UNPLACED, rows_wanted};
pub use tiles::{TileFrame, TileGeometry, tile_frames};

use crate::astro::FrameTime;
use crate::catalog::magnitude_code;
use crate::constants::{CELL_COUNT, SPEED_GROUP_COUNT};
use crate::model::{RenderOptions, SkyTables};
use crate::selection::Selected;

/// Everything step 4 keeps between frames: the simulation cache and the image cache of every group, the image of
/// every region, and the opacity table.
#[derive(Clone, Debug)]
pub struct RegionCaches {
    simulations: Simulations,
    groups: Vec<GroupImage>,                // CELL_COUNT × SPEED_GROUP_COUNT, region-major, like `Stars`
    regions: Vec<RegionImage>,              // CELL_COUNT
    opacities: StarOpacities,               // the curve without zoom boost; the boost is applied in the frame output
    bytes: usize,                           // texel bytes held by all images
}

/// What one refresh did, for timing and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub simulated: usize,                   // groups whose simulation was refreshed or extended
    pub rendered: usize,                    // group images drawn again (empty prefixes do not count)
    pub summed: usize,                      // region images summed again
}

impl Default for RegionCaches {
    fn default() -> Self {
        let mut opacities = StarOpacities::default();
        opacities.prepare(1.0);
        Self {
            simulations: Simulations::default(),
            groups: vec![GroupImage::default(); CELL_COUNT * SPEED_GROUP_COUNT],
            regions: vec![RegionImage::default(); CELL_COUNT],
            opacities,
            bytes: 0,
        }
    }
}

impl RegionCaches {
    pub fn simulations(&self) -> &Simulations { &self.simulations }
    pub fn group_image(&self, region: usize, group: usize) -> &GroupImage { &self.groups[region * SPEED_GROUP_COUNT + group] }
    pub fn region_image(&self, region: usize) -> &RegionImage { &self.regions[region] }
    pub fn tile_bytes(&self) -> usize { self.bytes }
    /// The opacity curve the tiles were drawn with (no zoom boost).
    pub fn opacities(&self) -> &StarOpacities { &self.opacities }
}

/// Bring the images of the selected regions and the constellation simulation up to date for this frame.
pub fn refresh_regions(caches: &mut RegionCaches, tables: &SkyTables, selected: &Selected, time: FrameTime, settings: &RenderOptions) -> RefreshReport {
    let mut report = RefreshReport::default();
    let tt = time.tt;
    let threshold = settings.magnitude_threshold;
    let threshold_code = magnitude_code(threshold);
    caches.simulations.update_constellations(tables.constellations.table(), tt);

    // per selected region: simulate and draw each group whose key changed, then re-sum the region if any did
    for &region in &selected.regions {
        let tile = TileGeometry::new(region);
        for (group, table) in tables.stars.region(region).iter().enumerate() {
            let slot = region * SPEED_GROUP_COUNT + group;
            let rows = rows_wanted(table, tt, threshold);
            if rows == 0 {
                caches.bytes -= caches.groups[slot].bytes();
                caches.groups[slot].render(ImageKey::EMPTY, &tile, table, &[], &[], &caches.opacities);
                continue;
            }
            if caches.simulations.update_group(table, region, group, tt, threshold) != Refresh::Unchanged { report.simulated += 1; }
            let simulation = caches.simulations.group(region, group);
            let key = ImageKey { epoch: simulation.epoch().expect("a simulated group has an epoch"), rows, threshold: threshold_code };
            let image = &mut caches.groups[slot];
            if image.key() == Some(key) { continue; }
            caches.bytes -= image.bytes();
            image.render(key, &tile, table, simulation.directions(), simulation.magnitudes(), &caches.opacities);
            caches.bytes += image.bytes();
            report.rendered += 1;
        }
        let groups = &caches.groups[region * SPEED_GROUP_COUNT..(region + 1) * SPEED_GROUP_COUNT];
        let image = &mut caches.regions[region];
        if image.is_current(groups) { continue; }
        caches.bytes -= image.bytes();
        image.sum(groups);
        caches.bytes += image.bytes();
        report.summed += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{J2000, JULIAN_YEAR_DAYS, Observer};
    use crate::constants::FAST_GROUP_HOLD_SECONDS;
    use crate::model::{ProjectionKind, View, ViewCenter};
    use crate::observation::observe;
    use crate::selection::select_regions;

    fn bsc5() -> SkyTables {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap(), &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    fn view(fov_degrees: f64, azimuth: f64) -> View { View { center: ViewCenter::Facing { azimuth, tilt: 0.5 }, projection: ProjectionKind::Stereographic, fov_degrees } }
    fn whole_sky_view(azimuth: f64) -> View { View { center: ViewCenter::Facing { azimuth, tilt: 0.5 }, projection: ProjectionKind::Equidistant, fov_degrees: 360.0 } }
    fn settings(threshold: f64) -> RenderOptions { RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: threshold, dynamic_names: true } }
    fn time(days: f64) -> FrameTime { FrameTime::from_utc(J2000 + days) }

    /// One refresh: observe, select and refresh for the given inputs.
    fn frame(caches: &mut RegionCaches, tables: &SkyTables, days: f64, view: &View, threshold: f64) -> (Selected, RefreshReport) {
        let observation = observe(&boston(), true, view, time(days));
        let selected = select_regions(&tables.stars, &observation);
        let report = refresh_regions(caches, tables, &selected, time(days), &settings(threshold));
        (selected, report)
    }

    /// The groups of the selected regions whose prefix at `threshold` is not empty, by speed group.
    fn drawn_groups(tables: &SkyTables, selected: &Selected, days: f64, threshold: f64) -> [usize; SPEED_GROUP_COUNT] {
        let mut counts = [0; SPEED_GROUP_COUNT];
        for &region in &selected.regions {
            for (group, table) in tables.stars.region(region).iter().enumerate() {
                if rows_wanted(table, time(days).tt, threshold) > 0 { counts[group] += 1; }
            }
        }
        counts
    }
    fn regions_with(tables: &SkyTables, selected: &Selected, days: f64, threshold: f64, group: usize) -> usize {
        selected.regions.iter().filter(|&&region| rows_wanted(tables.stars.group(region, group), time(days).tt, threshold) > 0).count()
    }

    #[test]
    fn the_invalidation_matrix_over_the_whole_sky() {
        let tables = bsc5();
        let mut caches = RegionCaches::default();
        let wide = whole_sky_view(1.0);                                                      // the patch covers the whole sky: every non-empty region is selected
        let (selected, first) = frame(&mut caches, &tables, 0.0, &wide, 6.0);
        assert!(selected.whole_sky);
        let drawn = drawn_groups(&tables, &selected, 0.0, 6.0);
        assert_eq!(first.rendered, drawn.iter().sum::<usize>());
        assert_eq!(first.simulated, first.rendered);
        assert_eq!(first.summed, selected.regions.len());                                    // every selected region is summed once, even those with only empty groups
        let snapshot = caches.clone();

        // nothing: the same frame, Earth rotation, panning, and time inside every hold
        for (days, azimuth) in [(0.0, 1.0), (1.0 / 24.0, 1.0), (0.0, 2.5), (9.0, 1.0)] {
            let (_, report) = frame(&mut caches, &tables, days, &whole_sky_view(azimuth), 6.0);
            assert_eq!(report, RefreshReport::default(), "{days} days, azimuth {azimuth}");
            assert!(caches.groups == snapshot.groups && caches.regions == snapshot.regions);
        }

        // the fast hold: only the fast groups and their regions
        let (_, fast) = frame(&mut caches, &tables, FAST_GROUP_HOLD_SECONDS / 86_400.0 + 1.0, &wide, 6.0);
        assert_eq!((fast.simulated, fast.rendered, fast.summed), (drawn[2], drawn[2], regions_with(&tables, &selected, 11.0, 6.0, 2)));
        assert!(fast.rendered > 0);
        for (region, group, table) in tables.stars.iter() {
            if group < 2 && !table.is_empty() { assert_eq!(caches.group_image(region, group), snapshot.group_image(region, group)); }
        }

        // the medium hold: medium and fast
        let year = JULIAN_YEAR_DAYS;
        let (_, medium) = frame(&mut caches, &tables, year, &wide, 6.0);
        assert_eq!((medium.simulated, medium.rendered), (drawn[1] + drawn[2], drawn[1] + drawn[2]));

        // a deeper threshold: every group with rows, extended where the prefix grew; a shallower one: every group with rows at the shallow threshold
        let (_, deeper) = frame(&mut caches, &tables, year, &wide, 7.0);
        let deep = drawn_groups(&tables, &selected, year, 7.0);
        assert_eq!(deeper.rendered, deep.iter().sum::<usize>());
        let grown = tables.stars.iter().filter(|(_, _, table)| rows_wanted(table, time(year).tt, 7.0) > rows_wanted(table, time(year).tt, 6.0)).count();
        assert_eq!(deeper.simulated, grown);                                                 // extended where the prefix grew, nothing re-evaluated
        assert!(grown > 0 && grown < deeper.rendered);
        let (_, shallower) = frame(&mut caches, &tables, year, &wide, 3.0);
        let shallow = drawn_groups(&tables, &selected, year, 3.0);
        assert_eq!((shallower.rendered, shallower.simulated), (shallow.iter().sum::<usize>(), 0));
        assert!(shallower.rendered < deeper.rendered);
        let (_, again) = frame(&mut caches, &tables, year, &wide, 3.0);
        assert_eq!(again, RefreshReport::default());
    }

    #[test]
    fn a_narrow_view_renders_only_regions_that_enter_the_selection_and_keeps_the_rest() {
        let tables = bsc5();
        let mut caches = RegionCaches::default();
        let narrow = view(20.0, 1.0);
        let (before, first) = frame(&mut caches, &tables, 0.0, &narrow, 6.5);
        assert!(!before.whole_sky && first.rendered > 0);
        let kept: Vec<RegionImage> = before.regions.iter().map(|&region| caches.region_image(region).clone()).collect();
        let (after, second) = frame(&mut caches, &tables, 1.0 / 24.0, &narrow, 6.5);        // an hour later: the sky has turned
        let entered: Vec<usize> = after.regions.iter().copied().filter(|region| !before.regions.contains(region)).collect();
        assert!(!entered.is_empty());
        let expected: usize = entered.iter().map(|&region| tables.stars.region(region).iter().filter(|table| rows_wanted(table, time(0.0).tt, 6.5) > 0).count()).sum();
        assert_eq!(second.rendered, expected);
        assert_eq!(second.summed, entered.len());
        for (&region, image) in before.regions.iter().zip(&kept) { assert_eq!(caches.region_image(region), image); } // regions that left the view keep their images
        let (_, third) = frame(&mut caches, &tables, 1.0 / 24.0, &narrow, 6.5);
        assert_eq!(third, RefreshReport::default());
        let (_, back) = frame(&mut caches, &tables, 0.0, &narrow, 6.5);
        assert_eq!(back, RefreshReport::default());                                          // looking back needs no work at all
    }

    #[test]
    fn the_byte_count_matches_the_images_held_and_every_image_is_one_tile() {
        let tables = bsc5();
        let mut caches = RegionCaches::default();
        for (days, fov, threshold) in [(0.0, 20.0, 6.0), (0.0, 60.0, 6.0), (20.0, 60.0, 3.0), (0.0, 180.0, 6.0)] {
            frame(&mut caches, &tables, days, &view(fov, 1.0), threshold);
            let held: usize = (0..CELL_COUNT).map(|region| caches.region_image(region).bytes() + (0..SPEED_GROUP_COUNT).map(|group| caches.group_image(region, group).bytes()).sum::<usize>()).sum();
            assert_eq!(caches.tile_bytes(), held);
            for region in 0..CELL_COUNT {
                for texels in [caches.region_image(region).texels().len()].into_iter().chain((0..SPEED_GROUP_COUNT).map(|group| caches.group_image(region, group).texels().len())) {
                    assert!(texels == 0 || texels == TileGeometry::texel_count());
                }
            }
        }
    }

    #[test]
    fn a_single_star_tile_holds_the_four_texel_glyph_of_the_old_pixel_renderer() {
        let tables = bsc5();
        let (region, group, table) = tables.stars.iter().find(|(_, _, table)| table.len() == 1).unwrap();
        let mut caches = RegionCaches::default();
        let selected = Selected { regions: vec![region], whole_sky: false };
        refresh_regions(&mut caches, &tables, &selected, time(0.0), &settings(9.0));
        let image = caches.group_image(region, group);
        let tile = TileGeometry::new(region);
        let simulation = caches.simulations().group(region, group);
        let (column, row) = tile.project(simulation.directions()[0]).unwrap();
        let (row, column) = (row.round() as usize, column.round() as usize);                 // the glyph's bottom right texel
        let opacity = caches.opacities().of(simulation.magnitudes()[0]);                     // no zoom boost in the tiles
        let expected = Texel { rgb: premultiplied_color(table.columns().display_color[0], opacity), opacity };
        let size = TileGeometry::size();
        let lit: Vec<usize> = image.texels().iter().enumerate().filter(|(_, texel)| texel.opacity > 0.0).map(|(index, _)| index).collect();
        assert_eq!(lit, [(row - 1) * size + column - 1, (row - 1) * size + column, row * size + column - 1, row * size + column]);
        for index in lit { assert_eq!(image.texels()[index], expected); }
        assert_eq!(caches.region_image(region).texels(), image.texels());                 // one group: the sum is the image
    }

    /// Release-mode cost of the images; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_refresh_of_bsc5_at_three_fields() {
        let tables = bsc5();
        for fov in [10.0, 60.0, 180.0] {
            let mut caches = RegionCaches::default();
            let view = view(fov, 1.0);
            let timed = |caches: &mut RegionCaches, days: f64, threshold: f64, what: &str| {
                let observation = observe(&boston(), true, &view, time(days));
                let selected = select_regions(&tables.stars, &observation);
                let started = std::time::Instant::now();
                let report = refresh_regions(caches, &tables, &selected, time(days), &settings(threshold));
                let elapsed = started.elapsed();
                eprintln!("{fov:>5}° {what:<14} {elapsed:>10.3?}  regions={} rendered={} summed={} simulated={} tile bytes={}", selected.regions.len(), report.rendered, report.summed, report.simulated, caches.tile_bytes());
            };
            timed(&mut caches, 0.0, 6.0, "first frame");
            timed(&mut caches, 1.0 / 24.0, 6.0, "an hour later");
            timed(&mut caches, 11.0, 6.0, "fast hold");
            timed(&mut caches, 11.0, 7.0, "deeper");
            timed(&mut caches, 11.0, 7.0, "steady");
        }
    }
}
