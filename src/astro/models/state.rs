//! Common geometric states and stable identity, independent of the selected ephemeris.
use crate::astro::Vector3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyId {
    Sun,
    Mercury,
    Venus,
    Earth,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Moon,
}

impl BodyId {
    pub const PLANETS: [Self; 9] = [
        Self::Sun,
        Self::Mercury,
        Self::Venus,
        Self::Earth,
        Self::Mars,
        Self::Jupiter,
        Self::Saturn,
        Self::Uranus,
        Self::Neptune,
    ];
}

/// Geometric J2000 equatorial state, AU and AU/day. The common origin is the solar-system barycenter. Lunar samples are explicitly parent-relative until composition.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyState {
    pub position: Vector3,
    pub velocity: Vector3,
}

impl BodyState {
    pub fn evaluate(self, days: f64) -> Self {
        Self {
            position: self.position + self.velocity * days,
            ..self
        }
    }
    pub fn add_parent(self, parent: Self) -> Self {
        Self {
            position: self.position + parent.position,
            velocity: self.velocity + parent.velocity,
        }
    }
}

/// Symmetric velocity with an exactly represented 1/1024-day offset (~84 seconds). Divides by the actual date
/// difference to retain precision at large Julian dates; both ends use the same frame adapter.
pub(super) fn evaluate_with_velocity(epoch: f64, position: impl Fn(f64) -> Vector3) -> BodyState {
    let (before, after) = (epoch - 1.0 / 1024.0, epoch + 1.0 / 1024.0);
    BodyState {
        position: position(epoch),
        velocity: (position(after) - position(before)) * (1.0 / (after - before)),
    }
}
