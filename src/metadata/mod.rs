//! What the metadata panel shows: local date and time, zodiac sign, lunar phase, observer location, elapsed simulation
//! time and speed, and view settings. How the fields are laid out is up to the renderer.

mod local_time;

use chrono::{Datelike, Timelike};

use crate::astro::{
    DegreesMinutesSeconds, ElapsedTime, MoonPhase, Observer, SimulationClock, ZodiacSign, azimuth_to_compass,
    julian_date_to_utc,
};
use crate::model::projection::{ProjectionKind, View, ViewCenter};
use crate::timing::StepTime;

use local_time::LocalTime;
use crate::model::metadata::ObserverTimeZone;
pub use local_time::{convert_observer_time, resolve_observer_timezone};

use crate::model::metadata::MetadataField;

/// The metadata fields for the simulation time `julian_date_utc`. `unicode` allows symbols such as the zodiac sign's.
pub fn collect_metadata_fields(
    julian_date_utc: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
    time_zone: &ObserverTimeZone,
) -> Vec<MetadataField> {
    let mut fields = Vec::with_capacity(10);
    fill_metadata_fields(&mut fields, julian_date_utc, clock, moon_phase, observer, view, unicode, time_zone);
    fields
}

/// Replace metadata entries while retaining the destination vector's capacity. Field strings are rebuilt.
#[allow(clippy::too_many_arguments)]
pub fn fill_metadata_fields(
    fields: &mut Vec<MetadataField>, julian_date_utc: f64, clock: &SimulationClock, moon_phase: MoonPhase,
    observer: &Observer, view: &View, unicode: bool, time_zone: &ObserverTimeZone,
) {
    let local_time = julian_date_to_utc(julian_date_utc).map(|utc| convert_observer_time(time_zone, utc));
    fields.clear();
    append_metadata_fields(fields, local_time, julian_date_utc, clock, moon_phase, observer, view, unicode);
}

/// Append fields for the local simulation time (`None` if it cannot be represented).
#[allow(clippy::too_many_arguments)]
fn append_metadata_fields(
    fields: &mut Vec<MetadataField>,
    local_time: Option<LocalTime>,
    julian_date_utc: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
) {
    // calendar: local date and time, and the zodiac sign of that date
    match local_time {
        Some(LocalTime { time, zone }) => {
            let date = format!(
                "{:02}-{:02}-{} {:02}:{:02}",
                time.day(),
                time.month(),
                format_calendar_year(time.year()),
                time.hour(),
                time.minute()
            );
            fields.push(create_field(format!("Date ({zone})"), date));

            let zodiac = ZodiacSign::from_date(time.month(), time.day());
            let sign = if unicode {
                format!("{} {}", zodiac.name(), zodiac.symbol())
            } else {
                zodiac.name().to_string()
            };
            fields.push(create_field("Zodiac", sign));
        }
        None => {
            fields.push(create_field("Date", "out of range"));
            fields.push(create_field("Zodiac", "-"));
        }
    }

    // sky and observer
    fields.push(create_field("Lunar Phase", moon_phase.name()));
    let latitude = DegreesMinutesSeconds::from_degrees(observer.latitude.to_degrees());
    fields.push(create_field("Latitude", latitude.to_string()));
    let longitude = DegreesMinutesSeconds::from_degrees(observer.longitude.to_degrees());
    fields.push(create_field("Longitude", longitude.to_string()));

    // time elapsed in the simulation, and how fast it runs
    let elapsed = ElapsedTime::from_days(julian_date_utc - clock.start_julian_date());
    let year_label = if elapsed.years == 1 { " year" } else { "years" };
    let day_label = if elapsed.days == 1 { " day" } else { "days" };
    let elapsed_text = format!(
        "{:03} {year_label}, {:03} {day_label}, {:02}:{:02}:{:02}",
        elapsed.years, elapsed.days, elapsed.hours, elapsed.minutes, elapsed.seconds
    );
    fields.push(create_field("Elapsed Time", elapsed_text));
    let pause_note = if clock.is_paused() { " (paused)" } else { "" };
    fields.push(create_field(
        "Speed",
        format!("{}x{pause_note}", format_speed(clock.speed())),
    ));

    // view settings that differ from the default
    if let ViewCenter::Facing { azimuth, tilt } = view.center {
        let degrees = azimuth.to_degrees();
        let facing = format!(
            "{degrees:.1}° ({}), tilt {:.1}°",
            azimuth_to_compass(degrees),
            tilt.to_degrees()
        );
        fields.push(create_field("Facing", facing));
    }
    if view.fov_degrees != 180.0 {
        fields.push(create_field("Field of View", format!("{:.1}°", view.fov_degrees)));
    }
    if view.projection == ProjectionKind::Equidistant {
        fields.push(create_field("Projection", "equidistant"));
    }
}

/// Fields for the smoothed frame step durations: their total, then each step (indented), in milliseconds.
pub fn format_step_time_fields(steps: &[StepTime]) -> Vec<MetadataField> {
    let mut fields = Vec::with_capacity(steps.len() + 1);
    append_step_time_fields(&mut fields, steps);
    fields
}

/// Append timing entries directly, including their total; existing metadata stays before them.
pub fn append_step_time_fields(fields: &mut Vec<MetadataField>, steps: &[StepTime]) {
    let format_ms = |seconds: f64| format!("{:.3} ms", seconds * 1000.0);
    let total = steps
        .iter()
        .filter(|step| step.depth == 0)
        .map(|step| step.average_seconds)
        .sum();
    fields.push(create_field("Frame Time", format_ms(total)));
    for step in steps {
        fields.push(create_field(
            format!("{}{}", "  ".repeat(step.depth + 1), step.name),
            format_ms(step.average_seconds),
        ));
    }
}

fn create_field(label: impl Into<String>, value: impl Into<String>) -> MetadataField {
    MetadataField {
        label: label.into(),
        value: value.into(),
    }
}

/// Astronomical year, signed outside the four-digit range so year zero and BC dates remain unambiguous.
fn format_calendar_year(year: i32) -> String {
    if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else {
        format!("{year:+05}")
    }
}

/// A speed multiplier rounded to 6 significant digits without trailing zeros, so repeated speed changes don't show
/// floating-point noise (e.g. `7` rather than `7.000000000000001`).
fn format_speed(speed: f64) -> String {
    if speed == 0.0 || !speed.is_finite() {
        return speed.to_string();
    }
    let decimals = (5 - speed.abs().log10().floor() as i32).max(0) as usize;
    let rounded = format!("{speed:.decimals$}");
    if rounded.contains('.') {
        rounded.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_metadata_fields(local_time: Option<LocalTime>, date: f64, clock: &SimulationClock, phase: MoonPhase, observer: &Observer, view: &View, unicode: bool) -> Vec<MetadataField> {
        let mut fields = Vec::new();
        append_metadata_fields(&mut fields, local_time, date, clock, phase, observer, view, unicode);
        fields
    }

    #[test]
    fn refilling_metadata_and_timings_retains_vector_capacity_without_stale_entries() {
        let observer = Observer::default();
        let zone = resolve_observer_timezone(&observer);
        let clock = SimulationClock::start(crate::astro::J2000, 0.0);
        let mut fields = Vec::with_capacity(64);
        let mut timing_fields = Vec::with_capacity(32);
        let field_pointer = fields.as_ptr();
        let timing_pointer = timing_fields.as_ptr();
        let capacities = (fields.capacity(), timing_fields.capacity());
        let steps = [StepTime { name: "Draw", depth: 0, average_seconds: 0.001 }];
        for view in [View { fov_degrees: 90.0, ..View::default() }, View::default()] {
            fill_metadata_fields(&mut fields, crate::astro::J2000, &clock, MoonPhase::Full, &observer, &view, true, &zone);
            let expected = collect_metadata_fields(crate::astro::J2000, &clock, MoonPhase::Full, &observer, &view, true, &zone);
            assert_eq!(fields, expected);
            assert_eq!(fields.as_ptr(), field_pointer);
            timing_fields.clear();
            append_step_time_fields(&mut timing_fields, &steps);
            assert_eq!(timing_fields.as_ptr(), timing_pointer);
            assert_eq!(timing_fields, format_step_time_fields(&steps));
            fields.append(&mut timing_fields);
            assert_eq!(fields[expected.len()].label, "Frame Time");
            assert!(timing_fields.is_empty());
            assert_eq!(timing_fields.as_ptr(), timing_pointer);
        }
        assert_eq!((fields.capacity(), timing_fields.capacity()), capacities);
    }

    #[test]
    fn metadata_panel_snapshot_uses_an_explicit_zone_and_frozen_clock() {
        let time = chrono::DateTime::parse_from_rfc3339("2025-03-01T20:00:00+09:00").unwrap();
        let local_time = Some(LocalTime {
            time,
            zone: "JST".to_string(),
        });
        let observer = Observer {
            latitude: 35.69_f64.to_radians(),
            longitude: 139.69_f64.to_radians(),
        };
        let mut clock = SimulationClock::start(2460736.9583333335, 0.0);
        clock.toggle_pause();
        let fields = format_metadata_fields(
            local_time,
            clock.start_julian_date(),
            &clock,
            MoonPhase::WaxingCrescent,
            &observer,
            &View::default(),
            true,
        );
        let mut panel = crate::canvas::Canvas::new(0, 0);
        crate::scene::draw_metadata_panel(&mut panel, &fields);
        let occupancy = (0..panel.height())
            .map(|row| {
                panel
                    .row(row)
                    .iter()
                    .map(|cell| if cell.is_continuation() { '>' } else { '.' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let snapshot = format!(
            "{}\ncontinuations:\n{occupancy}\ncolors: all default",
            panel.to_lines().join("\n")
        );
        insta::with_settings!({snapshot_path => concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots")}, {
            insta::assert_snapshot!("metadata_panel", snapshot);
        });
        assert!((0..panel.height()).all(|row| { panel.row(row).iter().all(|cell| cell.color.is_none()) }));
    }

    #[test]
    fn collects_all_fields() {
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
        let fields = format_metadata_fields(
            local_time,
            start + elapsed_days,
            &clock,
            MoonPhase::WaxingCrescent,
            &observer,
            &view,
            true,
        );

        let pairs: Vec<(&str, &str)> = fields
            .iter()
            .map(|field| (field.label.as_str(), field.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("Date (CET)", "02-01-2025 19:30"),
                ("Zodiac", "Capricorn ♑"),
                ("Lunar Phase", "Waxing Crescent"),
                ("Latitude", "-33° 52' 12.00\""),
                ("Longitude", "151° 12' 36.00\""),
                ("Elapsed Time", "001  year, 002 days, 02:03:04"),
                ("Speed", "-100x (paused)"),
                ("Facing", "334.0° (NNW), tilt 20.0°"),
                ("Field of View", "120.0°"),
                ("Projection", "equidistant"),
            ]
        );
    }

    #[test]
    fn default_view_settings_are_left_out() {
        let clock = SimulationClock::start(2460678.25, 1.0);
        let fields = |view: &View| {
            collect_metadata_fields(
                2460678.25,
                &clock,
                MoonPhase::Full,
                &Observer::default(),
                view,
                false,
                &resolve_observer_timezone(&Observer::default()),
            )
            .len()
        };
        assert_eq!(fields(&View::default()), 7);
        let facing = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            ..View::default()
        };
        assert_eq!(fields(&facing), 8);
    }

    #[test]
    fn step_times_are_listed_below_their_total() {
        let steps = [
            StepTime {
                name: "Stars",
                depth: 0,
                average_seconds: 0.000512,
            },
            StepTime {
                name: "Draw",
                depth: 0,
                average_seconds: 0.0012,
            },
        ];
        let fields = format_step_time_fields(&steps);
        let pairs: Vec<(&str, &str)> = fields
            .iter()
            .map(|field| (field.label.as_str(), field.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("Frame Time", "1.712 ms"),
                ("  Stars", "0.512 ms"),
                ("  Draw", "1.200 ms")
            ]
        );
    }

    #[test]
    fn nested_substeps_are_not_counted_twice_in_the_total() {
        let steps = [
            StepTime {
                name: "Observation",
                depth: 0,
                average_seconds: 0.0015,
            },
            StepTime {
                name: "Stars",
                depth: 1,
                average_seconds: 0.001,
            },
            StepTime {
                name: "Draw",
                depth: 0,
                average_seconds: 0.0005,
            },
        ];
        let fields = format_step_time_fields(&steps);
        assert_eq!(fields[0].value, "2.000 ms");
        assert_eq!(fields[2].label, "    Stars");
    }

    #[test]
    fn speeds_are_shown_without_floating_point_noise() {
        assert_eq!(format_speed(0.7 * 10.0), "7");
        assert_eq!(format_speed(0.1 * 3.0), "0.3");
        assert_eq!(format_speed(-100.0), "-100");
        assert_eq!(format_speed(2.5), "2.5");
        assert_eq!(format_speed(0.001), "0.001");
        assert_eq!(format_speed(123456789.0), "123456789");
        assert_eq!(format_speed(0.0), "0");
    }

    #[test]
    fn extended_calendar_years_keep_their_sign() {
        for (year, text) in [
            (-7974, "-7974"),
            (-1, "-0001"),
            (0, "0000"),
            (2026, "2026"),
            (12026, "+12026"),
        ] {
            assert_eq!(format_calendar_year(year), text);
        }
    }
}
