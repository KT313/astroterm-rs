use super::*;
use crate::catalog::StarId;

fn source() -> GaiaSource {
    GaiaSource { id: StarId(0), source_id: 4295806720, ra: 0.0, dec: 0.0, epoch: 2016.0,
        pmra: Some(100.0), pmdec: Some(-50.0), parallax: Some(100.0), parallax_signal: Some(50.0),
        radial_velocity: Some(30.0), magnitude: 7.0, bp: Some(8.0), rp: Some(6.0) }
}
fn assert_direction(actual: Vector3, expected: Vector3) {
    let separation = actual.cross(expected);
    assert!(separation.x.hypot(separation.y).hypot(separation.z) < 1e-12, "{actual:?} {expected:?}");
    assert!(actual.dot(expected) > 0.999999999999);
}

#[test]
fn rebasing_preserves_independent_cartesian_trajectory_and_g_brightness() {
    for radial in [-30.0, 0.0, 30.0] {
        let raw = GaiaSource { radial_velocity: Some(radial), ..source() };
        let star = convert_source(raw, &mut ConversionStats::default()).unwrap();
        let p = Vector3 { x: 10.0, y: 0.0, z: 0.0 };
        let v = Vector3 { x: radial * KM_S_TO_PC_YEAR, y: 10.0 * 100.0 * MAS_TO_RADIANS, z: -10.0 * 50.0 * MAS_TO_RADIANS };
        for year in [-100.0, 0.0, 16.0, 500.0] {
            let expected = p + v * (year - 16.0);
            let distance = expected.x.hypot(expected.y).hypot(expected.z);
            let q = star.motion.u0 + star.motion.w * year;
            let ratio = q.x.hypot(q.y).hypot(q.z);
            assert_direction(q * (1.0 / ratio), expected * (1.0 / distance));
            let magnitude = star.magnitude + 5.0 * ratio.log10();
            assert!((magnitude - (7.0 + 5.0 * (distance / 10.0).log10())).abs() < 1e-12);
        }
    }
}

#[test]
fn missing_and_unreliable_distance_keeps_angular_motion_without_brightness_evolution() {
    for parallax in [None, Some(-1.0), Some(0.0), Some(0.001)] {
        let mut stats = ConversionStats::default();
        let star = convert_source(GaiaSource { parallax, ..source() }, &mut stats).unwrap();
        assert_eq!(star.motion.distance_pc, None);
        assert_eq!(star.magnitude, 7.0);
        assert_eq!(stats.angular_only, 1);
        assert_direction(star.motion.evaluate(16.0, star.magnitude).direction, Vector3 { x: 1.0, y: 0.0, z: 0.0 });
    }
    assert!(convert_source(GaiaSource { parallax_signal: Some(9.99), ..source() }, &mut ConversionStats::default()).unwrap().motion.distance_pc.is_none());
    let mut stats = ConversionStats::default();
    let no_rv = convert_source(GaiaSource { radial_velocity: None, ..source() }, &mut stats).unwrap();
    let zero_rv = convert_source(GaiaSource { radial_velocity: Some(0.0), ..source() }, &mut ConversionStats::default()).unwrap();
    assert_eq!(no_rv, zero_rv);
    assert_eq!(stats.missing_rv, 1);
}

#[test]
fn missing_motion_stays_stationary_and_poles_remain_finite() {
    let mut stats = ConversionStats::default();
    let star = convert_source(GaiaSource { pmra: None, ..source() }, &mut stats).unwrap();
    assert_eq!(star.motion.w, Vector3::default());
    assert_eq!(star.magnitude, 7.0);
    assert_eq!(stats.missing_motion, 1);
    for dec in [-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2] {
        let star = convert_source(GaiaSource { dec, ..source() }, &mut stats).unwrap();
        let original = Equatorial { right_ascension: 0.0, declination: dec }.to_unit_vector();
        assert_direction(star.motion.evaluate(16.0, star.magnitude).direction, original);
    }
}

#[test]
fn color_mapping_is_monotone_and_matches_palette_boundaries() {
    let mut stats = ConversionStats::default();
    for i in 0..4000 { assert!(gaia_color(-0.5 + (i + 1) as f64 / 1000.0) > gaia_color(-0.5 + i as f64 / 1000.0)); }
    for (bv, below, at) in [(0.0, StarColor::BlueWhite, StarColor::Default), (0.8, StarColor::Default, StarColor::Orange), (1.4, StarColor::Orange, StarColor::RedOrange)] {
        assert_eq!(select_color(Some(gaia_color(bv) - 1e-10), &mut stats), below);
        assert_eq!(select_color(Some(gaia_color(bv)), &mut stats), at);
    }
    assert_eq!(select_color(None, &mut stats), StarColor::Default);
    assert_eq!(select_color(Some(-100.0), &mut stats), StarColor::BlueWhite);
    assert_eq!(select_color(Some(100.0), &mut stats), StarColor::RedOrange);
    assert_eq!((stats.missing_color, stats.saturated_color), (1, 2));
}
