//! Four-stage pipeline: immutable catalog and independently cached geometric simulation -> observer-relative sky
//! -> camera projection -> rendering. Astronomy families live in astro::models; no camera enters observation.
//! Common states use f64 J2000 equatorial AU/AU-day, with a barycentric origin. Earth is the only real
//! anchor, with a WGS84 sea-level site. Observation applies light-time, exact parallax and aberration before
//! horizon rotation and optional refraction; camera projection never changes these values.
//! Start with catalog/pipeline.rs for loading/preparation, simulation/pipeline.rs for model refreshes,
//! and observation/pipeline.rs for the apparent-position sequence. Import operations through this root.

mod catalog;
mod illumination;
mod observation;
mod positions;
mod simulation;

pub use catalog::{prepare_catalog, prepare_owned_catalog, prepare_star, create_sky_from_catalog, select_grid};
pub use catalog::{catalog_fingerprint, cache_path, load_sky_catalog, load_sky_catalog_with_times, write_cached_catalog, load_cached_catalog};
pub(crate) use catalog::{select_region, select_brightness, count_region_stars};
pub(crate) use observation::LIGHT_SPEED_AU_DAY;
pub use illumination::{compute_moon_illumination, name_moon_phase};
pub use observation::{prepare_observation_catalog, prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time, observe_cached_sky};
pub use observation::{compose_observer_state, observe_sky, observe_sky_candidates, prepare_light_time_samples, prepare_observation, prepare_observer};
pub use positions::{refract_sky_positions, update_sky_positions};
pub use simulation::{update_simulation, evaluate_body, evaluate_orientation};
