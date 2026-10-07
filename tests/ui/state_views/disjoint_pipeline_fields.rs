use astroterm::canvas::Canvas;
use astroterm::catalog::{Catalog, StarNames};
use astroterm::model::{ObservedSky, ProjectionViewport, View, Frame, RenderOptions};
use astroterm::projection::{borrow_projected, project_cached_sky};
use astroterm::scene::draw_characters;
use astroterm::state::{ObservationCache, ProjectionCache, RenderingState, Caches, SceneCache};
use astroterm::timing::StepTimes;
use std::sync::Arc;

fn main() {
    let catalog = Catalog::new(Vec::new(), StarNames::default(), Vec::new());
    let sky = ObservedSky::new(Arc::new(astroterm::sky::prepare_owned_catalog(catalog).unwrap().catalog));
    let mut run = Caches {
        sky, simulation: astroterm::state::SimulationCaches::default(), selection: astroterm::state::StarSelectionCache::default(), observer: astroterm::state::ObserverPreparationCache::default(), observation: ObservationCache::default(),
        projection: ProjectionCache::default(), rendering: RenderingState::Pending,
    };
    let mut frame = Frame { sky: Canvas::new(8, 8), panel: Some(Canvas::new(1, 8)) };
    let mut scene = SceneCache::default();
    let mut times = StepTimes::default();
    let view = View::default();
    let viewport = ProjectionViewport { width: 8, height: 8 };
    let options = RenderOptions {
        unicode: false, braille: false, color: false, constellations: false, grid: false,
        magnitude_threshold: 5.0, label_threshold: 0.0, dynamic_names: false,
    };

    let Caches { sky, projection, simulation, .. } = &mut run;
    project_cached_sky(projection, sky, &view, viewport, 2451545.0, &mut times); // source is read while output is written
    let projected = borrow_projected(projection, sky, &view, viewport);
    simulation.solar_system.begin_frame(); // changing a disjoint state owner does not invalidate the borrowed projection
    draw_characters(&mut scene, &mut frame.sky, &projected, &options, 2451545.0);

    let definitions = sky.constellations();
    let mut counts = Vec::new();
    counts.extend(definitions.iter().map(|figure| figure.segments.len())); // read only the figures, write separate results
    assert_eq!(counts.len(), definitions.len());
    let pixels = &frame.sky;
    frame.panel.as_mut().unwrap().clear(); // one frame canvas can be changed while the other remains borrowed
    assert_eq!(pixels.height(), 8);
    assert!(projected.stars.is_empty());
    assert_eq!(run.sky.catalog.stars.len(), 0);
}

fn simulate_disjoint_fields(run: &mut Caches, time: f64, times: &mut StepTimes) {
    let selected = run.selection.stars(); // immutable rows remain borrowed while only stellar outputs change
    astroterm::sky::simulate_stars(&mut run.simulation.stars, selected, time, times);
    let result = run.simulation.stars.results(selected);
    run.simulation.solar_system.begin_frame(); // solar storage is independent from intrinsic star results
    assert_eq!(selected.rows().len(), result.samples().len());
}
