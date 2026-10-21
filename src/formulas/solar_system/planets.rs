//! VSOP87E geometric barycentric ecliptic J2000 states, rotated to equatorial J2000, AU and AU/day, TT≈TDB.
//! Bretagnon & Francou (1988). Earth is the geocenter in this variant, not the Earth–Moon barycenter.
//! The complete series are embedded (`VSOP87E_TERMS`, packed by `build.rs` from the published files in
//! `data/vsop87/`) and summed in one pass together with their first two time derivatives, so a body's velocity
//! and acceleration cost one sine and cosine per term instead of further evaluations of the series. The sums use
//! a polynomial sin/cos in four lanes, compiled for AVX2 when the CPU has it (series.rs).
//! Observer corrections live in observer.rs and apparent_position.rs. Physical accuracy is qualified separately from cache error.
use crate::formulas::{AstronomicalTime, BodyId, BodyMotion, BodyState, Vector3, ecliptic_to_equatorial};
use crate::formulas::constants::{J2000, VSOP87E_TERMS, VSOP87_MILLENNIUM_DAYS};
use super::series::{read_u32, series_terms, sum_series};

/// One body's barycentric position, velocity and acceleration at one TT epoch (the periodic TDB−TT difference
/// is neglected). Each coordinate is `Σ_k t^k S_k(t)` with `S_k = Σ a·cos(b + c·t)` over the series of power `k`;
/// the derivatives follow by the product rule from `S_k' = −Σ a·c·sin(b + c·t)` and `S_k'' = −Σ a·c²·cos(b + c·t)`.
pub fn evaluate_planet(body: BodyId, tt: f64) -> BodyMotion {
    let (counts, mut first) = locate_body(body);
    let terms = series_terms();
    let t = (tt - J2000) / VSOP87_MILLENNIUM_DAYS;
    let mut sums = [[0.0; 3]; 3];                                                         // per coordinate: value, first and second derivative per millennium
    for (coordinate, sum) in sums.iter_mut().enumerate() {
        for power in 0..6 {
            // the series of this power and its two derivatives
            let series = first..first + read_u32(counts, coordinate * 6 + power);
            let [value, slope, curvature] = sum_series(&terms.amplitudes[series.clone()], &terms.phases[series.clone()], &terms.frequencies[series.clone()], t);
            first = series.end;

            // t^k·S and its derivatives: k·t^(k−1)·S + t^k·S', then k(k−1)·t^(k−2)·S + 2k·t^(k−1)·S' + t^k·S''
            let k = power as f64;
            let t_power = t.powi(power as i32);
            let t_below = if power >= 1 { t.powi(power as i32 - 1) } else { 0.0 };
            let t_twice_below = if power >= 2 { t.powi(power as i32 - 2) } else { 0.0 };
            sum[0] += t_power * value;
            sum[1] += k * t_below * value + t_power * slope;
            sum[2] += k * (k - 1.0) * t_twice_below * value + 2.0 * k * t_below * slope + t_power * curvature;
        }
    }

    // ecliptic to equatorial J2000, per day instead of per millennium
    let per_day = 1.0 / VSOP87_MILLENNIUM_DAYS;
    BodyMotion {
        position: ecliptic_to_equatorial(Vector3 { x: sums[0][0], y: sums[1][0], z: sums[2][0] }),
        velocity: ecliptic_to_equatorial(Vector3 { x: sums[0][1], y: sums[1][1], z: sums[2][1] } * per_day),
        acceleration: ecliptic_to_equatorial(Vector3 { x: sums[0][2], y: sums[1][2], z: sums[2][2] } * (per_day * per_day)),
    }
}

/// One barycentric position at TT.
pub fn compute_barycentric_position(body: BodyId, tt: f64) -> Vector3 {
    evaluate_planet(body, tt).position
}

/// The Earth's state at an application timestamp: what step 2 takes from step 1 when both run.
pub fn evaluate_earth_state(time_ms: i64) -> BodyState {
    evaluate_planet(BodyId::Earth, AstronomicalTime::from_time_ms(time_ms).tt_jd).state()
}

/// Planetary batch, including the moving Sun and Earth, at one TT epoch.
pub fn evaluate_planets(tt: f64) -> [BodyMotion; 9] {
    let mut states = [BodyMotion::default(); 9];
    for (state, body) in states.iter_mut().zip(BodyId::PLANETS) {
        *state = evaluate_planet(body, tt);
    }
    states
}

/// The 18 term counts of `body` inside `VSOP87E_TERMS` (coordinate-major, powers 0 to 5), and the index of its
/// first term among the decoded terms of all bodies.
fn locate_body(body: BodyId) -> (&'static [u8], usize) {
    assert!(body != BodyId::Moon, "Moon belongs to the lunar family");
    let (mut offset, mut first) = (8, 0);                                                // after the magic
    for _ in 0..body as usize {
        let mut terms = 0;
        for index in 0..18 {
            terms += read_u32(&VSOP87E_TERMS[offset..], index);
        }
        offset += 18 * 4 + 24 * terms;
        first += terms;
    }
    (&VSOP87E_TERMS[offset..offset + 18 * 4], first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::constants::VELOCITY_SAMPLE_OFFSET_DAYS;

    fn read_reference_vector(row: &serde_json::Value, field: &str) -> Vector3 {
        Vector3 {
            x: row[field][0].as_f64().unwrap(),
            y: row[field][1].as_f64().unwrap(),
            z: row[field][2].as_f64().unwrap(),
        }
    }

    #[test]
    fn the_embedded_tables_hold_the_complete_published_series() {
        assert_eq!(&VSOP87E_TERMS[..8], b"VSOP87E1");
        let published = [6634, 7880, 4657, 5556, 7575, 4542, 7522, 5125, 2389];           // terms per VSOP87E file, Sun to Neptune
        let mut total = 0;
        for (body, expected) in BodyId::PLANETS.into_iter().zip(published) {
            let (counts, first) = locate_body(body);
            let mut terms = 0;
            for index in 0..18 { terms += read_u32(counts, index); }
            assert_eq!(terms, expected, "{body:?}");
            assert_eq!(first, total, "{body:?}");
            total += terms;
        }
        assert_eq!(VSOP87E_TERMS.len(), 8 + 9 * 18 * 4 + 24 * total);
    }

    /// The series as summed before the polynomial kernel: one libm `sin_cos` per term, one running sum each.
    fn evaluate_planet_with_libm(body: BodyId, tt: f64) -> BodyMotion {
        let (counts, mut first) = locate_body(body);
        let terms = series_terms();
        let t = (tt - J2000) / VSOP87_MILLENNIUM_DAYS;
        let mut sums = [[0.0; 3]; 3];
        for (coordinate, sum) in sums.iter_mut().enumerate() {
            for power in 0..6 {
                let series = first..first + read_u32(counts, coordinate * 6 + power);
                let (mut value, mut slope, mut curvature) = (0.0, 0.0, 0.0);
                for index in series.clone() {
                    let (a, b, c) = (terms.amplitudes[index], terms.phases[index], terms.frequencies[index]);
                    let (sine, cosine) = (b + c * t).sin_cos();
                    value += a * cosine;
                    slope -= a * c * sine;
                    curvature -= a * c * c * cosine;
                }
                first = series.end;
                let k = power as f64;
                let t_power = t.powi(power as i32);
                let t_below = if power >= 1 { t.powi(power as i32 - 1) } else { 0.0 };
                let t_twice_below = if power >= 2 { t.powi(power as i32 - 2) } else { 0.0 };
                sum[0] += t_power * value;
                sum[1] += k * t_below * value + t_power * slope;
                sum[2] += k * (k - 1.0) * t_twice_below * value + 2.0 * k * t_below * slope + t_power * curvature;
            }
        }
        let per_day = 1.0 / VSOP87_MILLENNIUM_DAYS;
        BodyMotion {
            position: ecliptic_to_equatorial(Vector3 { x: sums[0][0], y: sums[1][0], z: sums[2][0] }),
            velocity: ecliptic_to_equatorial(Vector3 { x: sums[0][1], y: sums[1][1], z: sums[2][1] } * per_day),
            acceleration: ecliptic_to_equatorial(Vector3 { x: sums[0][2], y: sums[1][2], z: sums[2][2] } * (per_day * per_day)),
        }
    }

    #[test]
    fn series_sums_match_the_libm_reference_within_rounding() {
        let (mut position, mut velocity, mut acceleration) = (0.0_f64, 0.0_f64, 0.0_f64);
        for tt in super::super::series::tests::series_epochs() {
            for body in BodyId::PLANETS {
                let (actual, reference) = (evaluate_planet(body, tt), evaluate_planet_with_libm(body, tt));
                position = position.max((actual.position - reference.position).length());
                velocity = velocity.max((actual.velocity - reference.velocity).length());
                acceleration = acceleration.max((actual.acceleration - reference.acceleration).length());
            }
        }
        eprintln!("largest differences from libm: position {position:e} AU, velocity {velocity:e} AU/day, acceleration {acceleration:e} AU/day²");
        assert!(position <= 1e-12 && velocity <= 1e-14 && acceleration <= 1e-16, "{position:e} AU, {velocity:e} AU/day, {acceleration:e} AU/day²");
    }

    #[test]
    fn the_derivatives_match_finite_differences_of_the_series() {
        let h = VELOCITY_SAMPLE_OFFSET_DAYS;
        for body in [BodyId::Mercury, BodyId::Earth, BodyId::Jupiter, BodyId::Sun] {
            for tt in [J2000, 2461041.5, J2000 - 100.0 * 365.25] {
                let motion = evaluate_planet(body, tt);
                let (before, after) = (evaluate_planet(body, tt - h), evaluate_planet(body, tt + h));
                let velocity = (after.position - before.position) * (1.0 / (2.0 * h));
                let acceleration = (after.velocity - before.velocity) * (1.0 / (2.0 * h));
                assert!((motion.velocity - velocity).length() < 1e-10, "{body:?} {tt}: {:e}", (motion.velocity - velocity).length());
                assert!((motion.acceleration - acceleration).length() < 1e-9, "{body:?} {tt}: {:e}", (motion.acceleration - acceleration).length());
                assert!(motion.velocity.length() > 1e-6 || body == BodyId::Sun);         // the Sun barely moves around the barycentre
            }
        }
    }

    /// Release-mode cost of one Earth evaluation; run with `--release --ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_one_earth_evaluation() {
        let started = std::time::Instant::now();
        let evaluations = 2000;
        for index in 0..evaluations {
            std::hint::black_box(evaluate_planet(BodyId::Earth, J2000 + f64::from(index) / 1024.0));
        }
        eprintln!("evaluate_planet(Earth) {:?} per call", started.elapsed() / evaluations);
    }

    /// Release-mode cost of the whole planetary batch, Sun to Neptune; run with `--release --ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_all_bodies_evaluation() {
        let started = std::time::Instant::now();
        let evaluations = 200;
        for index in 0..evaluations {
            std::hint::black_box(evaluate_planets(J2000 + f64::from(index) / 1024.0));
        }
        eprintln!("evaluate_planets {:?} per call", started.elapsed() / evaluations);
    }

    #[test]
    fn earth_heliocentric_state_matches_independent_horizons_near_today() {
        let references: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/reference/references.json")).unwrap();
        let earth = &references["barycentric_vectors"]["earth"]["rows"][0];
        let sun = &references["barycentric_vectors"]["sun"]["rows"][0];
        let tt = earth["jd_tdb"].as_f64().unwrap(); // TT≈TDB, as documented by the reference generator
        let (earth_motion, sun_motion) = (evaluate_planet(BodyId::Earth, tt), evaluate_planet(BodyId::Sun, tt));
        let error = ((earth_motion.position - sun_motion.position) - (read_reference_vector(earth, "position_au") - read_reference_vector(sun, "position_au"))).length();
        assert!(error < 0.0001, "Earth position error {error} AU");
        let velocity_error = ((earth_motion.velocity - sun_motion.velocity) - (read_reference_vector(earth, "velocity_au_day") - read_reference_vector(sun, "velocity_au_day"))).length();
        assert!(velocity_error < 1e-6, "Earth velocity error {velocity_error} AU/day");
    }
}
