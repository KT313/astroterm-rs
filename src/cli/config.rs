//! Validated settings, converted to the units used internally (radians, Julian dates).

use std::fmt;

use crate::astro::{
    Observer, compass_point_to_azimuth, current_julian_date, datetime_to_julian_date, parse_utc_datetime,
};
use crate::catalog::{City, find_city};
use crate::projection::{ProjectionKind, View, ViewCenter};
use crate::scene::RenderOptions;

use super::Arguments;

/// Everything the application needs to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub observer: Observer,
    pub start_julian_date: f64,
    pub view: View,
    pub render: RenderOptions,
    pub metadata: bool,
    /// Lift objects by atmospheric refraction.
    pub refraction: bool,
    pub fps: u32,
    pub speed: f64,
    /// Cell height / width; detected from the terminal when `None`.
    pub aspect_ratio: Option<f64>,
    pub quit_on_any_key: bool,
}

/// An invalid argument, with a message for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Validate the arguments and convert them to a [`Config`]. `cities` resolves `--city`.
pub fn build_config(arguments: Arguments, cities: &[City]) -> Result<Config, ConfigError> {
    // observer location (a city overrides latitude and longitude) and time
    let observer = match &arguments.city {
        Some(name) => locate_city(cities, name)?,
        None => validate_observer(arguments.latitude, arguments.longitude)?,
    };
    let start_julian_date = match &arguments.datetime {
        Some(text) => parse_start_julian_date(text)?,
        None => current_julian_date(),
    };

    // frame rate and view
    let fps = u32::try_from(arguments.fps).ok().filter(|&fps| fps >= 1);
    let fps = fps.ok_or_else(|| ConfigError("FPS must be greater than or equal to 1".into()))?;
    let aspect_ratio = validate_aspect_ratio(arguments.aspect_ratio)?;
    let view = build_view(&arguments)?;

    let render = RenderOptions {
        unicode: arguments.unicode,
        braille: arguments.braille,
        color: arguments.color,
        constellations: arguments.constellations,
        grid: arguments.grid,
        magnitude_threshold: arguments.threshold,
        label_threshold: arguments.label_threshold,
    };
    Ok(Config {
        observer,
        start_julian_date,
        view,
        render,
        metadata: arguments.metadata,
        refraction: arguments.refraction,
        fps,
        speed: arguments.speed,
        aspect_ratio,
        quit_on_any_key: arguments.quit_on_any,
    })
}

/// Parse an azimuth in degrees [0, 360] or a 16-point compass direction such as "NNW" (case insensitive), in degrees.
pub fn parse_azimuth(text: &str) -> Option<f64> {
    compass_point_to_azimuth(text).or_else(|| {
        text.parse::<f64>()
            .ok()
            .filter(|degrees| (0.0..=360.0).contains(degrees))
    })
}

fn validate_observer(latitude: f64, longitude: f64) -> Result<Observer, ConfigError> {
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(ConfigError("Latitude out of range [-90°, 90°]".into()));
    }
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(ConfigError("Longitude out of range [-180°, 180°]".into()));
    }
    Ok(Observer {
        latitude: latitude.to_radians(),
        longitude: longitude.to_radians(),
    })
}

fn locate_city(cities: &[City], name: &str) -> Result<Observer, ConfigError> {
    let city = find_city(cities, name).ok_or_else(|| ConfigError(format!("Could not find city \"{name}\"")))?;
    Ok(Observer {
        latitude: city.latitude.to_radians(),
        longitude: city.longitude.to_radians(),
    })
}

fn parse_start_julian_date(text: &str) -> Result<f64, ConfigError> {
    let datetime = parse_utc_datetime(text).ok_or_else(|| {
        ConfigError(format!(
            "Unable to parse datetime string '{text}'\nDatetimes must be in form <yyyy-mm-ddThh:mm:ss>"
        ))
    })?;
    Ok(datetime_to_julian_date(&datetime))
}

fn validate_aspect_ratio(aspect_ratio: Option<f64>) -> Result<Option<f64>, ConfigError> {
    match aspect_ratio {
        Some(ratio) if !(ratio > 0.0 && ratio.is_finite()) => {
            Err(ConfigError("Aspect ratio must be greater than 0".into()))
        }
        _ => Ok(aspect_ratio),
    }
}

/// The view from --facing, --tilt, --fov and --equidistant.
fn build_view(arguments: &Arguments) -> Result<View, ConfigError> {
    // center: overhead, or facing a direction with an optional tilt
    let center = match &arguments.facing {
        Some(text) => {
            let azimuth = parse_azimuth(text).ok_or_else(|| {
                ConfigError(format!(
                    "Invalid facing direction \"{text}\". Use degrees [0°, 360°] or a compass direction such as NNW"
                ))
            })?;
            let tilt = arguments.tilt.unwrap_or(0.0);
            if !(-90.0..=90.0).contains(&tilt) {
                return Err(ConfigError("Tilt out of range [-90°, 90°]".into()));
            }
            ViewCenter::Facing {
                azimuth: azimuth.to_radians(),
                tilt: tilt.to_radians(),
            }
        }
        None if arguments.tilt.is_some() => return Err(ConfigError("--tilt requires --facing".into())),
        None => ViewCenter::Zenith,
    };

    // projection and field of view; 360° puts the point behind on the edge, which only the equidistant one can do
    let projection = if arguments.equidistant {
        ProjectionKind::Equidistant
    } else {
        ProjectionKind::Stereographic
    };
    let fov_degrees = arguments.fov.unwrap_or(180.0);
    let max_fov = projection.max_fov_degrees();
    if !(fov_degrees > 0.0 && fov_degrees <= max_fov) {
        return Err(ConfigError(
            "Field of view out of range (0°, 359°], or (0°, 360°] with --equidistant".into(),
        ));
    }
    Ok(View {
        center,
        projection,
        fov_degrees,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use clap::Parser;

    use super::*;
    use crate::catalog::load_embedded_cities;

    fn config_from(args: &[&str]) -> Result<Config, ConfigError> {
        let arguments = Arguments::try_parse_from(std::iter::once("astroterm").chain(args.iter().copied()));
        let cities = load_embedded_cities().expect("embedded cities load");
        build_config(arguments.expect("arguments parse"), &cities)
    }

    fn error_from(args: &[&str]) -> String {
        config_from(args).expect_err("invalid arguments").to_string()
    }

    #[test]
    fn defaults_match_the_original() {
        let config = config_from(&["-d", "2000-01-01T12:00:00"]).unwrap();
        assert_eq!(
            config.observer,
            Observer {
                latitude: 0.0,
                longitude: 0.0
            }
        );
        assert_eq!(config.start_julian_date, 2451545.0);
        assert_eq!(config.view, View::default());
        assert_eq!(
            (config.render.magnitude_threshold, config.render.label_threshold),
            (5.0, 0.25)
        );
        assert_eq!((config.fps, config.speed, config.aspect_ratio), (24, 1.0, None));
        assert!(!config.metadata && !config.refraction);
    }

    #[test]
    fn converts_degrees_to_radians() {
        let config = config_from(&["-a", "-33.87", "-o", "151.21", "-F", "NNW", "-T", "20"]).unwrap();
        assert!((config.observer.latitude + 33.87 * PI / 180.0).abs() < 1e-12);
        assert!((config.observer.longitude - 151.21 * PI / 180.0).abs() < 1e-12);
        let ViewCenter::Facing { azimuth, tilt } = config.view.center else {
            panic!("facing view expected")
        };
        assert!((azimuth - 337.5 * PI / 180.0).abs() < 1e-12 && (tilt - 20.0 * PI / 180.0).abs() < 1e-12);
    }

    #[test]
    fn city_overrides_latitude_and_longitude() {
        let config = config_from(&["-a", "10", "-i", "rio de janeiro", "-m", "-R"]).unwrap();
        assert!((config.observer.latitude - (-22.90642_f64).to_radians()).abs() < 1e-12);
        assert!((config.observer.longitude - (-43.18223_f64).to_radians()).abs() < 1e-12);
        assert!(config.metadata && config.refraction);
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert_eq!(error_from(&["-a", "91"]), "Latitude out of range [-90°, 90°]");
        assert_eq!(error_from(&["-o", "-181"]), "Longitude out of range [-180°, 180°]");
        assert_eq!(error_from(&["-f", "0"]), "FPS must be greater than or equal to 1");
        assert_eq!(error_from(&["-T", "10"]), "--tilt requires --facing");
        assert_eq!(error_from(&["-F", "N", "-T", "NaN"]), "Tilt out of range [-90°, 90°]");
        assert!(error_from(&["-F", "up"]).starts_with("Invalid facing direction \"up\""));
        assert!(error_from(&["-z", "360"]).starts_with("Field of view out of range"));
        assert!(error_from(&["-d", "2025-01-01"]).starts_with("Unable to parse datetime string '2025-01-01'"));
        assert_eq!(error_from(&["-r", "0"]), "Aspect ratio must be greater than 0");
        assert_eq!(error_from(&["-i", "Atlantis"]), "Could not find city \"Atlantis\"");
    }

    #[test]
    fn equidistant_allows_full_field_of_view() {
        let config = config_from(&["-e", "-z", "360", "-F", "N"]).unwrap();
        assert_eq!(
            (config.view.projection, config.view.fov_degrees),
            (ProjectionKind::Equidistant, 360.0)
        );
    }

    #[test]
    fn parse_azimuth_accepts_degrees_and_compass_points() {
        for (text, expected) in [
            ("334", 334.0),
            ("0", 0.0),
            ("360", 360.0),
            ("12.5", 12.5),
            ("NNW", 337.5),
            ("nnw", 337.5),
        ] {
            assert_eq!(parse_azimuth(text), Some(expected), "{text}");
        }
        for text in ["400", "-5", "foo", "12abc", "NNWW", "nan", ""] {
            assert_eq!(parse_azimuth(text), None, "{text}");
        }
    }
}
