//! Independent solar-system and catalog-star simulations.
mod solar_system;
mod stars;
pub use solar_system::{update_solar_system, evaluate_body, evaluate_orientation};
pub use stars::{prepare_stellar_catalog, simulate_stars};
pub(crate) use stars::simulate_stars_direct;
