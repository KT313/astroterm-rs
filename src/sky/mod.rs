//! Ordered sky processing: solar-system simulation → observer preparation → star selection → stellar simulation
//! → observation corrections. Projection and rendering follow in their own modules. All domain APIs are exported
//! here; implementation folders remain private. The binary calls each stage explicitly; pipeline.rs contains
//! headless compatibility coordinators over the same separated domains.
//! Intrinsic stellar directions are fixed-axis J2000 unit vectors. Solar-system states are barycentric f64
//! J2000 equatorial AU/AU-day; Moon samples are parent-relative until composed. Observer preparation uses WGS84,
//! UT1 spin and TT model times, and completes light-time sampling before the correction-only observation stage.
mod catalog;
mod diagnostics;
mod illumination;
mod observation;
mod observer;
mod pipeline;
mod positions;
mod selection;
mod simulation;

pub use catalog::{prepare_catalog, prepare_owned_catalog, prepare_constellation_set, prepare_star, create_sky_from_catalog, select_grid};
pub use catalog::{catalog_fingerprint, cache_path, load_sky_catalog, load_sky_catalog_with_times, write_cached_catalog, load_cached_catalog};
pub use simulation::{update_solar_system, evaluate_body, evaluate_orientation, prepare_stellar_catalog, simulate_stars};
pub use observer::{begin_solar_system_frame, prepare_observer_inputs, compose_observer_state, prepare_observer, prepare_light_time_samples, prepare_observation,
    prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time, prepare_cached_bodies};
pub use selection::select_cached_stars;
pub use observation::{observe_cached_sky, observe_cached_regions};
pub use illumination::{compute_moon_illumination, name_moon_phase};
pub use pipeline::{observe_sky, observe_sky_candidates};
pub use positions::{refract_sky_positions, update_sky_positions};

pub(crate) use catalog::{select_region, select_brightness, count_region_stars};
pub(crate) use diagnostics::{snapshot_cache, record_cache, record_observer_memory, describe_cache_reports};
pub(crate) use observer::sample_body_states;
pub(crate) use selection::{filter_brightness_candidates, merge_constellation_endpoints};
pub(crate) use simulation::simulate_stars_direct;
pub(crate) use observation::apply_direct_observation;
