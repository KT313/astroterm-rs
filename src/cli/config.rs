//! Validated settings, converted to the units used internally (radians, Julian dates).

use crate::catalog::datasets::Dataset;
use std::fmt;

use crate::astro::{
    Observer, compass_point_to_azimuth, current_julian_date, datetime_to_julian_date, parse_utc_datetime,
};
use crate::catalog::{City, find_city, suggest_cities};
use crate::projection::{ProjectionKind, View, ViewCenter};
use crate::scene::RenderOptions;
use crate::terminal::TerminalSettings;

use super::Arguments;

/// Everything the application needs to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub debug_singleframe: bool,
    pub cache: crate::cache::CacheConfig,
    /// Raster text size and spacing relative to terminal cells; ignored by native text renderers.
    pub text_scale: f64,
    pub renderer: crate::terminal::RendererKind,
    pub graphics_protocol: crate::terminal::GraphicsProtocol,
    pub simulation: SimulationSettings,
    /// The view to start with, and to reset to.
    pub view: View,
    pub render: RenderOptions,
    pub terminal: TerminalSettings,
    /// Frames per second.
    pub fps: u32,
    /// Star dataset to load instead of the embedded catalog.
    pub dataset: Option<Dataset>,
}

/// What is simulated: where, from when, how fast, and with which corrections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationSettings {
    pub observer: Observer,
    pub start_julian_date: f64,
    /// Simulated days per real day.
    pub speed: f64,
    /// Lift objects by atmospheric refraction.
    pub refraction: bool,
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
    if !arguments.text_scale.is_finite() || !(0.25..=4.0).contains(&arguments.text_scale) {
        return Err(ConfigError("Text scale must be finite and between 0.25 and 4".into()));
    }
    // reject non-finite values even when a city overrides the supplied coordinates
    for (value, name) in [
        (arguments.speed, "Speed"),
        (f64::from(arguments.threshold), "Magnitude threshold"),
        (f64::from(arguments.label_threshold), "Label threshold"),
        (arguments.latitude, "Latitude"),
        (arguments.longitude, "Longitude"),
    ] {
        if !value.is_finite() {
            return Err(ConfigError(format!("{name} must be finite")));
        }
    }

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
    let default_fps = match arguments.renderer {
        crate::terminal::RendererKind::Chars => 24,
        crate::terminal::RendererKind::Pixels => 12,
    };
    let fps = u32::try_from(arguments.fps.unwrap_or(default_fps))
        .ok()
        .filter(|&fps| fps >= 1);
    let fps = fps.ok_or_else(|| ConfigError("FPS must be greater than or equal to 1".into()))?;
    let aspect_ratio = validate_aspect_ratio(arguments.aspect_ratio)?;
    let view = build_view(&arguments)?;

    let render = RenderOptions {
        unicode: arguments.unicode,
        braille: arguments.braille,
        color: arguments.color,
        constellations: arguments.constellations,
        grid: arguments.grid,
        magnitude_threshold: f64::from(arguments.threshold),
        label_threshold: f64::from(arguments.label_threshold),
        dynamic_names: !arguments.disable_dynamic_names,
    };
    let simulation = SimulationSettings {
        observer,
        start_julian_date,
        speed: arguments.speed,
        refraction: arguments.refraction,
    };
    let terminal = TerminalSettings {
        aspect_ratio,
        metadata_panel: arguments.metadata || arguments.debug_frametimes,
        quit_on_any_key: arguments.quit_on_any,
        frame_times: arguments.debug_frametimes,
    };
    Ok(Config {
        debug_singleframe: arguments.debug_singleframe,
        cache: crate::cache::CacheConfig::load(arguments.cache_config.as_deref(), arguments.disable_cache)
            .map_err(|e| ConfigError(e.to_string()))?,
        text_scale: arguments.text_scale,
        renderer: arguments.renderer,
        graphics_protocol: arguments.graphics_protocol,
        simulation,
        view,
        render,
        terminal,
        fps,
        dataset: arguments
            .dataset
            .as_ref()
            .map(|p| Dataset::parse(p.as_os_str()))
            .transpose()
            .map_err(ConfigError)?,
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
    let city = find_city(cities, name).ok_or_else(|| {
        let suggestions = suggest_cities(cities, name);
        let mut message = format!("Could not find city \"{name}\"");
        if !suggestions.is_empty() {
            message.push_str(&format!(". Did you mean {}?", suggestions.join(", ")));
        }
        ConfigError(message)
    })?;
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
    fn misspelled_city_gets_suggestions() {
        assert!(error_from(&["-i", "Tokio"]).contains("Tokyo"));
        assert!(error_from(&["-i", "zzzzzzzzzz"]).ends_with("\"zzzzzzzzzz\""));
    }

    #[test]
    fn defaults_match_the_original() {
        let config = config_from(&["-d", "2000-01-01T12:00:00"]).unwrap();
        assert_eq!(
            config.simulation.observer,
            Observer {
                latitude: 0.0,
                longitude: 0.0
            }
        );
        assert_eq!(config.simulation.start_julian_date, 2451545.0);
        assert_eq!(config.view, View::default());
        assert_eq!(
            (config.render.magnitude_threshold, config.render.label_threshold),
            (5.0, 0.25)
        );
        assert_eq!(
            (config.fps, config.simulation.speed, config.terminal.aspect_ratio),
            (24, 1.0, None)
        );
        assert!(!config.terminal.metadata_panel && !config.simulation.refraction && !config.terminal.quit_on_any_key);
        assert!(!config.terminal.frame_times && config.render.dynamic_names);
    }

    #[test]
    fn converts_degrees_to_radians() {
        let config = config_from(&["-a", "-33.87", "-o", "151.21", "-F", "NNW", "-T", "20"]).unwrap();
        assert!((config.simulation.observer.latitude + 33.87 * PI / 180.0).abs() < 1e-12);
        assert!((config.simulation.observer.longitude - 151.21 * PI / 180.0).abs() < 1e-12);
        let ViewCenter::Facing { azimuth, tilt } = config.view.center else {
            panic!("facing view expected")
        };
        assert!((azimuth - 337.5 * PI / 180.0).abs() < 1e-12 && (tilt - 20.0 * PI / 180.0).abs() < 1e-12);
    }

    #[test]
    fn city_overrides_latitude_and_longitude() {
        let config = config_from(&["-a", "10", "-i", "rio de janeiro", "-m", "-R"]).unwrap();
        assert!((config.simulation.observer.latitude - (-22.90642_f64).to_radians()).abs() < 1e-12);
        assert!((config.simulation.observer.longitude - (-43.18223_f64).to_radians()).abs() < 1e-12);
        assert!(config.terminal.metadata_panel && config.simulation.refraction);
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
        assert!(error_from(&["-i", "Atlantis"]).starts_with("Could not find city \"Atlantis\""));
    }

    #[test]
    fn dataset_path_is_passed_through() {
        assert_eq!(config_from(&[]).unwrap().dataset, None);
        let config = config_from(&["--dataset", "datasets/athyg_40.csv.gz"]).unwrap();
        assert_eq!(config.dataset, Some(Dataset::Path("datasets/athyg_40.csv.gz".into())));
    }

    #[test]
    fn rejects_non_finite_values_before_starting_the_terminal() {
        for flag in [
            "--speed",
            "--threshold",
            "--label-thresh",
            "--latitude",
            "--longitude",
            "--aspect-ratio",
        ] {
            for value in ["NaN", "inf", "-inf"] {
                let argument = format!("{flag}={value}");
                assert!(config_from(&[&argument]).is_err(), "{argument}");
            }
        }
        assert!(config_from(&["-i", "Tokyo", "--latitude=NaN"]).is_err());
        for speed in ["0", "-1000", "1e12"] {
            assert!(config_from(&["--speed", speed]).is_ok());
        }
    }

    #[test]
    fn accepts_signed_extended_calendar_years() {
        for date in ["-7974-01-01T00:00:00", "+12026-12-31T00:00:00"] {
            assert!(config_from(&["-d", date]).is_ok());
        }
    }

    #[test]
    fn dynamic_names_can_be_disabled() {
        assert!(!config_from(&["--disable-dynamic-names"]).unwrap().render.dynamic_names);
    }

    #[test]
    fn renderer_frame_rate_defaults_preserve_explicit_overrides() {
        assert_eq!(config_from(&[]).unwrap().fps, 24);
        assert_eq!(config_from(&["--renderer", "pixels"]).unwrap().fps, 12);
        assert_eq!(config_from(&["--renderer", "pixels", "--fps", "24"]).unwrap().fps, 24);
        assert!(config_from(&["--renderer", "pixels", "--fps", "0"]).is_err());
    }

    #[test]
    fn raster_text_scale_defaults_and_invalid_values() {
        assert_eq!(config_from(&["--renderer", "pixels"]).unwrap().text_scale, 0.85);
        for value in ["0.25", "0.7", "1", "1.5", "4"] {
            assert_eq!(
                config_from(&["--text-scale", value]).unwrap().text_scale,
                value.parse::<f64>().unwrap()
            );
        }
        for value in ["NaN", "inf", "-inf", "0", "-1", "0.24", "4.01"] {
            assert!(config_from(&[&format!("--text-scale={value}")]).is_err(), "{value}");
        }
    }

    #[test]
    fn debug_frametimes_turns_on_the_metadata_panel() {
        let config = config_from(&["--debug-frametimes"]).unwrap();
        assert!(config.terminal.frame_times && config.terminal.metadata_panel);
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
