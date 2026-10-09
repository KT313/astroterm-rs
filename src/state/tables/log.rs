//! Render original table owners as Markdown, with bounded first/last-row previews.
use crate::constants::{MAX_TABLE_TEXT_CHARS, TABLE_PREVIEW_EDGE_ROWS};
use super::{Tables, Table};
use crate::cache::Group;
use crate::rows::short_type_name;
use crate::state::ApplicationState;
use crate::timing::format_bytes;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;


impl ApplicationState {
    /// Append a Markdown dump to a file, or print it to stdout when no path is given.
    /// Use a file while the terminal session is active; stdout belongs to the rendered scene.
    /// Each section contains one heading and one Markdown table per original data table.
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

    /// Write headings, memory details and Markdown previews without modifying the inspected state.
    pub fn write_tables(&self, out: &mut dyn Write, section: Option<&str>) -> io::Result<()> {
        writeln!(out, "\n## {}\n", markdown_text(section.unwrap_or("Working data")))?;
        let mut result = Ok(());
        self.visit_tables("", &mut |path, table, group| {
            if result.is_ok() { result = write_table(out, path, table, group.map(|g| ttl_text(&self.config.cache, g))); }
        });
        result
    }
}

fn write_table(out: &mut dyn Write, path: &str, table: &dyn Table, ttl: Option<String>) -> io::Result<()> {
    writeln!(out, "### {}\n", markdown_text(path))?;
    let bytes = table.bytes();
    writeln!(out, "**Shape:** `{:?}` · **Used:** {} · **Reserved:** {}\n", table.shape(), format_bytes(bytes.used), format_bytes(bytes.reserved))?;
    if let Some(ttl) = ttl { writeln!(out, "**Cache policy:** {}\n", markdown_text(&ttl))?; }
    if let Some(note) = table.note() { writeln!(out, "**Notes:** {}\n", markdown_text(&truncate(&note)))?; }
    for note in super::labels::column_notes(path) { writeln!(out, "**Columns:** {}\n", markdown_text(note))?; }
    write_preview(out, path, table)
}

/// Reuse policy of the group: disabled, dependency-only, or a maximum age in seconds.
fn ttl_text(config: &crate::cache::CacheConfig, group: Group) -> String {
    if !config.allows(group) { return "ttl=disabled".to_string(); }
    match config.age_seconds(group) {
        age if age > 0.0 => format!("ttl={age} s"),
        _ => "ttl=dependencies".to_string(),
    }
}

fn write_preview(out: &mut dyn Write, path: &str, table: &dyn Table) -> io::Result<()> {
    let mut columns = table.columns();
    if !columns.is_empty() { super::labels::label_columns(path, &mut columns); }
    super::labels::label_color_columns(&mut columns);
    let prepared = table.preview(); // one ordering/preparation for the entire table
    let width = columns.len().max(prepared.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0));
    if width == 0 { return writeln!(out, "*No rows to preview.*\n"); }
    let headers: Vec<_> = (0..width).map(|i| match columns.get(i) {
        Some(c) if !c.name.is_empty() => format!("{}: {}", c.name, short_type_name(c.dtype)),
        Some(c) => short_type_name(c.dtype),
        None => format!("Value {}", i + 1),
    }).collect();
    write_markdown_row(out, "Row", &headers, width)?;
    writeln!(out, "| ---: |{}", " --- |".repeat(width))?;
    for (position, (index, cells)) in prepared.iter().enumerate() {
        if table.rows() > 2 * TABLE_PREVIEW_EDGE_ROWS && position == TABLE_PREVIEW_EDGE_ROWS {
            write_markdown_row(out, "…", &[format!("{} rows omitted", table.rows() - 2 * TABLE_PREVIEW_EDGE_ROWS)], width)?;
        }
        write_markdown_row(out, &index.to_string(), cells, width)?;
    }
    writeln!(out)?;
    if prepared.is_empty() { writeln!(out, "*No rows to preview.*\n")?; }
    Ok(())
}

/// Every physical line has the same number of cells; wide tables stay intact in Markdown renderers.
fn write_markdown_row(out: &mut dyn Write, row: &str, cells: &[String], width: usize) -> io::Result<()> {
    write!(out, "| {} |", markdown_text(row))?;
    for i in 0..width { write!(out, " {} |", markdown_text(cells.get(i).map_or("", String::as_str)))?; }
    writeln!(out)
}

/// Escape data as literal Markdown text, including pipes, markup and embedded line breaks.
fn markdown_text(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\\' | '|' | '`' | '*' | '_' | '[' | ']' | '#' | '~' | '!' => { escaped.push('\\'); escaped.push(ch); }
            c if c.is_control() => escaped.extend(c.escape_default()),
            c => escaped.push(c),
        }
    }
    escaped
}
fn truncate(text: &str) -> String {
    match text.char_indices().nth(MAX_TABLE_TEXT_CHARS) {
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

    fn empty_state() -> ApplicationState {
        let config = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
        ApplicationState::new(config, StepTimes::with_trace(true))
    }
    fn dump(state: &ApplicationState, section: Option<&str>) -> String {
        let mut out = Vec::new();
        state.write_tables(&mut out, section).unwrap();
        String::from_utf8(out).unwrap()
    }
    fn paths(text: &str) -> Vec<String> {
        text.lines().filter_map(|l| l.strip_prefix("### ")).map(|l| l.replace("\\_", "_")).collect()
    }
    fn section<'a>(text: &'a str, path: &str) -> &'a str {
        text.split_once(&format!("### {}\n", markdown_text(path))).unwrap().1.split("\n### ").next().unwrap()
    }
    fn data_rows(text: &str) -> usize {
        text.lines().filter(|line| line.strip_prefix("| ").is_some_and(|tail| tail.chars().next().is_some_and(|c| c.is_ascii_digit()))).count()
    }

    /// Every stage lists its tables even before the catalog is loaded or a frame has run.
    #[test]
    fn empty_state_lists_every_stage_without_panicking() {
        let text = dump(&empty_state(), Some("empty"));
        assert!(text.starts_with("\n## empty\n"));
        let listed = paths(&text);
        assert!(!listed.iter().any(|p| p == "cache.observation.apparent" || p == "cache.observation.correction_sources" || p == "cache.observation.correction_revision" || p == "cache.observation.eligible" || p == "cache.observation.corrections" || p == "cache.simulation.stars.motion" || p == "cache.selection.candidates" || p == "cache.selection.selected"));
        for expected in [
            "persistent.catalog.stars", "persistent.catalog.star_exceptions", "persistent.catalog.names.boundaries", "persistent.catalog.names.ascii_alternatives",
            "persistent.catalog.stars.precise_motions", "persistent.catalog.grid.offsets",
            "persistent.catalog.grid.coarse_caps", "persistent.catalog.grid.fine_caps",
            "persistent.catalog.endpoint_indices", "preparation", "persistent.catalog.names",
            "persistent.catalog.constellations",
            "cache.sky.stars", "cache.sky.planets", "cache.sky.moon", "cache.sky.candidate_indices",
            "cache.simulation.solar_system.planets.sun", "cache.simulation.solar_system.planets.earth", "cache.simulation.solar_system.planets.neptune",
            "cache.simulation.solar_system.moon", "cache.simulation.solar_system.orientation",
            "cache.simulation.solar_system.planet_work", "cache.simulation.solar_system.moon_work", "cache.simulation.solar_system.orientation_work", "cache.simulation.solar_system.group",
            "cache.simulation.stars.prepared_classes", "cache.simulation.stars.stellar_scratch", "cache.selection.region",
            "cache.selection.requested_sources", "cache.selection.statistics", "cache.selection.selection_revision", "cache.selection.working",
            "cache.simulation.stars.regions", "cache.simulation.stars.region_output_work", "cache.observer.bodies", "cache.observation.relative",
            "cache.observation.layout_sources", "cache.observation.published", "cache.observation.use_refraction",
            "cache.observation.body_apparent", "cache.observation.horizontal", "cache.observation.refracted",
            "cache.observer.observer", "cache.observer.light_time", "cache.observation.illumination",
            "cache.projection.stale_slots", "cache.projection.regional_cell_work", "cache.projection.regional_direction_work", "cache.projection.regional_order_work",
            "cache.projection.star_candidate", "cache.projection.order_candidate", "cache.projection.draw_order_scratch",
            "cache.projection.stars", "cache.projection.stars.key", "cache.projection.order", "cache.projection.order.key",
            "cache.projection.bodies", "cache.projection.bodies.key", "cache.projection.constellations",
            "cache.projection.constellations.key", "cache.projection.horizon",
            "timings.steps", "timings.trace.steps",
        ] {
            assert!(listed.iter().any(|p| p == expected), "missing table {expected}\n{text}");
        }
        assert!(!listed.iter().any(|p| p.starts_with("cache.rendering")), "nothing is rendered yet");
        for path in &listed {
            let body = section(&text, path);
            assert!(body.contains("**Shape:**") && body.contains("**Used:**") && body.contains("**Reserved:**"), "{path}: {body}");
        }
        assert!(text.contains("ttl=") && text.contains("invalid=true"));
        assert!(section(&text, "cache.sky.moon").contains("**Shape:** `[1]`"));
        assert!(text.contains("| Row | phase: MoonPhase | illumination: MoonIllumination | direction: Vector3 |"));
        let work = section(&text, "cache.simulation.stars.region_output_work");
        assert!(work.contains("*No rows to preview.*") && work.contains(&markdown_text("direction_j2000")) && work.contains("**Shape:** `[0]`"));

    }

    /// Loaded catalog plus one headless observation and projection: columns, caches and previews are shown.
    #[test]
    fn loaded_state_shows_columns_caches_and_truncated_previews() {
        use crate::astro::{J2000, Observer};
        use crate::model::{FrameTime, ProjectionViewport, SkyRegion, View};
        use crate::state::Caches;
        let mut state = empty_state();
        state.replace_catalog(crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog().unwrap()).unwrap());
        let count = state.persistent.catalog.stars.len();
        {
            let Caches { sky, simulation, observer: observer_cache, selection, observation, projection, .. } = &mut state.cache;
            let time = FrameTime::from_utc(J2000);
            crate::sky::update_solar_system(&mut simulation.solar_system, time, &[], &mut state.timings).unwrap();
            let mut site = crate::sky::prepare_cached_observer(observer_cache, &simulation.solar_system, time, Observer::default()).unwrap();
            crate::sky::prepare_cached_light_time(observer_cache, &mut simulation.solar_system, &mut site, &mut state.timings).unwrap();
            crate::sky::prepare_cached_bodies(observer_cache, &simulation.solar_system, &site, &mut state.timings).unwrap();
        crate::sky::select_cached_stars(selection, &sky.catalog, &site, 5.0, true, SkyRegion::All, &mut state.timings);
        crate::sky::simulate_stars(&mut simulation.stars, selection.stars(), time.tt, &mut state.timings);
        crate::sky::observe_cached_sky(observation, simulation.stars.results(selection.stars()), observer_cache.bodies(&site), &site, 5.0, true, sky, &mut state.timings);
            crate::projection::project_cached_sky(projection, sky, &View::default(), ProjectionViewport { width: 80, height: 40 }, time.tt, &mut state.timings);
        }

        let text = dump(&state, None);
        let stars = section(&text, "persistent.catalog.stars");
        assert!(stars.contains(&format!("**Shape:** `[{count}, 8]`")));
        assert!(!text.contains("persistent.catalog.stars.u0"));
        assert!(stars.contains(&format!("{} rows omitted", count - 2 * TABLE_PREVIEW_EDGE_ROWS)));
        assert!(text.contains("ttl=864000 s") && text.contains("ttl=dependencies") && text.contains("invalid=false"));
        for path in paths(&text) { assert!(data_rows(section(&text, &path)) <= 2 * TABLE_PREVIEW_EDGE_ROWS); }

        let star_header = stars.lines().find(|l| l.starts_with("| Row |")).unwrap();
        let mut columns = crate::model::StarRow::columns();
        super::super::labels::label_columns("persistent.catalog.stars", &mut columns);
        for column in columns {
            let name = format!("{}: {}", column.name, short_type_name(column.dtype));
            assert!(star_header.contains(&markdown_text(&name)), "missing column: {star_header}");
        }
        assert_eq!(data_rows(stars), 20);
        for row in stars.lines().filter(|l| l.starts_with('|')) { assert_eq!(row.matches('|').count(), 10); }
        let sky = section(&text, "cache.sky.stars");
        assert!(sky.contains("| Row | catalog\\_row\\_index: usize | passes\\_brightness\\_filter: bool | current\\_magnitude: f64 | direction: Vector3 |"));
        assert!(section(&text, "cache.simulation.solar_system.planets.earth").contains(&markdown_text("sampled_state: BodyState")));
        let trace = section(&text, "timings.trace.steps");
        assert!(trace.contains("parent\\_step\\_index: Option&lt;usize&gt;"));
        assert!(section(&text, "cache.simulation.stars.regions").contains("| Row | simulation\\_region\\_id: usize | calculated\\_at\\_tt\\_jd: Option&lt;f64&gt; |"));

    }

    /// A file path appends; two calls give two sections in one file.
    #[test]
    fn log_data_appends_sections_to_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("tables.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let state = empty_state();
        state.log_data(Some(&path), Some("first")).unwrap();
        state.log_data(Some(&path), Some("second")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("## first").count(), 1);
        assert_eq!(text.matches("## second").count(), 1);
        assert_eq!(paths(&text).iter().filter(|p| p.as_str() == "persistent.catalog.stars").count(), 2);
        assert!(text.find("## first").unwrap() < text.find("## second").unwrap());
    }

    #[test]
    fn truncation_keeps_whole_characters() {
        let long: String = "é".repeat(MAX_TABLE_TEXT_CHARS + 5);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_TABLE_TEXT_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short"), "short");
    }
    #[test]
    fn preview_prepares_once_and_keeps_all_markdown_columns_and_unknown_sizes() {
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
        let mut output = Vec::new();
        write_table(&mut output, "fixture", &fixture, None).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(fixture.0.get(), 1);
        assert_eq!(text.lines().filter(|l| l.contains("field12=")).count(), 20);
        assert_eq!(data_rows(&text), 20);
        assert!(text.contains("80 rows omitted"));
        assert!(text.contains("**Used:** unknown · **Reserved:** unknown"));
        for line in text.lines().filter(|l| l.starts_with('|')) { assert_eq!(line.matches('|').count(), 15); }

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

    #[test]
    fn markdown_escapes_cell_markup_and_cannot_create_extra_rows() {
        let input = r"a|b \ [x] `t` <tag> & *bold*";
        assert_eq!(markdown_text(input), r"a\|b \\ \[x\] \`t\` &lt;tag&gt; &amp; \*bold\*");
        let mut output = Vec::new();
        write_markdown_row(&mut output, "0", &["one|two\n<script>\rnext".into(), "é".into()], 2).unwrap();
        let row = String::from_utf8(output).unwrap();
        assert_eq!(row, "| 0 | one\\|two\\n&lt;script&gt;\\rnext | é |\n");
        assert_eq!(row.lines().count(), 1);
        assert!(!row.contains("<script>"));
    }

}
