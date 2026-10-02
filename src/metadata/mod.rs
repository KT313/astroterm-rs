//! What the metadata panel shows: local date and time, zodiac sign, lunar phase, observer location, elapsed simulation
//! time and speed, and view settings. How the fields are laid out is up to the renderer.

mod local_time;

use chrono::{Datelike, Timelike};

use crate::astro::{
    DegreesMinutesSeconds, ElapsedTime, MoonPhase, Observer, SimulationClock, ZodiacSign, azimuth_to_compass,
    julian_date_to_utc,
};
use crate::projection::{ProjectionKind, View, ViewCenter};

use local_time::{LocalTime, convert_to_local_time};

/// One line of metadata, e.g. label "Lunar Phase" and value "Waxing Crescent".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataField {
    pub label: String,
    pub value: String,
}

/// The metadata fields for the simulation time `julian_date`. `unicode` allows symbols such as the zodiac sign's.
pub fn collect_metadata_fields(
    julian_date: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
) -> Vec<MetadataField> {
    let local_time = julian_date_to_utc(julian_date).map(convert_to_local_time);
    format_metadata_fields(local_time, julian_date, clock, moon_phase, observer, view, unicode)
}

/// The fields, given the simulation time in the local timezone (`None` if it can't be represented).
fn format_metadata_fields(
    local_time: Option<LocalTime>,
    julian_date: f64,
    clock: &SimulationClock,
    moon_phase: MoonPhase,
    observer: &Observer,
    view: &View,
    unicode: bool,
) -> Vec<MetadataField> {
    let mut fields = Vec::with_capacity(10);

    // calendar: local date and time, and the zodiac sign of that date
    match local_time {
        Some(LocalTime { time, zone }) => {
            let date = format!(
                "{:02}-{:02}-{:04} {:02}:{:02}",
                time.day(),
                time.month(),
                time.year(),
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
    let elapsed = ElapsedTime::from_days(julian_date - clock.start_julian_date());
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
    fields
}

fn create_field(label: impl Into<String>, value: impl Into<String>) -> MetadataField {
    MetadataField {
        label: label.into(),
        value: value.into(),
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
            collect_metadata_fields(2460678.25, &clock, MoonPhase::Full, &Observer::default(), view, false).len()
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
    fn speeds_are_shown_without_floating_point_noise() {
        assert_eq!(format_speed(0.7 * 10.0), "7");
        assert_eq!(format_speed(0.1 * 3.0), "0.3");
        assert_eq!(format_speed(-100.0), "-100");
        assert_eq!(format_speed(2.5), "2.5");
        assert_eq!(format_speed(0.001), "0.001");
        assert_eq!(format_speed(123456789.0), "123456789");
        assert_eq!(format_speed(0.0), "0");
    }
}
