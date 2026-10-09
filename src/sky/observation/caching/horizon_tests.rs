//! Horizon rotation covers the bodies only; cached stars stay apparent and the view rotates them when read.
use crate::{astro::{J2000, Matrix3, Vector3, models::stars::StellarSample, refract_direction}, cache::{Cache, CacheConfig},
    model::{ApparentFrame, ObservedRegion, ObservedSky, ObservedStars, RegionData, SelectedStar}, state::ObservationCache, timing::StepTimes};

/// Three stars in three fixture regions (the middle one empty), with hand-made samples addressed by the fixture offsets.
struct Fixture { storage: ObservationCache, output: ObservedSky, samples: Vec<Cache<(), Vec<StellarSample>>>, offsets: [usize; 4] }

fn fixture() -> Fixture {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(3);
    source.constellations.clear();
    let mut output = crate::sky::create_sky_from_catalog(&source).unwrap();
    for (index, star) in output.stars.iter_mut().enumerate() { star.source_index = index; }
    let mut storage = ObservationCache::default();
    storage.regions.resize_with(3, Default::default);
    let offsets = [0, 2, 2, 3];
    let mut samples = Vec::new();
    for (id, start, end) in [(0, 0, 2), (1, 2, 2), (2, 2, 3)] {
        let rows: Vec<_> = output.stars[start..end].iter().enumerate().map(|(i, star)| SelectedStar { source_index: star.source_index, drawable: i == 0 }).collect();
        let directions: Vec<_> = (start..end).map(|i| Vector3 { x: 1.0, y: i as f64 + 0.5, z: -0.1 }.normalized()).collect();
        storage.regions[id].corrections.store((1, 1), J2000, 0.0, (rows, Default::default()));
        storage.regions[id].apparent.store((1, 1, Vector3::default()), J2000, 0.0, directions);
        storage.regional_output.push(ObservedRegion { region: id, start, end, selection_generation: 1, motion_generation: 1, apparent_generation: 1 });
        let mut region_samples = Cache::default();
        region_samples.store((), J2000, 0.0, output.stars[start..end].iter().map(|star| StellarSample { direction: Vector3::default(), magnitude: star.magnitude, used_singular_fallback: false }).collect());
        samples.push(region_samples);
    }
    let bodies = output.planets.iter().enumerate().map(|(i, _)| Vector3 { x: i as f64 + 1.0, y: 2.0, z: -0.1 }).collect();
    storage.body_apparent.store((1, Vector3::default()), J2000, 0.0, (bodies, Vector3 { x: 4.0, y: -2.0, z: 0.5 }));
    output.stars[1].drawable = false; // faint retained endpoint; it still has a direction
    Fixture { storage, output, samples, offsets }
}
impl Fixture {
    fn rotate(&mut self, rotation: Matrix3, times: &mut StepTimes) {
        super::corrections::update_horizon_rotation(&self.storage.body_apparent, &mut self.storage.horizontal, &self.storage.config, J2000, rotation, times);
        let bodies = self.storage.horizontal.value();
        for (planet, &direction) in self.output.planets.iter_mut().zip(&bodies.0) { planet.position = direction; }
        self.output.moon.position = bodies.1; // test-only materialization for the independent reference assertions
    }
    fn stars<'a>(&'a self, frame: &'a ApparentFrame) -> ObservedStars<'a> {
        ObservedStars::regional(&self.output.catalog.stars, &self.storage.regional_output, &self.storage.regions, &self.samples, &self.offsets, frame)
    }
    fn apparent(&self, index: usize) -> Vector3 {
        let region = self.storage.regional_output.iter().find(|region| region.start <= index && index < region.end).unwrap();
        self.storage.regions[region.region].apparent.value()[index - region.start]
    }
}
fn bodies(output: &ObservedSky) -> (Vec<Vector3>, Vector3) { (output.planets.iter().map(|body| body.position).collect(), output.moon.position) }
fn rotation() -> Matrix3 { Matrix3([[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]) }

#[test]
fn rotation_covers_the_bodies_only_and_never_changes_apparent_inputs() {
    let mut fixture = fixture();
    let saved: Vec<_> = fixture.storage.regions.iter().map(|region| region.apparent.clone()).collect();
    let apparent = fixture.storage.body_apparent.clone();
    let mut expected = fixture.output.clone();
    for (planet, &value) in expected.planets.iter_mut().zip(&apparent.value().0) { planet.position = value; }
    expected.moon.position = apparent.value().1;
    super::super::stages::rotate_sky_to_horizon(rotation(), &mut expected); // independent in-place reference over copied test inputs
    for bypass in [false, true] {
        fixture.storage.config = if bypass { CacheConfig::disabled() } else { CacheConfig::default() };
        for _ in 0..2 {
            let bad = Vector3 { x: f64::NAN, y: f64::NAN, z: f64::NAN };
            for body in &mut fixture.output.planets { body.position = bad; }
            fixture.output.moon.position = bad;
            fixture.rotate(rotation(), &mut StepTimes::default());
            assert_eq!(bodies(&fixture.output), bodies(&expected));
            super::corrections::update_refraction(&fixture.storage.horizontal, &mut fixture.storage.refracted, &fixture.storage.config, J2000, true, &mut StepTimes::default());
            let refracted = fixture.storage.refracted.value();
            assert_eq!(refracted.0, expected.planets.iter().map(|body| refract_direction(body.position)).collect::<Vec<_>>());
            assert_eq!(refracted.1, refract_direction(expected.moon.position));
            assert_eq!(fixture.storage.body_apparent, apparent);
            for (region, saved) in fixture.storage.regions.iter().zip(&saved) { assert!(region.apparent == *saved); }
        }
    }
    assert!(fixture.storage.horizontal.value().0.len() == fixture.output.planets.len(), "no star-sized direction buffer exists");
}

#[test]
fn body_changes_refresh_the_horizontal_result_and_equal_bodies_reuse_it() {
    let mut fixture = fixture();
    fixture.rotate(Matrix3::IDENTITY, &mut StepTimes::default());
    let old_generation = fixture.storage.horizontal.generation;
    let key = *fixture.storage.body_apparent.key().unwrap();
    let mut bodies = fixture.storage.body_apparent.value().clone();
    bodies.1 = Vector3 { x: 0.0, y: 2.0, z: 3.0 };
    fixture.storage.body_apparent.store(key, J2000, 0.0, bodies.clone());
    fixture.rotate(Matrix3::IDENTITY, &mut StepTimes::default());
    assert!(fixture.storage.horizontal.generation > old_generation);
    assert_eq!(fixture.output.moon.position, bodies.1);
    fixture.storage.body_apparent.store(key, J2000, 0.0, bodies);
    let before = fixture.storage.horizontal.stats;
    fixture.rotate(Matrix3::IDENTITY, &mut StepTimes::default());
    assert_eq!(fixture.storage.horizontal.stats.hits, before.hits + 1);
    fixture.rotate(Matrix3::rotate_z(0.2), &mut StepTimes::default());             // a new rotation is a new key
    assert_eq!(fixture.storage.horizontal.stats.refreshes, before.refreshes + 1);
}

#[test]
fn cached_stars_are_rotated_and_refracted_when_read_and_expose_their_apparent_frame() {
    let fixture = fixture();
    for refraction in [false, true] {
        let frame = ApparentFrame { horizon: rotation(), refraction };
        let stars = fixture.stars(&frame);
        assert_eq!(stars.len(), 3);
        for (slot, region) in fixture.storage.regional_output.iter().enumerate() {
            let (columns, base) = stars.slot_columns(slot);
            assert_eq!(base, region.start);
            assert_eq!(columns.apparent_frame(), Some(frame));
            for row in 0..columns.len() {
                let apparent = fixture.apparent(region.start + row);
                let rotated = rotation().apply(apparent);
                let expected = if refraction { refract_direction(rotated) } else { rotated };
                assert_eq!(columns.apparent_directions().unwrap()[row], apparent);          // the stored direction, no frame change
                assert_eq!(columns.position(row), expected);                              // the same arithmetic the direct path applies in place
                assert_eq!(columns.position(row), frame.to_horizontal(apparent));
                assert_eq!(columns.star(row), stars.get(region.start + row).state.into_owned());
                assert_eq!(columns.star(row).magnitude, fixture.output.stars[region.start + row].magnitude);
                assert_eq!(columns.drawable(row), row == 0);
            }
        }
        assert_eq!(stars.iter().map(|star| star.position).collect::<Vec<_>>(), (0..3).map(|index| frame.to_horizontal(fixture.apparent(index))).collect::<Vec<_>>());
    }
    let owned = [fixture.output.stars[0]];
    let flat = RegionData::Owned(&owned);
    assert_eq!(flat.apparent_frame(), None);                                               // owned rows are horizontal already
    assert_eq!(flat.apparent_directions(), None);
}

#[test]
fn mismatched_region_length_or_version_cannot_be_read() {
    for case in 0..4 {
        let mut fixture = fixture();
        let descriptor = fixture.storage.regional_output[0];
        match case {
            0 => fixture.storage.regional_output[0].end -= 1,                             // row range shorter than the records
            1 => fixture.storage.regional_output[0].apparent_generation += 1,              // apparent version does not match
            2 => fixture.storage.regional_output[0].selection_generation += 1,             // membership version does not match
            _ => {                                                                         // apparent directions calculated for older membership
                let region = &mut fixture.storage.regions[0];
                let mut records = region.corrections.value().clone();
                records.0[0].drawable = !records.0[0].drawable;
                region.corrections.store((2, 2), J2000, 0.0, records);
                fixture.storage.regional_output[0].selection_generation = region.corrections.generation;
            }
        }
        let frame = ApparentFrame { horizon: Matrix3::IDENTITY, refraction: false };
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.stars(&frame).slot_columns(0).0.apparent_directions())).is_err(), "case {case}");
        fixture.storage.regional_output[0] = descriptor;
        if case == 3 { continue; }
        assert_eq!(fixture.stars(&frame).slot_columns(0).0.apparent_directions().unwrap().len(), 2);
    }
}
