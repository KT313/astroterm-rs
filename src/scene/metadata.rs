//! The metadata panel: local date and time, zodiac sign, lunar phase, observer location, elapsed simulation time and
//! speed, and view settings.

use chrono::{Datelike, Timelike};

use crate::astro::{
    DegreesMinutesSeconds, ElapsedTime, MoonPhase, Observer, SimulationClock, ZodiacSign, azimuth_to_compass,
    julian_date_to_utc,
};
use crate::canvas::Canvas;
use crate::projection::{ProjectionKind, View, ViewCenter};

use super::local_time::{LocalTime, convert_to_local_time};

/// Columns of the metadata panel, enough for the longest line (elapsed time).
const PANEL_WIDTH: usize = 45;

/// Tab stops are every 8 columns, as in curses.
const TAB_WIDTH: usize = 8;

/// Values start at this column at the earliest, so they line up for short labels too.
const VALUE_COLUMN: usize = 16;

/// Draw the metadata for the simulation time `julian_date` onto the panel canvas, resizing it to fit the lines.
pub fn draw_metadata(
    canvas: &mut Canvas,
    julian_date: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
) {
    let local_time = julian_date_to_utc(julian_date).map(convert_to_local_time);
    let lines = format_metadata_lines(local_time, julian_date, clock, moon_phase, observer, view, unicode);

    canvas.resize(lines.len(), PANEL_WIDTH);
    canvas.clear();
    for (row, line) in lines.iter().enumerate() {
        canvas.put_str_truncated(row as i32, 0, line, None);
    }
}

/// The panel's lines. `local_time` is the simulation time in the local timezone, if representable.
fn format_metadata_lines(
    local_time: Option<LocalTime>,
    julian_date: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
) -> Vec<String> {
    let mut lines = Vec::with_capacity(10);

    // calendar: local date and time, and the zodiac sign of that date
    match local_time {
        Some(LocalTime { time, zone }) => {
            let date_label = format!("Date ({zone}): ");
            let date = format!(
                "{:02}-{:02}-{:04} {:02}:{:02}",
                time.day(),
                time.month(),
                time.year(),
                time.hour(),
                time.minute()
            );
            lines.push(format_field(&date_label, &date));

            let zodiac = ZodiacSign::from_date(time.month(), time.day());
            let sign = if unicode {
                format!("{} {}", zodiac.name(), zodiac.symbol())
            } else {
                zodiac.name().to_string()
            };
            lines.push(format_field("Zodiac: ", &sign));
        }
        None => {
            lines.push(format_field("Date: ", "out of range"));
            lines.push(format_field("Zodiac: ", "-"));
        }
    }

    // sky and observer
    lines.push(format_field("Lunar Phase: ", moon_phase.name()));
    lines.push(format_field(
        "Latitude: ",
        &DegreesMinutesSeconds::from_degrees(observer.latitude.to_degrees()).to_string(),
    ));
    lines.push(format_field(
        "Longitude: ",
        &DegreesMinutesSeconds::from_degrees(observer.longitude.to_degrees()).to_string(),
    ));

    // time elapsed in the simulation, and how fast it runs
    let elapsed = ElapsedTime::from_days(julian_date - clock.start_julian_date());
    let year_label = if elapsed.years == 1 { " year" } else { "years" };
    let day_label = if elapsed.days == 1 { " day" } else { "days" };
    let elapsed_text = format!(
        "{:03} {year_label}, {:03} {day_label}, {:02}:{:02}:{:02}",
        elapsed.years, elapsed.days, elapsed.hours, elapsed.minutes, elapsed.seconds
    );
    lines.push(format_field("Elapsed Time: ", &elapsed_text));
    let pause_note = if clock.is_paused() { " (paused)" } else { "" };
    lines.push(format_field("Speed: ", &format!("{}x{pause_note}", clock.speed())));

    // view settings that differ from the default
    if let ViewCenter::Facing { azimuth, tilt } = view.center {
        let degrees = azimuth.to_degrees();
        let facing = format!(
            "{degrees:.1}° ({}), tilt {:.1}°",
            azimuth_to_compass(degrees),
            tilt.to_degrees()
        );
        lines.push(format_field("Facing: ", &facing));
    }
    if view.fov_degrees != 180.0 {
        lines.push(format_field("Field of View: ", &format!("{:.1}°", view.fov_degrees)));
    }
    if view.projection == ProjectionKind::Equidistant {
        lines.push(format_field("Projection: ", "equidistant"));
    }
    lines
}

/// `label` followed by a tab (expanded to the next tab stop, at least the value column) and `value`.
fn format_field(label: &str, value: &str) -> String {
    let width = label.chars().count();
    let tab_stop = ((width / TAB_WIDTH + 1) * TAB_WIDTH).max(VALUE_COLUMN);
    format!("{label}{}{value}", " ".repeat(tab_stop - width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_fits_its_lines() {
        let clock = SimulationClock::start(2460678.25, 1.0);
        let (mut canvas, phase, observer) = (Canvas::new(0, 0), MoonPhase::Full, Observer::default());
        draw_metadata(
            &mut canvas,
            2460678.25,
            &clock,
            phase,
            &observer,
            &View::default(),
            false,
        );
        assert_eq!((canvas.height(), canvas.width()), (7, PANEL_WIDTH));

        let facing = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            ..View::default()
        };
        draw_metadata(&mut canvas, 2460678.25, &clock, phase, &observer, &facing, false);
        assert_eq!(canvas.height(), 8);
    }

    #[test]
    fn formats_all_lines() {
        let time = chrono::DateTime::parse_from_rfc3339("2025-01-02T19:30:00+01:00").unwrap();
        let local_time = Some(LocalTime {
            time,
            zone: "CET".to_string(),
        });
        let observer = Observer {
            latitude: (-33.87_f64).to_radians(),
            longitude: 151.21_f64.to_radians(),
        };
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 334_f64.to_radians(),
                tilt: 20_f64.to_radians(),
            },
            projection: ProjectionKind::Equidistant,
            fov_degrees: 120.0,
        };
        let start = 2460678.25;
        let elapsed_days = 366.25 + 1.0 + 2.0 / 24.0 + 3.0 / 1440.0 + 4.5 / 86400.0;
        let mut clock = SimulationClock::start(start, -100.0);
        clock.toggle_pause();
        let lines = format_metadata_lines(
            local_time,
            start + elapsed_days,
            &clock,
            MoonPhase::WaxingCrescent,
            &observer,
            &view,
            true,
        );

        assert_eq!(lines[0], "Date (CET):     02-01-2025 19:30");
        assert_eq!(lines[1], "Zodiac:         Capricorn ♑");
        assert_eq!(lines[2], "Lunar Phase:    Waxing Crescent");
        assert_eq!(lines[3], "Latitude:       -33° 52' 12.00\"");
        assert_eq!(lines[4], "Longitude:      151° 12' 36.00\"");
        assert_eq!(lines[5], "Elapsed Time:   001  year, 002 days, 02:03:04");
        assert_eq!(lines[6], "Speed:          -100x (paused)");
        assert_eq!(lines[7], "Facing:         334.0° (NNW), tilt 20.0°");
        assert_eq!(lines[8], "Field of View:  120.0°");
        assert_eq!(lines[9], "Projection:     equidistant");
        assert!(lines.iter().all(|line| line.chars().count() <= PANEL_WIDTH));
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(format_field("Zodiac: ", "Leo"), "Zodiac:         Leo");
        assert_eq!(format_field("Speed: ", "1x"), "Speed:          1x");
        assert_eq!(format_field("Date (+05:30): ", "x"), "Date (+05:30):  x");
        assert_eq!(format_field("A label of sixteen", "x"), "A label of sixteen      x");
    }
}
