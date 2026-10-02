//! Pure model families. Stable body identity is independent of formula choice. Catalog I/O, observation, scheduling,
//! camera projection and rendering live outside this layer. All present physical calculations use f64.
pub mod moons;
pub mod orientation;
pub mod planets;
pub mod stars;
mod state;
pub use state::{BodyId, BodyState};
