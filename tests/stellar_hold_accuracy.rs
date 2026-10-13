//! Opt-in, offline comparison of held directions with fresh evaluations of the same stellar motion model.
//! These audits may fail under the current fixed holds. They never change TTLs, download data or write caches.
use astroterm::astro::{COMPUTATIONAL_INTERVAL, J2000, JULIAN_YEAR_DAYS, Vector3, models::stars::{StellarMotion, years_since_j2000}};
use astroterm::catalog::{decode_magnitude, encode_brightness_bound, magnitude_code};
use astroterm::constants::{FAST_GROUP_HOLD_SECONDS, HELD_DIRECTION_ERROR, SPEED_GROUP_COUNT};
use astroterm::model::{StarTable, StarLabels, Data, StarRow, StarRowVec, speed_group, speed_group_hold_seconds};
use astroterm::progress::Report;
use astroterm::regions::{Refresh};
use astroterm::model::{SimulationCache};

const NUMERIC_SLACK: f64 = 1e-12;                                                           // radians: tolerate f64 roundoff, not physical drift

#[derive(Clone, Copy, Debug)]
struct Worst {
    id: u32,
    region: Option<usize>,
    calculated_tt: f64,
    requested_tt: f64,
    radians: f64,
}

#[derive(Default)]
struct GroupReport {
    stars: usize,
    comparisons: usize,
    held: usize,
    refreshed: usize,
    violations: usize,
    worst: Option<Worst>,
}

struct Audit {
    groups: [GroupReport; SPEED_GROUP_COUNT + 1],                                            // the last slot is the separately held constellation table
}

impl Default for Audit {
    fn default() -> Self { Self { groups: std::array::from_fn(|_| GroupReport::default()) } }
}

impl Audit {
    fn qualified(&self) -> bool {
        self.groups.iter().any(|group| group.comparisons > 0) && self.groups.iter().all(|group| group.violations == 0)
    }

    fn report(&self, source: &str) {
        eprintln!("{source}: sampled held-direction accuracy; limit={:.6} arcsec; numerical slack={:.9} arcsec",
            arcseconds(HELD_DIRECTION_ERROR), arcseconds(NUMERIC_SLACK));
        for (index, group) in self.groups.iter().enumerate() {
            let label = if index == SPEED_GROUP_COUNT { "constellations".into() } else { format!("speed group {index}") };
            eprintln!("{label}: stars={}; comparisons={}; held={}; refreshed={}; violations={}",
                group.stars, group.comparisons, group.held, group.refreshed, group.violations);
            if let Some(worst) = group.worst {
                eprintln!("  worst={:.9} arcsec; internal star ID={}; region={:?}; calculated TT JD={:.9}; requested TT JD={:.9}",
                    arcseconds(worst.radians), worst.id, worst.region, worst.calculated_tt, worst.requested_tt);
            }
        }
        eprintln!("Sampled model-relative direction error only; not total astronomical accuracy, brightness or tile/warp error.");
    }
}

fn arcseconds(radians: f64) -> f64 { radians.to_degrees() * 3600.0 }

/// Borrow one catalog row and copy its compact fields into a one-row table, so its closest-approach epoch can be
/// audited independently through the real SimulationCache cache. Label text is irrelevant to direction holding.
fn copy_row(table: &StarTable, row: usize) -> StarTable {
    let c = table.columns();
    let mut columns = StarRowVec::with_capacity(1);
    columns.push(StarRow { u0: c.u0[row], w: c.w[row], magnitude: c.magnitude[row], brightness_key: c.brightness_key[row],
        distance: c.distance[row], id: c.id[row], name: 0, display_color: c.display_color[row] }.into());
    StarTable::new(columns, StarLabels::Names(vec![0]))
}

/// Both ends of the supported interval, representative interior dates, and dates around closest approach.
/// The audit does not extrapolate the claimed policy outside that interval.
fn sample_epochs(motion: StellarMotion, hold_days: f64) -> Vec<f64> {
    let interval = COMPUTATIONAL_INTERVAL;
    let closest = motion.closest_approach(years_since_j2000(interval.start_tt), years_since_j2000(interval.end_tt)).0;
    let closest_tt = J2000 + closest * JULIAN_YEAR_DAYS;
    let mut epochs = vec![interval.start_tt, interval.end_tt.next_down(), J2000,
        J2000 - 2000.0 * JULIAN_YEAR_DAYS, J2000 + 2000.0 * JULIAN_YEAR_DAYS, J2000 + 10000.0 * JULIAN_YEAR_DAYS,
        closest_tt - hold_days, closest_tt, closest_tt + hold_days];
    epochs.retain(|&tt| interval.contains(tt));
    epochs.sort_unstable_by(f64::total_cmp);
    epochs.dedup();
    epochs
}

/// Exercise forward and backward reuse, the exact floating-point hold edge, and its immediate neighbours.
fn audit_star(table: &StarTable, row: usize, region: Option<usize>, hold_seconds: f64, report: &mut GroupReport) {
    let table = copy_row(table, row);
    let hold_days = hold_seconds / 86_400.0;
    report.stars += 1;
    for epoch in sample_epochs(table.motion(0), hold_days) {
        for sign in [-1.0, 1.0] {
            let edge = epoch + sign * hold_days;
            let (inside, outside) = if sign > 0.0 { (edge.next_down(), edge.next_up()) } else { (edge.next_up(), edge.next_down()) };
            let dates = [epoch, epoch + sign * hold_days * 0.5, inside, edge, outside];
            let mut simulation = SimulationCache::default();
            simulation.update(&table, epoch, 1, hold_seconds);
            for requested_tt in dates.into_iter().filter(|&tt| COMPUTATIONAL_INTERVAL.contains(tt)) {
                let refreshed = simulation.update(&table, requested_tt, 1, hold_seconds);
                let actual = measure_error(&table, &simulation, requested_tt, region);
                report.comparisons += 1;
                report.held += usize::from(refreshed == Refresh::Unchanged);
                report.refreshed += usize::from(matches!(refreshed, Refresh::Refreshed(_)));
                report.violations += usize::from(actual.radians > HELD_DIRECTION_ERROR + NUMERIC_SLACK);
                if report.worst.is_none_or(|worst| actual.radians > worst.radians) { report.worst = Some(actual); }
            }
        }
    }
}

fn measure_error(table: &StarTable, simulation: &SimulationCache, requested_tt: f64, region: Option<usize>) -> Worst {
    let current = table.motion(0).evaluate(years_since_j2000(requested_tt), decode_magnitude(table.columns().magnitude[0]));
    let held = simulation.directions()[0];
    let radians = held.cross(current.direction).length().atan2(held.dot(current.direction));
    assert!(radians.is_finite(), "nonfinite direction comparison for star {} at TT JD {requested_tt}", table.id(0).0);
    Worst { id: table.id(0).0, region, calculated_tt: simulation.epoch().unwrap(), requested_tt, radians }
}

fn audit_catalog(tables: &Data, limit_per_group: usize) -> Audit {
    let mut audit = Audit::default();
    for (region, group, table) in tables.star_tables() {
        let report = &mut audit.groups[group];
        let count = table.len().min(limit_per_group.saturating_sub(report.stars));
        for row in 0..count { audit_star(table, row, Some(region), speed_group_hold_seconds(group), report); }
    }
    let table = &tables.constellation_stars;
    for row in 0..table.len().min(limit_per_group) {
        audit_star(table, row, None, FAST_GROUP_HOLD_SECONDS, &mut audit.groups[SPEED_GROUP_COUNT]);
    }
    audit
}

/// Synthetic catalog inputs: one unit reference direction, motion in units of reference distance per year.
fn synthetic_star(id: u32, velocity: Vector3, physical_distance: bool) -> StarTable {
    let pack = |v: Vector3| [v.x as f32, v.y as f32, v.z as f32];
    let motion = StellarMotion { u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 }, w: velocity, distance_pc: physical_distance.then_some(1.0) };
    let mut columns = StarRowVec::new();
    columns.push(StarRow { u0: pack(motion.u0), w: pack(motion.w), magnitude: magnitude_code(5.0),
        brightness_key: encode_brightness_bound(motion.brightest_magnitude(5.0)).0, distance: if physical_distance { 1.0 } else { 0.0 },
        id, name: 0, display_color: 0 }.into());
    StarTable::new(columns, StarLabels::Names(vec![0]))
}

fn synthetic_catalog() -> Vec<StarTable> {
    let mut tables: Vec<_> = [0.0_f64, 0.01, 0.1, 1.0, 100.0].into_iter().enumerate()
        .map(|(id, rate)| synthetic_star(id as u32, Vector3 { x: 0.0, y: (rate / 3600.0).to_radians(), z: 0.0 }, false)).collect();
    tables.push(synthetic_star(5, Vector3 { x: -0.00009, y: 1e-8, z: 0.0 }, true));             // approaches near the far end; slow by the current grouping rule
    tables.push(synthetic_star(6, Vector3 { x: -0.0001, y: 5e-7, z: 0.0 }, true));              // closest approach inside the interval, above the singular cutoff
    tables
}

#[test]
fn offline_check_detects_the_known_slow_group_counterexample() {
    let table = synthetic_star(5, Vector3 { x: -0.00009, y: 1e-8, z: 0.0 }, true);
    assert_eq!(speed_group(table.motion_bound(0)), 0);
    let epoch = J2000 + 10000.0 * JULIAN_YEAR_DAYS;
    let requested = epoch + 9.49 * JULIAN_YEAR_DAYS;
    let mut simulation = SimulationCache::default();
    simulation.update(&table, epoch, 1, speed_group_hold_seconds(0));
    assert_eq!(simulation.update(&table, requested, 1, speed_group_hold_seconds(0)), Refresh::Unchanged);
    let worst = measure_error(&table, &simulation, requested, Some(0));
    assert!((1.9..2.1).contains(&arcseconds(worst.radians)));
    assert_eq!(worst.calculated_tt, epoch);
    let mut audit = Audit::default();
    audit_star(&table, 0, Some(0), speed_group_hold_seconds(0), &mut audit.groups[0]);
    assert!(!audit.qualified());
    assert!(audit.groups[0].held > 0 && audit.groups[0].refreshed > 0);
}

#[test]
fn offline_check_accepts_stationary_and_moderate_tangential_motion_in_each_group() {
    let mut audit = Audit::default();
    assert!(!audit.qualified());                                                           // no comparisons must never count as qualification
    for table in synthetic_catalog().iter().take(4) {
        let group = speed_group(table.motion_bound(0));
        audit_star(table, 0, Some(0), speed_group_hold_seconds(group), &mut audit.groups[group]);
    }
    assert!(audit.qualified());
    assert!(audit.groups[..SPEED_GROUP_COUNT].iter().all(|group| group.comparisons > 0));
    let table = &synthetic_catalog()[3];
    audit_star(table, 0, None, FAST_GROUP_HOLD_SECONDS, &mut audit.groups[SPEED_GROUP_COUNT]);
    assert!(audit.qualified());
}

#[test]
#[ignore = "offline qualification audit; current fixed holds have known violations"]
fn audit_synthetic_holds() {
    let mut audit = Audit::default();
    for table in synthetic_catalog() {
        let group = speed_group(table.motion_bound(0));
        audit_star(&table, 0, Some(0), speed_group_hold_seconds(group), &mut audit.groups[group]);
        audit_star(&table, 0, None, FAST_GROUP_HOLD_SECONDS, &mut audit.groups[SPEED_GROUP_COUNT]);
    }
    audit.report("synthetic stars");
    assert!(audit.qualified(), "held-direction errors exceed the target; see the audit above");
}

#[test]
#[ignore = "offline qualification audit of the embedded BSC5 catalog"]
fn audit_embedded_holds() {
    let parsed = astroterm::catalog::load_embedded_catalog(&mut Report::quiet()).unwrap();
    let prepared = astroterm::sky::prepare_owned_catalog(parsed, &mut Report::quiet()).unwrap();
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut Report::quiet()).unwrap();
    let audit = audit_catalog(&tables, usize::MAX);
    audit.report("embedded BSC5, all stars");
    assert!(audit.qualified(), "held-direction errors exceed the target; see the audit above");
}

#[test]
#[ignore = "requires ASTROTERM_HOLD_AUDIT_PREPARED; reads an existing prepared catalog only, never downloads"]
fn audit_local_prepared_holds() {
    use std::io::Read;
    let path = std::path::PathBuf::from(std::env::var_os("ASTROTERM_HOLD_AUDIT_PREPARED").expect("set a local .catalog file"));
    let limit = std::env::var("ASTROTERM_HOLD_AUDIT_STARS_PER_GROUP").map_or(Ok(1000), |value| value.parse::<usize>()).expect("positive star limit");
    assert!(limit > 0, "positive star limit required");
    let mut header = [0; 48];
    std::fs::File::open(&path).unwrap().read_exact(&mut header).unwrap();
    let fingerprint = header[16..48].try_into().unwrap();                                    // audit the supplied file's identity; do not resolve or rebuild source data
    let prepared = astroterm::sky::load_cached_catalog(&path, &fingerprint, &mut Report::quiet()).unwrap(); // normal format, checksum and semantic validation still apply
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut Report::quiet()).unwrap();
    drop(prepared);
    let audit = audit_catalog(&tables, limit);
    audit.report(&format!("{}; first {limit} stars per speed group in region/table order, plus constellations", path.display()));
    assert!(audit.qualified(), "held-direction errors exceed the target; see the audit above");
}
