//! Horizon rotation borrows unchanged regional/body inputs and publishes only transformed directions.
use crate::{astro::{J2000, Matrix3, Vector3}, cache::CacheConfig,
    model::{ObservedRegion, ObservedSky, SelectedStar}, state::{ApparentDirections, ObservationCache}, timing::StepTimes};

fn fixture() -> (ObservationCache, ObservedSky) {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(3);
    source.constellations.clear();
    let mut output = crate::sky::create_sky_from_catalog(&source).unwrap();
    let mut storage = ObservationCache::default();
    storage.regions.resize_with(3, Default::default);
    for (id, start, end) in [(0, 0, 2), (1, 2, 2), (2, 2, 3)] {
        let rows: Vec<_> = output.stars[start..end].iter().enumerate().map(|(i, star)| SelectedStar { source_index: star.source_index, drawable: i == 0 }).collect();
        let directions: Vec<_> = (start..end).map(|i| Vector3 { x: 1.0, y: i as f64 + 0.5, z: -0.1 }.normalized()).collect();
        storage.regions[id].corrections.store((1, 1), J2000, 0.0, (rows, Default::default()));
        storage.regions[id].apparent.store((1, 1, Vector3::default()), J2000, 0.0, directions);
        storage.regional_output.push(ObservedRegion { region: id, start, end, selection_generation: 1, motion_generation: 1, apparent_generation: 1 });
    }
    let bodies = output.planets.iter().enumerate().map(|(i, _)| Vector3 { x: i as f64 + 1.0, y: 2.0, z: -0.1 }).collect();
    storage.body_apparent.store((1, Vector3::default()), J2000, 0.0, (bodies, Vector3 { x: 4.0, y: -2.0, z: 0.5 }));
    output.stars[1].drawable = false; // faint retained endpoint; its direction still needs rotation
    (storage, output)
}
fn rotate(storage: &mut ObservationCache, output: &mut ObservedSky, rotation: Matrix3, times: &mut StepTimes) {
    let input = ApparentDirections::new(&storage.regional_output, &storage.regions, &storage.body_apparent);
    super::corrections::update_horizon_rotation(input, &mut storage.horizontal_sources, &mut storage.horizontal, &mut storage.horizontal_work,
        &storage.config, J2000, rotation, times);
    let directions = storage.horizontal.value();
    for (star, &direction) in output.stars.iter_mut().zip(&directions.0) { star.position = direction; }
    for (planet, &direction) in output.planets.iter_mut().zip(&directions.1) { planet.position = direction; }
    output.moon.position = directions.2; // test-only materialization for the existing independent reference assertions
}
fn directions(output: &ObservedSky) -> (Vec<Vector3>, Vec<Vector3>, Vector3) {
    (output.stars.iter().map(|star| star.position).collect(), output.planets.iter().map(|body| body.position).collect(), output.moon.position)
}
fn poison(output: &mut ObservedSky) {
    let bad = Vector3 { x: f64::NAN, y: f64::NAN, z: f64::NAN };
    for star in &mut output.stars { star.position = bad; }
    for body in &mut output.planets { body.position = bad; }
    output.moon.position = bad;
}

#[test]
fn rotation_ignores_output_positions_and_never_changes_apparent_inputs() {
    let (mut storage, mut output) = fixture();
    let mut expected = output.clone();
    let saved: Vec<_> = storage.regions.iter().map(|region| region.apparent.clone()).collect();
    let bodies = storage.body_apparent.clone();
    let rotation = Matrix3([[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    for (region, values) in ApparentDirections::new(&storage.regional_output, &storage.regions, &storage.body_apparent).regions() {
        for (star, &value) in expected.stars[region.start..region.end].iter_mut().zip(values) { star.position = value; }
    }
    for (planet, &value) in expected.planets.iter_mut().zip(&bodies.value().0) { planet.position = value; }
    expected.moon.position = bodies.value().1;
    super::super::stages::rotate_sky_to_horizon(rotation, &mut expected); // independent in-place reference over copied test inputs
    for bypass in [false, true] {
        storage.config = if bypass { CacheConfig::disabled() } else { CacheConfig::default() };
        for _ in 0..2 {
            poison(&mut output);
            rotate(&mut storage, &mut output, rotation, &mut StepTimes::default());
            assert_eq!(directions(&output), directions(&expected));
            super::corrections::update_refraction(&storage.horizontal, &mut storage.refracted, &mut storage.refraction_work, &storage.config, J2000, true, &mut StepTimes::default());
            assert_eq!(storage.body_apparent, bodies);
            for (region, saved) in storage.regions.iter().zip(&saved) { assert!(region.apparent == *saved); }
        }
    }
}

#[test]
fn body_only_changes_refresh_horizontal_without_touching_stellar_regions() {
    let (mut storage, mut output) = fixture();
    rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    let old_stars: Vec<_> = output.stars.iter().map(|star| star.position).collect();
    let old_revision = storage.horizontal_sources.revision;
    let old_generation = storage.horizontal.generation;
    let key = *storage.body_apparent.key().unwrap();
    let mut bodies = storage.body_apparent.value().clone();
    bodies.1 = Vector3 { x: 0.0, y: 2.0, z: 3.0 };
    storage.body_apparent.store(key, J2000, 0.0, bodies.clone());
    rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    assert!(storage.horizontal_sources.revision > old_revision);
    assert!(storage.horizontal.generation > old_generation);
    assert_eq!(output.moon.position, bodies.1);
    assert_eq!(output.stars.iter().map(|star| star.position).collect::<Vec<_>>(), old_stars);
    let revision = storage.horizontal_sources.revision;
    storage.body_apparent.store(key, J2000, 0.0, bodies);
    let before = storage.horizontal.stats;
    rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    assert_eq!(storage.horizontal_sources.revision, revision);
    assert_eq!(storage.horizontal.stats.hits, before.hits + 1);
}

#[test]
fn mismatched_region_length_or_version_cannot_publish_horizontal_results() {
    for wrong_length in [false, true] {
        let (mut storage, mut output) = fixture();
        rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
        let saved = storage.horizontal.value().clone();
        let descriptor = storage.regional_output[0];
        if wrong_length { storage.regional_output[0].end -= 1; }
        else { storage.regional_output[0].apparent_generation += 1; }
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
        }));
        assert!(failed.is_err());
        assert_eq!(storage.horizontal.stored().unwrap(), &saved);
        assert!(storage.horizontal.has_been_invalidated);
        storage.regional_output[0] = descriptor;
        rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
        assert_eq!(storage.horizontal.generation, 1);
        assert_eq!(directions(&output), saved);
    }
}

#[test]
fn invalid_apparent_region_is_rejected_and_equal_retry_reuses_output_generation() {
    let (mut storage, mut output) = fixture();
    rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    storage.regions[0].apparent.invalidate();
    storage.horizontal.invalidate();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    })).is_err());
    let region = &mut storage.regions[0].apparent;
    region.store(*region.key().unwrap(), J2000, 0.0, region.stored().unwrap().clone());
    rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    assert_eq!(storage.horizontal.generation, 1);
}

#[test]
fn apparent_values_for_old_membership_are_rejected_even_when_result_versions_match() {
    let (mut storage, mut output) = fixture();
    let region = &mut storage.regions[0];
    let mut records = region.corrections.value().clone();
    records.0[0].drawable = !records.0[0].drawable;
    region.corrections.store((2, 2), J2000, 0.0, records);
    storage.regional_output[0].selection_generation = region.corrections.generation;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rotate(&mut storage, &mut output, Matrix3::IDENTITY, &mut StepTimes::default());
    })).is_err());
    assert!(storage.horizontal.stored().is_none());
}
