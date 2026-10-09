//! Production raster reuse compares protected region versions before reading individual stars.
use crate::{cache::Group, model::{RenderProjection, RenderOptions, SceneKey, StarKeys, ProductionRasterKey, RasterRegion},
    state::SceneCache, timing::{StepTimes, BufferId, BufferShape, IndexDomain, Operation}};

pub(in crate::scene) fn draw_prepared_pixels<'a>(storage: &'a mut SceneCache, projected: &RenderProjection<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) -> Option<&'a image::RgbaImage> {
    let sky = projected.sky();
    times.measure("Raster dependencies", || capture_dependencies(&mut storage.pixel_candidate, projected, *options)); // small per-region records, no star or geometry copies
    let key = storage.pixel_candidate.as_ref().expect("raster dependencies prepared");
    times.record_build(BufferId::PixelCandidate, || BufferShape::vector(&key.production.as_ref().unwrap().regions, IndexDomain::Objects));
    let refresh = times.measure("Raster cache decision", || storage.pixels.needs_refresh(key, epoch, None, storage.config.allows(Group::Raster)));
    times.record_candidate_decision(times.last_memory_step(), BufferId::PixelCandidate, BufferId::PixelScene, refresh, storage.pixels.stats.last_reason);
    times.describe("Raster dependencies", || format!("requested regions={}; star records visited=0; geometry copied=0; refresh={refresh}", projected.regions.len()));
    if !refresh {
        super::clear_pixel_candidate(storage, times);
        return Some(storage.pixel_image());
    }

    times.measure("Raster drawing inputs", || {
        storage.pixel_inputs.clear();
        storage.pixel_inputs.reserve(sky.stars.len());
        storage.pixel_inputs.extend(super::keys::prepare_pixel_star_inputs(sky, options));
    });
    times.record_build(BufferId::PixelDrawingInputs, || BufferShape::vector(&storage.pixel_inputs, IndexDomain::DrawOrder));
    times.describe("Raster drawing inputs", || format!("input stars={}; accepted inputs={}; prepared once for redraw", sky.stars.len(), storage.pixel_inputs.len()));
    let image = super::super::pipeline::draw_pixel_sky_from_inputs(&mut storage.star_layer, sky, options, times, storage.pixel_inputs.iter().copied())?;
    let outcome = times.measure("Raster cache store", || {
        storage.pixels.store(storage.pixel_candidate.take().expect("raster dependencies prepared"), epoch, 0.0, image)
    });
    super::super::diagnostics::memory::record_scene_commit(times, BufferId::PixelCandidate, BufferId::PixelScene, outcome);
    let before = times.inspect_memory(|| BufferShape::vector(&storage.pixel_inputs, IndexDomain::DrawOrder));
    storage.pixel_inputs.clear(); // retain capacity without retaining duplicate star rows
    times.record_shape(BufferId::PixelDrawingInputs, Operation::Clear, before, || BufferShape::vector(&storage.pixel_inputs, IndexDomain::DrawOrder));
    Some(storage.pixel_image())
}

fn capture_dependencies(candidate: &mut Option<SceneKey>, projected: &RenderProjection<'_>, options: RenderOptions) {
    let sky = projected.sky();
    let key = candidate.get_or_insert_with(|| SceneKey { production: None, viewport: sky.viewport,
        pixel_fov_degrees: Some(sky.fov_degrees), facing: sky.facing, warning: sky.outside_accuracy_range,
        brightness_warning: sky.magnitude_clipping().any(), options, canvas_size: None, stars: StarKeys::Pixels(Vec::new()),
        planets: Vec::new(), moon: None, constellations: Vec::new(), horizon: Vec::new(), labels: Vec::new() });
    if key.production.is_none() { key.stars = StarKeys::Pixels(Vec::new()); } // release obsolete exact star-key storage when switching modes
    super::keys::clear_scene_candidate(key);
    key.viewport = sky.viewport;
    key.pixel_fov_degrees = Some(sky.fov_degrees);
    key.facing = sky.facing;
    key.warning = sky.outside_accuracy_range;
    key.brightness_warning = sky.magnitude_clipping().any();
    key.options = options;
    key.canvas_size = None;
    let source = key.production.get_or_insert_with(|| ProductionRasterKey { source: projected.source, geometry: projected.geometry, regions: Vec::new() });
    source.source = projected.source;
    source.geometry = projected.geometry;
    source.regions.reserve(projected.regions.len());
    source.regions.extend(projected.regions.iter().zip(projected.assembled).map(|(region, assembled)| RasterRegion { observed: *region, cells: assembled.3, order: assembled.4 }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObservedRegion, View, ProjectionViewport};

    fn options() -> RenderOptions {
        RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 20.0, dynamic_names: true }
    }

    #[test]
    fn magnitude_versions_refresh_even_with_identical_cells_and_order() {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars.truncate(1); parsed.constellations.clear();
        let mut sky = crate::sky::create_sky_from_catalog(&parsed).unwrap();
        sky.stars[0].position = crate::astro::Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let view = View::default(); let size = ProjectionViewport { width: 24, height: 24 };
        let data = crate::projection::project_sky(&sky, &view, size);
        let assembled = [(0, 0, 1, 1, 1)];
        let mut region = [ObservedRegion { region: 0, start: 0, end: 1, selection_generation: 1, motion_generation: 1, apparent_generation: 1 }];
        let mut cache = SceneCache::default();
        let mut previous = None;
        for magnitude in [0.0, 4.0] {
            sky.stars[0].magnitude = magnitude;
            region[0].motion_generation += 1;
            let projected = RenderProjection { sky: data.view(&sky), source: (1, 1), regions: &region, assembled: &assembled, geometry: [1; 3] };
            let image = draw_prepared_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()).unwrap().clone();
            assert_eq!(image, crate::scene::draw_pixel_sky(projected.sky(), &options(), &mut StepTimes::default()).unwrap());
            if let Some(previous) = previous { assert_ne!(image, previous); }
            previous = Some(image);
            assert!(cache.pixel_inputs.is_empty());
            assert!(matches!(&cache.pixels.key().unwrap().stars, StarKeys::Pixels(rows) if rows.is_empty()));
            assert!(cache.pixels.key().unwrap().planets.is_empty());
        }
        assert_eq!(cache.pixels.stats.refreshes, 2);
    }

    #[test]
    fn failed_redraw_stays_invalid_and_retry_reuses_scratch() {
        let sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 20, height: 20 });
        let mut projected = RenderProjection { sky: data.view(&sky), source: (1, 1), regions: &[], assembled: &[], geometry: [1; 3] };
        let mut cache = SceneCache::default();
        draw_prepared_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()).unwrap();
        let expected = cache.pixel_image().clone();
        projected.sky.viewport.width = usize::MAX; // simulate a failed image-size request without a huge allocation
        assert!(draw_prepared_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()).is_none());
        assert!(cache.pixels.has_been_invalidated);
        assert_eq!(cache.pixels.stored().unwrap(), &expected);
        let capacity = cache.pixel_inputs.capacity();
        projected.sky.viewport.width = 20;
        let mut times = StepTimes::with_trace(true);
        assert_eq!(draw_prepared_pixels(&mut cache, &projected, &options(), 0.0, &mut times).unwrap(), &expected);
        assert!(times.trace().unwrap().steps.iter().any(|step| step.name == "Raster drawing inputs"));
        assert_eq!(cache.pixel_inputs.capacity(), capacity);
        assert!(cache.pixel_inputs.is_empty());
    }
}
