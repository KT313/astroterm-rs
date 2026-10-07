//! Aberration formulas and their numerical regression tests.
use crate::astro::{Vector3, LIGHT_SPEED_AU_DAY};
/// First-order annual plus diurnal aberration; error is O(beta²), below 0.002 arcseconds for Earth.
pub fn apply_aberration(direction: Vector3, velocity: Vector3) -> Vector3 {
    (direction.normalized() + velocity * (1.0 / LIGHT_SPEED_AU_DAY)).normalized()
}

/// Aberration for an already normalized stellar direction. `beta` is observer velocity / c, prepared once per pass.
/// The near-unit output has a bounded squared norm, so a square root suffices; retain robust normalization for
/// unusual synthetic velocities/cancellation. The generic distance-vector API above retains both normalizations.
pub(super) fn apply_unit_aberration(direction: Vector3, beta: Vector3) -> Vector3 {
    debug_assert!(
        (direction.dot(direction) - 1.0).abs() < 1e-12,
        "stellar direction must be unit length"
    );
    let apparent = direction + beta;
    let squared_norm = apparent.dot(apparent);
    if (0.5..=2.0).contains(&squared_norm) {
        apparent * (1.0 / squared_norm.sqrt())
    } else {
        apparent.normalized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_aberration_matches_generic_correction_and_preserves_normalization() {
        let mut maximum = 0.0_f64;
        for i in 0..10000 {
            let a = i as f64 * 0.013;
            let b = i as f64 * 0.029;
            let direction = Vector3 {
                x: a.cos() * b.cos(),
                y: a.sin() * b.cos(),
                z: b.sin(),
            }
            .normalized();
            for velocity in [
                Vector3::default(),
                Vector3 {
                    x: 0.0176,
                    y: -0.001,
                    z: 0.0003,
                },
                Vector3 {
                    x: -0.0004,
                    y: 0.017,
                    z: -0.001,
                },
                Vector3 {
                    x: 1000.0,
                    y: -500.0,
                    z: 0.0,
                },
                Vector3 {
                    x: 1e200,
                    y: 1e200,
                    z: 0.0,
                },
            ] {
                let expected = apply_aberration(direction, velocity);
                let actual = apply_unit_aberration(direction, velocity * (1.0 / LIGHT_SPEED_AU_DAY));
                let error = expected.cross(actual).length().atan2(expected.dot(actual)).to_degrees() * 3600.0;
                maximum = maximum.max(error);
                assert!(error < 1e-8, "aberration delta {error} arcseconds");
                assert!((actual.dot(actual) - 1.0).abs() < 2e-15);
            }
        }
        println!("maximum unit-aberration delta: {maximum:.12e} arcseconds");
    }

    #[test]
    fn aberration_points_towards_velocity_and_never_exceeds_its_geometric_bound() {
        let velocity = Vector3 {
            x: 0.0,
            y: 0.0176,
            z: 0.00027,
        };
        let beta = velocity.length() / LIGHT_SPEED_AU_DAY;
        for i in 0..1000 {
            let a = i as f64 * std::f64::consts::TAU / 1000.0;
            let direction = Vector3 {
                x: a.cos(),
                y: a.sin(),
                z: 0.0,
            };
            let apparent = apply_aberration(direction, velocity);
            let angle = direction.cross(apparent).length().atan2(direction.dot(apparent));
            assert!(angle <= beta.asin() + 1e-15);
            assert!(apparent.dot(velocity) >= direction.dot(velocity) - 1e-15);
        }
    }
    #[test]
    #[ignore = "release measurement of the phase-6 per-star correction"]
    fn measure_aberration_cost() {
        use std::hint::black_box;
        let directions: Vec<_> = (0..1_000_000)
            .map(|i| {
                let a = i as f64 * 0.001;
                Vector3 {
                    x: a.cos(),
                    y: a.sin(),
                    z: 0.2,
                }
                .normalized()
            })
            .collect();
        let velocity = Vector3 {
            x: 0.0,
            y: 0.0176,
            z: 0.00027,
        };
        let beta = velocity * (1.0 / LIGHT_SPEED_AU_DAY);
        for unit_input in [false, true] {
            let start = std::time::Instant::now();
            for direction in &directions {
                black_box(if unit_input {
                    apply_unit_aberration(black_box(*direction), black_box(beta))
                } else {
                    apply_aberration(black_box(*direction), black_box(velocity))
                });
            }
            println!(
                "unit_input={unit_input}: {:.3} ns/star (1,000,000 directions)",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
