//! Print every table the state holds: path, shape, bytes, cache metadata, the first and last rows.
use super::{Tables, EDGE_ROWS};
use crate::cache::Group;
use crate::rows::short_type_name;
use crate::state::ApplicationState;
use crate::timing::format_bytes;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

/// Longest row or note text before it is cut.
const MAX_TEXT: usize = 160;

impl ApplicationState {
    /// Dump every data table (catalog columns, caches, buffers) for memory debugging.
    ///
    /// `path`: append to that file (created if missing); `None` prints to stdout. `section`: a header line
    /// written first, so one log file can hold several dumps ("after frame 3", "before resize", ...).
    ///
    /// Call it with a path from inside the frame loop: the terminal session owns stdout and the alternate screen
    /// is active, so printing there garbles the display. `None` is for before the terminal opens, for tests, or
    /// when stdout is redirected.
    pub fn log_data(&self, path: Option<&Path>, section: Option<&str>) -> io::Result<()> {
        match path {
            Some(path) => {
                let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                self.write_tables(&mut file, section)?;
                file.flush()
            }
            None => {
                let mut out = io::stdout().lock();
                self.write_tables(&mut out, section)?;
                out.flush()
            }
        }
    }

    /// The formatting behind `log_data`, for any writer.
    pub fn write_tables(&self, out: &mut dyn Write, section: Option<&str>) -> io::Result<()> {
        if let Some(section) = section { writeln!(out, "== {section} ==")?; }

        // collect first so the path column can be aligned
        let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
        self.visit_tables("", &mut |path, table, group| {
            let header = describe_header(table, group.map(|g| ttl_text(&self.config.cache, g)));
            entries.push((path.to_string(), header, preview_rows(table)));
        });
        let width = entries.iter().map(|(path, ..)| path.len()).max().unwrap_or(0);

        for (path, header, rows) in &entries {
            writeln!(out, "{path:<width$}  {header}")?;
            for row in rows { writeln!(out, "{row}")?; }
        }
        Ok(())
    }
}

/// `shape=[..]  used=..  reserved=..  | ttl=..  <note>`
fn describe_header(table: &dyn super::Table, ttl: Option<String>) -> String {
    let bytes = table.bytes();
    let mut text = format!(
        "shape={:?}  used={}  reserved={}",
        table.shape(), format_bytes(bytes.used), format_bytes(bytes.reserved),
    );
    let extras: Vec<String> = ttl.into_iter().chain(table.note().map(|n| truncate(&n))).collect();
    if !extras.is_empty() { text.push_str("  | "); text.push_str(&extras.join("  ")); }
    text
}
/// Reuse policy of the group: disabled, dependency-only (no age limit) or a maximum age in seconds.
fn ttl_text(config: &crate::cache::CacheConfig, group: Group) -> String {
    if !config.allows(group) { return "ttl=disabled".to_string(); }
    match config.age_seconds(group) {
        age if age > 0.0 => format!("ttl={age} s"),
        _ => "ttl=dependencies".to_string(),
    }
}

/// The `columns:` line, then all rows when there are few; otherwise the first and last `EDGE_ROWS` with an
/// "omitted" line between.
fn preview_rows(table: &dyn super::Table) -> Vec<String> {
    let mut rows = Vec::new();
    let columns: Vec<_> = table.columns().iter().map(|c| {
        let dtype = short_type_name(c.dtype);
        if c.name.is_empty() { dtype } else { format!("{}: {dtype}", c.name) }
    }).collect();
    if !columns.is_empty() { append_cells(&mut rows, "  columns: ", &columns); }
    let count = table.rows();
    let prepared = table.preview(); // exactly one preparation, including any map ordering
    for (position, (index, cells)) in prepared.iter().enumerate() {
        if count > 2 * EDGE_ROWS && position == EDGE_ROWS {
            rows.push(format!("  ... {} rows omitted ...", count - 2 * EDGE_ROWS));
        }
        append_cells(&mut rows, &format!("  [{index}] "), cells);
    }
    rows
}

/// Wrap between complete cells. A wide table keeps every column instead of losing its tail to truncation.
fn append_cells(lines: &mut Vec<String>, prefix: &str, cells: &[String]) {
    let mut line = prefix.to_string();
    let mut count = 0;
    for cell in cells {
        if count > 0 && line.chars().count() + 3 + cell.chars().count() > MAX_TEXT {
            lines.push(line);
            line = "    | ".to_string();
            count = 0;
        }
        if count > 0 { line.push_str(" | "); }
        line.push_str(cell);
        count += 1;
    }
    lines.push(line);
}
fn truncate(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Arguments, build_config};
    use crate::timing::StepTimes;
    use crate::rows::Row;
    use clap::Parser;
    use std::sync::Arc;

    fn empty_state() -> ApplicationState {
        let config = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
        ApplicationState::new(config, StepTimes::with_trace(true))
    }
    fn dump(state: &ApplicationState, section: Option<&str>) -> String {
        let mut out = Vec::new();
        state.write_tables(&mut out, section).unwrap();
        String::from_utf8(out).unwrap()
    }
    fn paths(text: &str) -> Vec<&str> {
        text.lines().filter(|l| !l.starts_with(' ') && !l.starts_with("==")).map(|l| l.split_whitespace().next().unwrap()).collect()
    }

    /// Every stage lists its tables even before the catalog is loaded or a frame has run.
    #[test]
    fn empty_state_lists_every_stage_without_panicking() {
        let text = dump(&empty_state(), Some("empty"));
        assert!(text.starts_with("== empty ==\n"));
        let listed = paths(&text);
        for expected in [
            "persistent.catalog.stars", "persistent.catalog.stars.name_table",
            "persistent.catalog.stars.precise_motions", "persistent.catalog.grid.offsets",
            "persistent.catalog.grid.coarse_caps", "persistent.catalog.grid.fine_caps",
            "persistent.catalog.endpoint_indices", "persistent.catalog.always_checked", "persistent.catalog.names",
            "persistent.catalog.constellations",
            "cache.sky.stars", "cache.sky.planets", "cache.sky.moon", "cache.sky.candidate_indices",
            "cache.sky.constellations", "cache.sky.names",
            "cache.simulation.planets", "cache.simulation.moon", "cache.simulation.orientation",
            "cache.observation.prepared_classes", "cache.observation.stellar_scratch", "cache.observation.region",
            "cache.observation.candidates", "cache.observation.selected", "cache.observation.working",
            "cache.observation.stellar", "cache.observation.motion", "cache.observation.eligible",
            "cache.observation.corrections", "cache.observation.bodies", "cache.observation.relative",
            "cache.observation.apparent", "cache.observation.horizontal", "cache.observation.refracted",
            "cache.observation.observer", "cache.observation.light_time", "cache.observation.illumination",
            "cache.projection.prepared_figures", "cache.projection.prepared_endpoints",
            "cache.projection.star_candidate", "cache.projection.order_candidate", "cache.projection.draw_order_scratch",
            "cache.projection.stars", "cache.projection.stars.key", "cache.projection.order", "cache.projection.order.key",
            "cache.projection.bodies", "cache.projection.bodies.key", "cache.projection.constellations",
            "cache.projection.constellations.key", "cache.projection.horizon",
            "timings.steps", "timings.trace.steps",
        ] {
            assert!(listed.contains(&expected), "missing table {expected}\n{text}");
        }
        assert!(!listed.iter().any(|p| p.starts_with("cache.rendering")), "nothing is rendered yet");
        for line in text.lines().filter(|l| !l.starts_with(' ') && !l.starts_with("==")) {
            assert!(line.contains("shape=") && line.contains("used=") && line.contains("reserved="), "bad header: {line}");
        }
        assert!(text.contains("cache.observation.motion") && text.contains("ttl=") && text.contains("invalid=true"));
        assert!(text.contains("cache.sky.moon") && text.contains("shape=[1]"));
        assert!(text.contains("  columns: phase: MoonPhase | illumination: MoonIllumination | position: Vector3"));
        let motion_lines: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("cache.observation.motion ")).take(2).collect();
        assert!(!motion_lines[1].contains("columns:"), "an empty cache has no value to describe: {motion_lines:?}");
    }

    /// Loaded catalog plus one headless observation and projection: columns, caches and previews are shown.
    #[test]
    fn loaded_state_shows_columns_caches_and_truncated_previews() {
        use crate::astro::{J2000, Observer};
        use crate::model::{FrameTime, ProjectionViewport, SkyRegion, View};
        use crate::state::Caches;
        let mut state = empty_state();
        state.replace_catalog(Arc::new(crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog().unwrap())));
        let count = state.persistent.catalog.stars.len();
        {
            let Caches { sky, simulation, observation, projection, .. } = &mut state.cache;
            let time = FrameTime::from_utc(J2000);
            crate::sky::update_simulation(simulation, time, &[], &mut state.timings).unwrap();
            let mut site = crate::sky::prepare_cached_observer(observation, simulation, time, Observer::default()).unwrap();
            crate::sky::prepare_cached_light_time(observation, simulation, &mut site, &mut state.timings).unwrap();
            crate::sky::observe_cached_sky(observation, simulation, &site, 5.0, true, SkyRegion::All, sky, &mut state.timings).unwrap();
            crate::projection::project_cached_sky(projection, sky, &View::default(), ProjectionViewport { width: 80, height: 40 }, time.tt, &mut state.timings);
        }

        let text = dump(&state, None);
        let stars = text.lines().find(|l| l.starts_with("persistent.catalog.stars ")).expect("full star table listed");
        assert!(stars.contains(&format!("shape=[{count}, 13]")), "{stars}");
        assert!(!text.contains("persistent.catalog.stars.u0"));
        assert!(text.contains(&format!("... {} rows omitted ...", count - 2 * EDGE_ROWS)));
        assert!(text.contains("ttl=360 s"), "stellar state carries its maximum age");
        assert!(text.contains("ttl=dependencies"), "dependency-only groups say so");
        assert!(text.contains("invalid=false"), "a refreshed cache is listed as valid");
        let longest = text.lines().max_by_key(|l| l.chars().count()).unwrap();
        assert!(longest.chars().count() <= 60 + 2 * MAX_TEXT, "rows and notes are truncated: {longest}"); // path, header, cache note and a cut note

        // continuation lines keep all columns; at most twenty source rows are printed
        let mut rows_in_table = 0;
        for line in text.lines() {
            if line.starts_with("  [") { rows_in_table += 1; assert!(rows_in_table <= 2 * EDGE_ROWS, "too many rows: {line}"); } else if !line.starts_with(' ') { rows_in_table = 0; }
        }

        // column names come from the row_columns! lines, types from the compiler
        let after = |path: &str| -> Vec<&str> {
            text.lines().skip_while(|l| !l.starts_with(&format!("{path} "))).skip(1).take_while(|l| l.starts_with(' ')).collect()
        };
        assert_eq!(after("cache.sky.stars")[0], "  columns: source_index: usize | drawable: bool | magnitude: f64 | position: Vector3");
        let star_rows = after("persistent.catalog.stars");
        let header = star_rows.iter().take_while(|line| !line.starts_with("  [")).copied().collect::<Vec<_>>().join(" ");
        for column in crate::model::StarRow::columns() {
            assert!(header.contains(&format!("{}: {}", column.name, short_type_name(column.dtype))), "missing column: {header}");
        }
        assert_eq!(after("cache.simulation.planets")[0], "  columns: epoch: f64 | half_span: f64 | value: [BodyState; 9]");
        let trace = after("timings.trace.steps")[0];
        assert!(trace.contains("parent: Option<usize>") && !trace.contains("memory_"), "{trace}");
        assert!(after("cache.observation.stellar")[0].starts_with("  columns: catalog_index: usize | direction: Vector3 |"));
    }

    /// A file path appends; two calls give two sections in one file.
    #[test]
    fn log_data_appends_sections_to_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("tables.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let state = empty_state();
        state.log_data(Some(&path), Some("first")).unwrap();
        state.log_data(Some(&path), Some("second")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("== first ==").count(), 1);
        assert_eq!(text.matches("== second ==").count(), 1);
        assert_eq!(paths(&text).iter().filter(|&&p| p == "persistent.catalog.stars").count(), 2);
        assert!(text.find("== first ==").unwrap() < text.find("== second ==").unwrap());
    }

    #[test]
    fn truncation_keeps_whole_characters() {
        let long: String = "é".repeat(MAX_TEXT + 5);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_TEXT + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short"), "short");
    }
    #[test]
    fn preview_prepares_once_wraps_all_columns_and_keeps_unknown_sizes() {
        use super::super::{Table, TableBytes};
        use std::cell::Cell;
        struct Fixture(Cell<usize>);
        impl Table for Fixture {
            fn shape(&self) -> Vec<usize> { vec![100, 13] }
            fn rows(&self) -> usize { 100 }
            fn bytes(&self) -> TableBytes { TableBytes { used: None, reserved: None } }
            fn preview(&self) -> Vec<(usize, Vec<String>)> {
                self.0.set(self.0.get() + 1);
                super::super::preview_indices(100).map(|i| (i, (0..13).map(|j| format!("field{j}={}", "x".repeat(50))).collect())).collect()
            }
        }
        let fixture = Fixture(Cell::new(0));
        let lines = preview_rows(&fixture);
        assert_eq!(fixture.0.get(), 1);
        assert_eq!(lines.iter().filter(|l| l.contains("field12=")).count(), 20);
        assert_eq!(lines.iter().filter(|l| l.starts_with("  [")).count(), 20);
        assert!(lines.iter().any(|l| l.contains("80 rows omitted")));
        assert!(describe_header(&fixture, None).contains("used=unknown  reserved=unknown"));
    }

    #[test]
    fn writer_errors_propagate_without_mutating_state() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::ErrorKind::BrokenPipe.into()) }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let state = empty_state();
        let before = dump(&state, None);
        assert_eq!(state.write_tables(&mut Broken, None).unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(dump(&state, None), before);
    }

}
