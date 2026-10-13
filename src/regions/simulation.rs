//! The simulation cache of one group (plan section 4, "Simulation"): where the group's brightest stars are and
//! how bright they are at one simulated epoch.
//!
//! **Key:** the simulated epoch (TT) the rows were evaluated at. **Invalidated by:** the simulated time moving
//! more than the group's hold time from that epoch, in either direction; then every row is evaluated again at
//! the new time. Nothing else invalidates it: rotation, panning, zoom and time inside the hold leave it alone. A
//! deeper magnitude threshold appends the rows it needs, evaluated at the stored epoch so that all rows share
//! one epoch; a shallower threshold keeps the rows (the drawing step uses its own prefix count). Outside the
//! computational interval the stored brightness keys do not apply, so the whole table is simulated.
use crate::astro::models::stars::years_since_j2000;
use crate::astro::{COMPUTATIONAL_INTERVAL, Vector3};
use crate::catalog::decode_magnitude;
use crate::model::{StarTable, SimulationCache};

/// A row the motion model cannot place (its straight line passes through the Sun); never drawn.
pub const UNPLACED: u16 = u16::MAX;

/// What `SimulationCache::update` did, for tests and timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    Unchanged,
    Extended(usize),                        // rows appended at the stored epoch
    Refreshed(usize),                       // rows evaluated at the new epoch
}

impl SimulationCache {
    pub fn epoch(&self) -> Option<f64> { self.last_update }
    pub fn len(&self) -> usize { self.directions.len() }
    pub fn is_empty(&self) -> bool { self.directions.is_empty() }
    pub fn directions(&self) -> &[Vector3] { &self.directions }
    pub fn magnitudes(&self) -> &[u16] { &self.magnitudes }

    /// Whether the rows must be evaluated again for `tt`: never refreshed, or `tt` more than `hold_seconds` away
    /// from the epoch. The boundary itself is still inside the hold.
    pub fn expired(&self, tt: f64, hold_seconds: f64) -> bool {
        self.last_update.is_none_or(|epoch| (tt - epoch).abs() > hold_seconds / 86_400.0)
    }

    /// Bring the first `rows` rows of `table` up to date for `tt`: evaluate everything at `tt` when the hold has
    /// expired, otherwise append the rows that are missing, at the stored epoch.
    pub fn update(&mut self, table: &StarTable, tt: f64, rows: usize, hold_seconds: f64) -> Refresh {
        if self.expired(tt, hold_seconds) {
            self.last_update = Some(tt);
            self.directions.clear();
            self.magnitudes.clear();
            self.evaluate_rows(table, tt, rows);
            return Refresh::Refreshed(rows);
        }
        let missing = rows.saturating_sub(self.len());
        if missing == 0 { return Refresh::Unchanged; }
        self.evaluate_rows(table, self.last_update.expect("an epoch exists once the hold did not expire"), rows);
        Refresh::Extended(missing)
    }

    fn evaluate_rows(&mut self, table: &StarTable, tt: f64, rows: usize) {
        let years = years_since_j2000(tt);
        let columns = table.columns();
        for row in self.len()..rows {
            let sample = table.motion(row).evaluate(years, decode_magnitude(columns.magnitude[row]));
            self.directions.push(sample.direction);
            self.magnitudes.push(if sample.used_singular_fallback { UNPLACED } else { sample.magnitude });
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
    use crate::model::speed_group_hold_seconds;
    use super::*;
    use crate::astro::{FrameTime, J2000, JULIAN_YEAR_DAYS};
    use crate::catalog::{SpaceMotion, StarId, load_embedded_catalog, magnitude_code};
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

    fn tt(days_from_j2000: f64) -> f64 { FrameTime::from_utc(J2000 + days_from_j2000).tt }

    #[test]
    fn the_hold_boundary_is_absolute_in_both_directions_for_every_group() {
        let tables = bsc5();
        for group in 0..SPEED_GROUP_COUNT {
            let (region, _, table) = tables.star_tables().find(|&(_, g, table)| g == group && table.len() >= 2).unwrap();
            let hold_days = speed_group_hold_seconds(group) / 86_400.0;
            let mut simulations = tables.clone();
            assert_eq!((simulations.star_regions[region].speed_groups[group].simulation.update(table, J2000, rows_wanted(table, J2000, 6.5), speed_group_hold_seconds(group))), Refresh::Refreshed(table.len()));
            let before = simulations.star_regions[region].speed_groups[group].simulation.clone();
            for inside in [0.0, 0.9 * hold_days, -0.9 * hold_days, hold_days, -hold_days] {
                assert_eq!((simulations.star_regions[region].speed_groups[group].simulation.update(table, J2000 + inside, rows_wanted(table, J2000 + inside, 6.5), speed_group_hold_seconds(group))), Refresh::Unchanged, "group {group} at {inside} days");
                assert_eq!((&simulations.star_regions[region].speed_groups[group].simulation), &before);                      // rows stay as evaluated at the epoch
            }
            for expired in [(J2000 + hold_days).next_up(), (J2000 - hold_days).next_down()] {
                let mut fresh = simulations.clone();
                assert_eq!((fresh.star_regions[region].speed_groups[group].simulation.update(table, expired, rows_wanted(table, expired, 6.5), speed_group_hold_seconds(group))), Refresh::Refreshed(table.len()), "group {group}");
                assert_eq!(fresh.star_regions[region].speed_groups[group].simulation.epoch(), Some(expired));
                assert_ne!(fresh.star_regions[region].speed_groups[group].simulation.directions(), before.directions());
            }
        }
    }

    #[test]
    fn a_zero_hold_expires_on_any_time_change_but_not_at_the_same_time() {
        let tables = bsc5();
        let (_, _, table) = tables.star_tables().find(|&(_, _, table)| table.len() >= 2).unwrap();
        let mut simulation = SimulationCache::default();
        assert_eq!(simulation.update(table, J2000, table.len(), 0.0), Refresh::Refreshed(table.len()));
        assert_eq!(simulation.update(table, J2000, table.len(), 0.0), Refresh::Unchanged);
        assert_eq!(simulation.update(table, J2000.next_up(), table.len(), 0.0), Refresh::Refreshed(table.len()));
    }

    #[test]
    fn a_deeper_threshold_appends_rows_at_the_stored_epoch_and_a_shallower_one_keeps_them() {
        let tables = bsc5();
        let (region, group, table) = tables.star_tables().find(|&(_, _, table)| table.len() >= 3).unwrap();
        let columns = table.columns();
        let (faint, bright) = (decode_magnitude(columns.magnitude[table.len() - 1]), decode_magnitude(columns.magnitude[0]));
        let mut simulations = tables.clone();
        let first = simulations.star_regions[region].speed_groups[group].simulation.update(table, J2000, rows_wanted(table, J2000, bright), speed_group_hold_seconds(group));
        let Refresh::Refreshed(rows) = first else { panic!("{first:?}") };
        assert!(rows >= 1 && rows < table.len());
        let later = J2000 + 1.0;                                                            // inside every hold
        let extended = simulations.star_regions[region].speed_groups[group].simulation.update(table, later, rows_wanted(table, later, faint), speed_group_hold_seconds(group));
        assert_eq!(extended, Refresh::Extended(table.len() - rows));
        let simulation = &simulations.star_regions[region].speed_groups[group].simulation;
        assert_eq!((simulation.epoch(), simulation.len()), (Some(J2000), table.len()));
        let years = years_since_j2000(J2000);
        for row in 0..table.len() {                                                           // every row, old and new, is the model's value at the epoch
            let sample = table.motion(row).evaluate(years, decode_magnitude(columns.magnitude[row]));
            assert_eq!((simulation.directions()[row], simulation.magnitudes()[row]), (sample.direction, sample.magnitude), "row {row}");
        }
        assert_eq!((simulations.star_regions[region].speed_groups[group].simulation.update(table, later, rows_wanted(table, later, bright), speed_group_hold_seconds(group))), Refresh::Unchanged);
        assert_eq!(simulations.star_regions[region].speed_groups[group].simulation.len(), table.len());
    }

    #[test]
    fn simulated_rows_equal_direct_evaluation_for_a_whole_region() {
        let tables = bsc5();
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.2, y: -0.7, z: 0.6 });
        let time = tt(5000.0 * JULIAN_YEAR_DAYS);
        let mut simulations = tables.clone();
        for (group, table) in tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).enumerate() {
            (simulations.star_regions[region].speed_groups[group].simulation.update(table, time, rows_wanted(table, time, 6.5), speed_group_hold_seconds(group)));
            let simulation = &simulations.star_regions[region].speed_groups[group].simulation;
            assert_eq!(simulation.len(), table.len());
            for row in 0..table.len() {
                let sample = table.motion(row).evaluate(years_since_j2000(time), decode_magnitude(table.columns().magnitude[row]));
                assert_eq!(simulation.directions()[row], sample.direction);
                assert_eq!(simulation.magnitudes()[row], sample.magnitude);
            }
        }
    }

    #[test]
    fn threshold_crossing_uses_the_brightness_key_then_the_current_magnitude() {
        let tables = single_star(Vector3 { x: 0.0, y: 0.0, z: 1.0 }, Vector3 { x: 0.0, y: 0.0, z: -0.00005 }, 5.5); // approaching along the line of sight
        let (table, group) = only_table(&tables);
        let mut simulation = SimulationCache::default();
        for (years, drawn) in [(0.0, false), (3000.0, false), (5000.0, true), (15000.0, true)] {
            let time = tt(years * JULIAN_YEAR_DAYS);
            let rows = rows_wanted(table, time, 5.0);
            assert_eq!(rows, 1, "{years}: the key says it may brighten past 5.0");
            simulation.update(table, time, rows, 0.0);
            assert_eq!(simulation.magnitudes()[0] <= magnitude_code(5.0), drawn, "{years} years");
        }
        let time = tt(20000.0 * JULIAN_YEAR_DAYS);                                           // the straight line passes the Sun: singular
        simulation.update(table, time, rows_wanted(table, time, 5.0), 0.0);
        assert_eq!(simulation.magnitudes(), [UNPLACED]);
        let sample = table.motion(0).evaluate(20000.0, 5.5);
        assert!(sample.used_singular_fallback && sample.magnitude_value() == 5.5);           // the model falls back to the catalog brightness
        assert_eq!(group, 0);                                                                // radial motion is no proper motion: the slow group
    }

    #[test]
    fn outside_the_interval_the_whole_table_is_simulated_and_judged_by_current_magnitude() {
        let tables = single_star(Vector3 { x: 1.0, y: 0.0, z: 0.0 }, Vector3 { x: -0.00004, y: 0.0, z: 0.0 }, 6.0);
        let (table, _) = only_table(&tables);
        let inside = tt(0.0);
        assert_eq!(rows_wanted(table, inside, 4.0), 0);                                         // the key keeps it out at threshold 4
        let outside = tt(24000.0 * JULIAN_YEAR_DAYS);
        assert_eq!(rows_wanted(table, outside, 4.0), 1);
        let mut simulation = SimulationCache::default();
        simulation.update(table, outside, 1, 0.0);
        assert!(simulation.magnitudes()[0] < magnitude_code(4.0));                             // much closer, so much brighter
    }

    #[test]
    fn the_constellation_table_is_simulated_whole_with_the_fast_hold() {
        let tables = bsc5();
        let mut simulations = tables.clone();
        let table = &tables.constellation_stars;
        assert_eq!(simulations.constellation_simulation.update(&tables.constellation_stars, J2000, table.len(), crate::constants::FAST_GROUP_HOLD_SECONDS), Refresh::Refreshed(table.len()));
        assert_eq!(simulations.constellation_simulation.len(), table.len());
        assert_eq!(simulations.constellation_simulation.update(&tables.constellation_stars, J2000 + 10.0, table.len(), crate::constants::FAST_GROUP_HOLD_SECONDS), Refresh::Unchanged);
        assert_eq!(simulations.constellation_simulation.update(&tables.constellation_stars, (J2000 + 10.0).next_up(), table.len(), crate::constants::FAST_GROUP_HOLD_SECONDS), Refresh::Refreshed(table.len()));
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
            let time = tt(f64::from(pass) * 400.0);                                          // every pass expires every hold
            for (region, group, table) in tables.star_tables() {
                if let Refresh::Refreshed(n) = simulations.star_regions[region].speed_groups[group].simulation.update(table, time, rows_wanted(table, time, 99.0), speed_group_hold_seconds(group)) { rows += n; }
            }
        }
        let elapsed = started.elapsed();
        eprintln!("{rows} star evaluations in {elapsed:?}: {:.0} ns per star", elapsed.as_nanos() as f64 / rows as f64);
    }
}
