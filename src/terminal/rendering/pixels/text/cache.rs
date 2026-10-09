//! Validate retained labels/text before reading star candidates or resetting the text grid.
use crate::model::{PixelLabel, PixelLabelKey, PixelTextCache, PixelTextKey, ProductionRasterKey, RasterRegion,
    RenderProjection, RenderResultVersion, ProjectedSky, RenderOptions, MetadataField};
use crate::scene::{format_star_label, select_pixel_star_labels, star_rgb};
use crate::timing::{BufferId, BufferShape, IndexDomain, StepTimes};
use ratatui::{buffer::Buffer, layout::Rect};

#[allow(clippy::too_many_arguments)]
pub(in crate::terminal::rendering::pixels) fn refresh_text(cache: &mut PixelTextCache, buffer: &mut Buffer, version: &mut RenderResultVersion, sky: &ProjectedSky<'_>, prepared: Option<&RenderProjection<'_>>, options: &RenderOptions, screen: Rect, area: Rect, fields: &[MetadataField], notice: Option<&str>, reuse: bool, times: &mut StepTimes) {
    refresh_labels(cache, sky, prepared, options, area, reuse, times);                       // refresh only the bounded label result before checking the whole grid
    let fields = &fields[..fields.len().min(usize::from(screen.height))];                   // hidden metadata does not affect displayed text
    let hit = reuse && version.current().is_some() && cache.text_key.as_ref().is_some_and(|key| matches_text(key, cache.labels_version.current().unwrap(), sky, options, screen, area, fields, notice));
    times.describe("Text layout", || format!("completed text reused={hit}; label descriptions={}", cache.labels.len()));
    if hit {
        times.record_shape(BufferId::TextCells, crate::timing::Operation::Reuse, None, || BufferShape::vector(&buffer.content, IndexDomain::Cells));
        return;
    }
    version.invalidate();                                                                  // a partially rewritten grid cannot be reused after unwinding
    super::compose_text_into(buffer, &cache.labels, sky, options, screen, area, fields, notice, times);
    cache.text_key = Some(PixelTextKey { labels: cache.labels_version.current().unwrap(), screen, area,
        viewport: sky.viewport, facing: sky.facing, grid: options.grid,
        planets: sky.planets.iter().map(|p| (p.kind, p.cell)).collect(), moon: sky.moon.cell,
        horizon: sky.horizon_labels.to_vec(), fields: fields.to_vec(), notice: notice.map(str::to_owned),
        accuracy_warning: sky.outside_accuracy_range, brightness_warning: sky.magnitude_clipping().any() });
    version.publish(true);                                                                  // explicitly conservative revision: refreshed inputs may produce equal text
}

fn refresh_labels(cache: &mut PixelTextCache, sky: &ProjectedSky<'_>, prepared: Option<&RenderProjection<'_>>, options: &RenderOptions, area: Rect, reuse: bool, times: &mut StepTimes) {
    let hit = reuse && cache.labels_version.current().is_some() && prepared.is_some_and(|source| cache.labels_key.as_ref().is_some_and(|key| matches_labels(key, source, options, area)));
    if hit {
        times.record_shape(BufferId::PixelLabels, crate::timing::Operation::Reuse, None, || BufferShape::vector(&cache.labels, IndexDomain::Objects));
        times.describe("Text layout", || format!("star labels reused={}; examined candidates=0; examined regions=0", cache.labels.len()));
        return;
    }
    cache.labels_version.invalidate();
    let mut regions = cache.labels_key.take().map_or_else(Vec::new, |key| key.projection.regions); // editable callers always refresh without retaining false provenance
    regions.clear();                                                                       // ...but the region list keeps its capacity for the new key
    times.measure_steps("Star labels", |times| {
        let candidates = select_pixel_star_labels(options, sky);
        let (examined, regions, eligible) = (candidates.examined, candidates.regions, candidates.eligible);
        cache.labels.clear();
        for index in candidates {
            let entry = sky.stars.get(index);
            let (row, col) = super::map_pixel_to_cell(sky, area, entry.cell.expect("selected star has a drawable cell"));
            cache.labels.push(PixelLabel { text: format_star_label(&entry.star, sky.names, true).into_owned(),
                row: row - 1, col: col + 1, rgb: star_rgb(&entry.star) });
        }
        times.record_build(BufferId::PixelLabels, || BufferShape::vector(&cache.labels, IndexDomain::Objects));
        times.describe("Star labels", || format!("examined regions={regions}; examined candidates={examined}; eligible candidates={eligible}; selected labels={}; newly formatted={}; clipped origins get no replacement", cache.labels.len(), cache.labels.len()));
    });
    cache.labels_key = prepared.map(|source| {
        regions.extend(source.regions.iter().zip(source.assembled).map(|(region, assembled)| RasterRegion { observed: *region, cells: assembled.3, order: assembled.4 }));
        PixelLabelKey { projection: ProductionRasterKey { source: source.source, geometry: source.geometry, regions },
            viewport: sky.viewport, area, threshold: options.magnitude_threshold, enabled: options.dynamic_names }
    });
    cache.labels_version.publish(true);
}

fn matches_labels(key: &PixelLabelKey, source: &RenderProjection<'_>, options: &RenderOptions, area: Rect) -> bool {
    key.area == area && key.viewport == source.sky().viewport && key.threshold == options.magnitude_threshold
        && key.enabled == options.dynamic_names && key.projection.source == source.source
        && key.projection.regions.len() == source.regions.len()
        && key.projection.regions.iter().zip(source.regions.iter().zip(source.assembled)).all(|(saved, (region, assembled))| {
            saved.observed == *region && saved.cells == assembled.3 && saved.order == assembled.4
        })
}

#[allow(clippy::too_many_arguments)]
fn matches_text(key: &PixelTextKey, labels: u64, sky: &ProjectedSky<'_>, options: &RenderOptions, screen: Rect, area: Rect, fields: &[MetadataField], notice: Option<&str>) -> bool {
    key.labels == labels && key.screen == screen && key.area == area && key.viewport == sky.viewport
        && key.facing == sky.facing && key.grid == options.grid && key.moon == sky.moon.cell
        && key.planets.iter().copied().eq(sky.planets.iter().map(|p| (p.kind, p.cell)))
        && key.horizon == sky.horizon_labels && key.fields == fields && key.notice.as_deref() == notice
        && key.accuracy_warning == sky.outside_accuracy_range && key.brightness_warning == sky.magnitude_clipping().any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObservedRegion, View, ProjectionViewport};

    fn fixture() -> crate::model::ObservedSky {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars.truncate(12); parsed.constellations.clear();
        for (index, star) in parsed.stars.iter_mut().enumerate() { star.magnitude = index as f64 * 0.25; }
        let mut sky = crate::sky::create_sky_from_catalog(&parsed).unwrap();
        for star in &mut sky.stars { star.position = crate::astro::Vector3 { x: 0.0, y: 0.0, z: 1.0 }; }
        sky
    }
    fn options() -> RenderOptions {
        RenderOptions { dynamic_names: true, magnitude_threshold: 20.0, color: true, unicode: true,
            constellations: true, braille: false, grid: false }
    }
    fn traced_refresh(cache: &mut PixelTextCache, buffer: &mut Buffer, version: &mut RenderResultVersion, prepared: &RenderProjection<'_>, fields: &[MetadataField], reuse: bool) -> StepTimes {
        let mut times = StepTimes::with_trace(true);
        times.measure_steps("Text layout", |times| refresh_text(cache, buffer, version, prepared.sky(), Some(prepared), &options(), Rect::new(0, 0, 40, 20), Rect::new(0, 0, 40, 20), fields, None, reuse, times));
        times
    }
    fn visited(times: &StepTimes, name: &str) -> bool { times.trace().unwrap().steps.iter().any(|s| s.name == name) }

    #[test]
    fn trusted_paused_text_reuses_grid_and_labels_but_visible_metadata_refreshes() {
        let sky = fixture();
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 160, height: 80 });
        let regions = [ObservedRegion { region: 0, start: 0, end: 12, selection_generation: 1, motion_generation: 1, apparent_generation: 1 }];
        let assembled = [(0, 0, 12, 1, 1)];
        let prepared = RenderProjection { sky: data.view(&sky), source: (1, 1), regions: &regions, assembled: &assembled, geometry: [1; 3] };
        let mut cache = PixelTextCache::default(); let mut version = RenderResultVersion::default();
        let mut buffer = Buffer::empty(Rect::default());
        let fields = [MetadataField { label: "Time".into(), value: "Paused".into() }];
        let first = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &fields, true);
        assert!(visited(&first, "Star labels") && visited(&first, "Text canvas"));
        let old_version = version.current(); let expected = buffer.clone(); let pointer = buffer.content.as_ptr();
        let second = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &fields, true);
        assert!(!visited(&second, "Star labels") && !visited(&second, "Text canvas"));
        assert_eq!(version.current(), old_version); assert_eq!(buffer, expected); assert_eq!(buffer.content.as_ptr(), pointer);
        let changed = [MetadataField { label: "Time".into(), value: "X".into() }];
        let third = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &changed, true);
        assert!(!visited(&third, "Star labels") && visited(&third, "Text canvas"));
        assert_ne!(version.current(), old_version);
        assert_eq!(buffer[(7, 0)].symbol(), " "); // the shorter field must erase the old suffix
        let mut reference_cache = PixelTextCache::default(); let mut reference_version = RenderResultVersion::default();
        let mut reference = Buffer::empty(Rect::default());
        traced_refresh(&mut reference_cache, &mut reference, &mut reference_version, &prepared, &changed, false);
        assert_eq!(buffer, reference);
        let bypass = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &changed, false);
        assert!(visited(&bypass, "Star labels") && visited(&bypass, "Text canvas"));
    }

    #[test]
    fn provenance_layout_settings_and_editable_inputs_never_reuse_stale_labels() {
        let sky = fixture();
        let mut data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 160, height: 80 });
        let mut regions = [ObservedRegion { region: 0, start: 0, end: 12, selection_generation: 1, motion_generation: 1, apparent_generation: 1 }];
        let assembled = [(0, 0, 12, 1, 1)];
        let mut cache = PixelTextCache::default(); let mut version = RenderResultVersion::default(); let mut buffer = Buffer::empty(Rect::default());
        for source in [(1, 1), (2, 1), (2, 2)] {
            let prepared = RenderProjection { sky: data.view(&sky), source, regions: &regions, assembled: &assembled, geometry: [1; 3] };
            let times = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &[], true);
            assert!(visited(&times, "Star labels"));
        }
        regions[0].motion_generation += 1;
        let prepared = RenderProjection { sky: data.view(&sky), source: (2, 2), regions: &regions, assembled: &assembled, geometry: [1; 3] };
        assert!(visited(&traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &[], true), "Star labels"));
        let area = Rect::new(0, 0, 40, 20);
        for (threshold, enabled, layout) in [(0.25, true, area), (20.0, false, area), (20.0, true, Rect::new(0, 0, 30, 10))] {
            let options = RenderOptions { magnitude_threshold: threshold, dynamic_names: enabled, ..options() };
            refresh_text(&mut cache, &mut buffer, &mut version, prepared.sky(), Some(&prepared), &options, area, layout, &[], None, true, &mut StepTimes::default());
            assert_eq!(cache.labels.is_empty(), !enabled);
        }
        data.order.clear(); // mutable/headless source has no trusted revision, so it always refreshes
        refresh_text(&mut cache, &mut buffer, &mut version, &data.view(&sky), None, &options(), area, area, &[], None, true, &mut StepTimes::default());
        assert!(cache.labels.is_empty()); assert!(cache.labels_key.is_none());
    }

    #[test]
    fn hidden_metadata_is_ignored_and_notices_are_removed_without_stale_cells() {
        let sky = fixture();
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 160, height: 80 });
        let prepared = RenderProjection { sky: data.view(&sky), source: (1, 1), regions: &[], assembled: &[], geometry: [1; 3] };
        let mut cache = PixelTextCache::default(); let mut version = RenderResultVersion::default(); let mut buffer = Buffer::empty(Rect::default());
        let mut fields = vec![MetadataField { label: "Counter".into(), value: "1".into() }; 21];
        traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &fields, true);
        let old = version.current(); fields[20].value = "hidden change".into();
        let second = traced_refresh(&mut cache, &mut buffer, &mut version, &prepared, &fields, true);
        assert!(!visited(&second, "Text canvas")); assert_eq!(version.current(), old);
        let area = Rect::new(0, 0, 40, 20);
        for notice in [Some("Protocol notice"), None] {
            refresh_text(&mut cache, &mut buffer, &mut version, prepared.sky(), Some(&prepared), &options(), area, area, &[], notice, true, &mut StepTimes::default());
            let content = buffer.content.iter().map(|c| c.symbol()).collect::<String>();
            assert_eq!(content.contains("Protocol notice"), notice.is_some());
        }
    }
}
