//! The simulation cache of one group (plan section 4, "Simulation"): where the group's brightest stars are and
//! how bright they are at one simulated epoch.
//!
//! **Key:** the simulated epoch (TT) the rows were evaluated at. **Invalidated by:** the simulated time moving
//! more than the group's hold time from that epoch, in either direction; then every row is evaluated again at
//! the new time. Nothing else invalidates it: rotation, panning, zoom and time inside the hold leave it alone. A
//! deeper magnitude threshold appends the rows it needs, evaluated at the stored epoch so that all rows share
//! one epoch; a shallower threshold keeps the rows (the drawing step uses its own prefix count). Outside the
//! computational interval the stored brightness keys do not apply, so the whole table is simulated.
use crate::formulas::years_since_j2000;
use crate::formulas::{COMPUTATIONAL_INTERVAL, Vector3};
use crate::catalog::{decode_magnitude, magnitude_code};
use crate::model::{StarGroup, StarTable, SimulationCache};

/// A row the motion model cannot place (its straight line passes through the Sun); never drawn.
pub const UNPLACED: u16 = u16::MAX;

/// What `SimulationCache::update` did, for tests and timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    Unchanged,
    Extended(usize),                        // rows appended at the stored epoch
    Refreshed(usize),                       // rows evaluated at the new epoch
}

impl StarGroup {
    /// Update samples and their integer timestamp together. tt_jd is derived from time_ms once by the refresh pass.
    pub fn update_simulation(&mut self, time_ms: i64, tt_jd: f64, rows: usize) -> Refresh {
        // Pass the group's original stars, cache and validity information to the shared update routine.
        self.simulation.update(
            &self.stars,
            time_ms,
            tt_jd,
            rows,
            self.ttl_ms,
            &mut self.last_update_ms,
        )
    }
}

impl SimulationCache {
    pub fn len(&self) -> usize { self.directions.len() }
    pub fn is_empty(&self) -> bool { self.directions.is_empty() }
    pub fn directions(&self) -> &[Vector3] { &self.directions }
    pub fn magnitudes(&self) -> &[u16] { &self.magnitudes }

    /// Bring the first `rows` rows up to date for `time_ms`, evaluating at the supplied derived `tt_jd` when the hold has
    /// expired, otherwise append missing rows at the stored date. `last_update_ms` belongs to the owner and must
    /// describe these samples. The exact TTL boundary remains valid, in either time direction.
    pub fn update(&mut self, table: &StarTable, time_ms: i64, tt_jd: f64, rows: usize, hold_ms: i64, last_update_ms: &mut Option<i64>) -> Refresh {
        // Recalculate all requested rows if this is the first simulation or the stored date is too old.
        assert!(hold_ms >= 0, "simulation TTL must be nonnegative");
        let expired = last_update_ms.is_none_or(|epoch_ms| (i128::from(time_ms) - i128::from(epoch_ms)).abs() > i128::from(hold_ms));
        if expired {
            *last_update_ms = Some(time_ms);
            self.directions.clear();
            self.magnitudes.clear();
            self.evaluate_rows(table, tt_jd, rows);
            return Refresh::Refreshed(rows);
        }

        // Keep complete valid samples, or append missing rows at their existing simulation date.
        let missing = rows.saturating_sub(self.len());
        if missing == 0 { return Refresh::Unchanged; }
        let stored_date = last_update_ms.expect("an epoch exists once the hold did not expire");
        self.evaluate_rows(table, crate::formulas::AstronomicalTime::from_time_ms(stored_date).tt_jd, rows);
        Refresh::Extended(missing)
    }

    fn evaluate_rows(&mut self, table: &StarTable, tt: f64, rows: usize) {
        let years = years_since_j2000(tt);
        let columns = table.columns();
        for row in self.len()..rows {
            let sample = table.motion(row).evaluate(years, decode_magnitude(columns.magnitude[row]));
            self.directions.push(sample.direction);
            self.magnitudes.push(if sample.used_singular_fallback { UNPLACED } else { magnitude_code(sample.magnitude) });
        }
    }
}

/// How many rows of a group table a frame needs: the brightness prefix for the threshold inside the
/// computational interval, the whole table outside it, where the stored brightness keys are not valid.
pub fn rows_wanted(table: &StarTable, tt: f64, threshold: f64) -> usize {
    if COMPUTATIONAL_INTERVAL.contains(tt) { table.bright_count(threshold) } else { table.len() }
}

#[cfg(test)]
mod tests {
    use crate::model::get_ttl_of_speed_group_ms;
    use super::*;
    use crate::formulas::{J2000, JULIAN_YEAR_DAYS};
    use crate::catalog::{SpaceMotion, StarId, encode_brightness_bound, load_embedded_catalog, magnitude_code};
    use crate::formulas::{Equatorial, StellarMotion, computational_years};
    use proptest::prelude::*;
    use crate::constants::{GRID_DEPTH, SPEED_GROUP_COUNT};
    use crate::model::{Data, hash_direction};

    fn tables_of(catalog: crate::catalog::Catalog) -> Data {
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn bsc5() -> Data { tables_of(load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap()) }

    /// One star at `direction` (unit) with the given motion per year in units of its distance, as a catalog of its own.
    fn single_star(direction: Vector3, velocity: Vector3, magnitude: f64) -> Data {
        let mut parsed = load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        let mut star = parsed.stars[0].clone();
        parsed.stars.clear();
        parsed.constellations.clear();
        parsed.hr_representatives.clear();
        star.id = StarId(1);
        star.has_data = true;
        star.magnitude = magnitude;
        star.right_ascension = direction.y.atan2(direction.x);
        star.declination = direction.z.asin();
        star.space_motion = Some(SpaceMotion { distance_pc: 1.0, position: direction, velocity });
        parsed.stars.push(star);
        tables_of(parsed)
    }

    /// The one table that holds stars in a single-star catalog.
    fn only_table(tables: &Data) -> (&StarTable, usize) {
        tables.star_tables().find_map(|(_, group, table)| (!table.is_empty()).then_some((table, group))).unwrap()
    }

    fn date_ms(days_from_j2000: f64) -> i64 { crate::formulas::utc_jd_to_time_ms(J2000 + days_from_j2000).unwrap() }
    fn tt(time_ms: i64) -> f64 { crate::formulas::AstronomicalTime::from_time_ms(time_ms).tt_jd }

    proptest! {
        #[test]
        fn evaluated_magnitude_codes_never_pass_the_stored_brightness_key(
            ra in 0.0_f64..std::f64::consts::TAU, dec in -1.5_f64..1.5,
            wx in -1e-4_f64..1e-4, wy in -1e-4_f64..1e-4, wz in -1e-4_f64..1e-4,
            distance in proptest::option::of(1.0_f64..1e4), magnitude in -1.5_f64..20.0, fraction in 0.0_f64..=1.0,
        ) {
            // The prefix cut compares the key's code, rounded towards brighter, and drawing the sample's code, so a
            // sample may never round to a brighter code than its key. Velocities up to 1e-4 distances per year
            // reach the singular policy for the nearest stars.
            let direction = Equatorial { right_ascension: ra, declination: dec }.to_unit_vector();
            let mut star = StellarMotion { u0: direction, w: Vector3 { x: wx, y: wy, z: wz }, distance_pc: distance };
            star.remove_singular_distance();
            let (key, _) = encode_brightness_bound(star.brightest_magnitude(magnitude));
            let (start, end) = computational_years();
            for years in [start, start + (end - start) * fraction, end, 0.0] {
                let sample = star.evaluate(years, magnitude);
                prop_assert!(magnitude_code(sample.magnitude) >= key, "{} brighter than the key code {key} at {years}", sample.magnitude);
            }
        }
    }

    #[test]
    fn loaded_stars_take_the_singular_fallback_exactly_where_the_overflow_safe_length_did() {
        // The evaluation measures |q| with sqrt of the dot product, the load-time singular policy with nested hypot;
        // no catalog star may sit so close to the singular ratio that the two disagree.
        let hypot_length = |v: Vector3| v.x.hypot(v.y).hypot(v.z);
        let (start, end) = crate::formulas::computational_years();
        let mut evaluated = 0;
        for (_, _, table) in bsc5().star_tables() {
            for row in 0..table.len() {
                let motion = table.motion(row);
                for years in [start, -2000.0, 0.0, 26.0, 1000.0, end] {
                    let singular_before = motion.distance_pc.is_some() && hypot_length(motion.u0 + motion.w * years) < crate::formulas::SINGULAR_RATIO;
                    assert_eq!(motion.evaluate(years, 5.0).used_singular_fallback, singular_before, "row {row} at {years} years");
                    evaluated += 1;
                }
            }
        }
        assert!(evaluated > 50_000, "{evaluated}");
    }

    #[test]
    fn the_hold_boundary_is_absolute_in_both_directions_for_every_group() {
        let tables = bsc5();
        let start_ms = date_ms(0.0);
        for group in 0..SPEED_GROUP_COUNT {
            let (region, _, table) = tables.star_tables().find(|&(_, g, table)| g == group && table.len() >= 2).unwrap();
            let hold_ms = get_ttl_of_speed_group_ms(group);
            let mut simulations = tables.clone();
            assert_eq!(simulations.star_regions[region].speed_groups[group].update_simulation(start_ms, tt(start_ms), table.len()), Refresh::Refreshed(table.len()));
            let before = simulations.star_regions[region].speed_groups[group].simulation.clone();
            for offset in [0, hold_ms / 2, -hold_ms / 2, hold_ms, -hold_ms] {
                let now_ms = start_ms + offset;
                assert_eq!(simulations.star_regions[region].speed_groups[group].update_simulation(now_ms, tt(now_ms), table.len()), Refresh::Unchanged);
                assert_eq!(simulations.star_regions[region].speed_groups[group].last_update_ms, Some(start_ms));
                assert_eq!(simulations.star_regions[region].speed_groups[group].simulation, before);
            }
            for expired_ms in [start_ms + hold_ms + 1, start_ms - hold_ms - 1] {
                let mut fresh = simulations.clone();
                assert_eq!(fresh.star_regions[region].speed_groups[group].update_simulation(expired_ms, tt(expired_ms), table.len()), Refresh::Refreshed(table.len()));
                assert_eq!(fresh.star_regions[region].speed_groups[group].last_update_ms, Some(expired_ms));
                assert_ne!(fresh.star_regions[region].speed_groups[group].simulation.directions(), before.directions());
            }
        }
    }

    #[test]
    fn zero_is_a_valid_timestamp_and_zero_hold_expires_after_one_millisecond() {
        let tables = bsc5();
        let (_, _, table) = tables.star_tables().find(|&(_, _, table)| table.len() >= 2).unwrap();
        let mut simulation = SimulationCache::default();
        let mut last_update_ms = None;
        assert_eq!(simulation.update(table, 0, tt(0), table.len(), 0, &mut last_update_ms), Refresh::Refreshed(table.len()));
        assert_eq!(simulation.update(table, 0, tt(0), table.len(), 0, &mut last_update_ms), Refresh::Unchanged);
        assert_eq!(simulation.update(table, 1, tt(1), table.len(), 0, &mut last_update_ms), Refresh::Refreshed(table.len()));
        // Opposite i64 endpoints must not overflow the expiration subtraction.
        last_update_ms = Some(i64::MIN);
        assert_eq!(simulation.update(table, i64::MAX, tt(0), 0, i64::MAX, &mut last_update_ms), Refresh::Refreshed(0));
    }

    #[test]
    fn a_deeper_threshold_appends_rows_at_the_stored_epoch_and_a_shallower_one_keeps_them() {
        let tables = bsc5();
        let (region, group, table) = tables.star_tables().find(|&(_, _, table)| table.len() >= 3).unwrap();
        let columns = table.columns();
        let (faint, bright) = (decode_magnitude(columns.magnitude[table.len() - 1]), decode_magnitude(columns.magnitude[0]));
        let mut simulations = tables.clone();
        let first = simulations.star_regions[region].speed_groups[group].update_simulation(date_ms(0.0), tt(date_ms(0.0)), rows_wanted(table, tt(date_ms(0.0)), bright));
        let Refresh::Refreshed(rows) = first else { panic!("{first:?}") };
        assert!(rows >= 1 && rows < table.len());
        let later = date_ms(0.0) + 86_400_000;                                                            // inside every hold
        let extended = simulations.star_regions[region].speed_groups[group].update_simulation(later, tt(later), rows_wanted(table, tt(later), faint));
        assert_eq!(extended, Refresh::Extended(table.len() - rows));
        let simulation = &simulations.star_regions[region].speed_groups[group].simulation;
        assert_eq!((simulations.star_regions[region].speed_groups[group].last_update_ms, simulation.len()), (Some(date_ms(0.0)), table.len()));
        let years = years_since_j2000(tt(date_ms(0.0)));
        for row in 0..table.len() {                                                           // every row, old and new, is the model's value at the epoch
            let sample = table.motion(row).evaluate(years, decode_magnitude(columns.magnitude[row]));
            assert_eq!((simulation.directions()[row], simulation.magnitudes()[row]), (sample.direction, magnitude_code(sample.magnitude)), "row {row}");
        }
        assert_eq!((simulations.star_regions[region].speed_groups[group].update_simulation(later, tt(later), rows_wanted(table, tt(later), bright))), Refresh::Unchanged);
        assert_eq!(simulations.star_regions[region].speed_groups[group].simulation.len(), table.len());
    }

    #[test]
    fn simulated_rows_equal_direct_evaluation_for_a_whole_region() {
        let tables = bsc5();
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.2, y: -0.7, z: 0.6 });
        let time = date_ms(5000.0 * JULIAN_YEAR_DAYS);
        let mut simulations = tables.clone();
        for (group, table) in tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).enumerate() {
            (simulations.star_regions[region].speed_groups[group].update_simulation(time, tt(time), rows_wanted(table, tt(time), 6.5)));
            let simulation = &simulations.star_regions[region].speed_groups[group].simulation;
            assert_eq!(simulation.len(), table.len());
            for row in 0..table.len() {
                let sample = table.motion(row).evaluate(years_since_j2000(tt(time)), decode_magnitude(table.columns().magnitude[row]));
                assert_eq!(simulation.directions()[row], sample.direction);
                assert_eq!(simulation.magnitudes()[row], magnitude_code(sample.magnitude));
            }
        }
    }

    #[test]
    fn threshold_crossing_uses_the_brightness_key_then_the_current_magnitude() {
        let tables = single_star(Vector3 { x: 0.0, y: 0.0, z: 1.0 }, Vector3 { x: 0.0, y: 0.0, z: -0.00005 }, 5.5); // approaching along the line of sight
        let (table, group) = only_table(&tables);
        let mut simulation = SimulationCache::default();
        let mut last_update_ms = None;
        for (years, drawn) in [(0.0, false), (3000.0, false), (5000.0, true), (15000.0, true)] {
            let time = date_ms(years * JULIAN_YEAR_DAYS);
            let rows = rows_wanted(table, tt(time), 5.0);
            assert_eq!(rows, 1, "{years}: the key says it may brighten past 5.0");
            simulation.update(table, time, tt(time), rows, 0, &mut last_update_ms);
            assert_eq!(simulation.magnitudes()[0] <= magnitude_code(5.0), drawn, "{years} years");
        }
        let time = date_ms(20000.0 * JULIAN_YEAR_DAYS);                                           // the straight line passes the Sun: singular
        simulation.update(table, time, tt(time), rows_wanted(table, tt(time), 5.0), 0, &mut last_update_ms);
        assert_eq!(simulation.magnitudes(), [UNPLACED]);
        let sample = table.motion(0).evaluate(20000.0, 5.5);
        assert!(sample.used_singular_fallback && sample.magnitude == 5.5);           // the model falls back to the catalog brightness
        assert_eq!(group, 0);                                                                // radial motion is no proper motion: the slow group
    }

    #[test]
    fn outside_the_interval_the_whole_table_is_simulated_and_judged_by_current_magnitude() {
        let tables = single_star(Vector3 { x: 1.0, y: 0.0, z: 0.0 }, Vector3 { x: -0.00004, y: 0.0, z: 0.0 }, 6.0);
        let (table, _) = only_table(&tables);
        let inside = date_ms(0.0);
        assert_eq!(rows_wanted(table, tt(inside), 4.0), 0);                                         // the key keeps it out at threshold 4
        let outside = date_ms(24000.0 * JULIAN_YEAR_DAYS);
        assert_eq!(rows_wanted(table, tt(outside), 4.0), 1);
        let mut simulation = SimulationCache::default();
        let mut last_update_ms = None;
        simulation.update(table, outside, tt(outside), 1, 0, &mut last_update_ms);
        assert!(simulation.magnitudes()[0] < magnitude_code(4.0));                             // much closer, so much brighter
    }

    #[test]
    fn the_constellation_table_is_simulated_whole_with_the_fast_hold() {
        let tables = bsc5();
        let mut simulations = tables.clone();
        let table = &tables.constellation_stars;
        let hold_ms = get_ttl_of_speed_group_ms(SPEED_GROUP_COUNT - 1);
        let start_ms = date_ms(0.0);
        assert_eq!(simulations.constellation_simulation.update(table, start_ms, tt(start_ms), table.len(), hold_ms, &mut simulations.constellation_last_update_ms), Refresh::Refreshed(table.len()));
        assert_eq!(simulations.constellation_simulation.len(), table.len());
        assert_eq!(simulations.constellation_simulation.update(table, start_ms + hold_ms, tt(start_ms + hold_ms), table.len(), hold_ms, &mut simulations.constellation_last_update_ms), Refresh::Unchanged);
        assert_eq!(simulations.constellation_simulation.update(table, start_ms + hold_ms + 1, tt(start_ms + hold_ms + 1), table.len(), hold_ms, &mut simulations.constellation_last_update_ms), Refresh::Refreshed(table.len()));
    }

    /// Release-mode cost of one star evaluation; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_evaluation_of_every_bsc5_star() {
        let tables = bsc5();
        let mut simulations = tables.clone();
        let started = std::time::Instant::now();
        let mut rows = 0;
        for pass in 0..20 {
            let time = date_ms(f64::from(pass) * 400.0);                                          // every pass expires every hold
            for (region, group, table) in tables.star_tables() {
                if let Refresh::Refreshed(n) = simulations.star_regions[region].speed_groups[group].update_simulation(time, tt(time), rows_wanted(table, tt(time), 99.0)) { rows += n; }
            }
        }
        let elapsed = started.elapsed();
        eprintln!("{rows} star evaluations in {elapsed:?}: {:.0} ns per star", elapsed.as_nanos() as f64 / rows as f64);
    }
}
