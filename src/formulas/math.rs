//! Vector, matrix and shared numerical operations used by the physical formulas.
use std::f64::consts::TAU;
use super::{Vector3, Matrix3, Equatorial, Horizontal, BodyState};

impl Horizontal {
    /// Unit vector pointing at this position, with x East, y North and z up.
    pub fn to_unit_vector(self) -> Vector3 {
        let (sin_alt, cos_alt) = self.altitude.sin_cos();
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        Vector3 {
            x: cos_alt * sin_az,
            y: cos_alt * cos_az,
            z: sin_alt,
        }
    }

    /// Position a vector (x East, y North, z up) points at.
    pub fn from_vector(vector: Vector3) -> Horizontal {
        let altitude = (vector.z / vector.length()).clamp(-1.0, 1.0).asin();
        Horizontal {
            azimuth: vector.x.atan2(vector.y).rem_euclid(TAU),
            altitude,
        }
    }
}

impl Equatorial {
    /// Unit vector pointing at this position, with x towards the equinox, y at right ascension 90° and z North.
    pub fn to_unit_vector(self) -> Vector3 {
        let (sin_dec, cos_dec) = self.declination.sin_cos();
        let (sin_ra, cos_ra) = self.right_ascension.sin_cos();
        Vector3 {
            x: cos_dec * cos_ra,
            y: cos_dec * sin_ra,
            z: sin_dec,
        }
    }
}

impl Vector3 {
    pub fn normalized(self) -> Self {
        self * (1.0 / self.x.hypot(self.y).hypot(self.z))
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn cross(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn dot(self, other: Vector3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
}

impl std::ops::Add for Vector3 {
    type Output = Vector3;

    fn add(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

impl std::ops::Mul<f64> for Vector3 {
    type Output = Vector3;

    fn mul(self, factor: f64) -> Vector3 {
        Vector3 {
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }
}

impl std::ops::Sub for Vector3 {
    type Output = Vector3;

    fn sub(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }
}

impl std::ops::Neg for Vector3 {
    type Output = Vector3;

    fn neg(self) -> Vector3 {
        Vector3 {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

/// Normalize an angle in radians to [0, 2π).
pub fn normalize_radians(angle: f64) -> f64 {
    angle.rem_euclid(TAU)
}

impl Matrix3 {

    pub fn apply(self, v: Vector3) -> Vector3 {
        let [x, y, z] = self.0.map(|row| row[0] * v.x + row[1] * v.y + row[2] * v.z);
        Vector3 { x, y, z }
    }

    pub fn transpose(self) -> Self {
        Self(std::array::from_fn(|i| std::array::from_fn(|j| self.0[j][i])))
    }

    pub fn compose(self, right: Self) -> Self {
        Self(std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                self.0[i][0] * right.0[0][j] + self.0[i][1] * right.0[1][j] + self.0[i][2] * right.0[2][j]
            })
        }))
    }

    /// Passive rotation about +Z, as in ERFA R3.
    pub fn rotate_z(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
    }
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
pub(crate) fn evaluate_with_velocity(epoch: f64, position: impl Fn(f64) -> Vector3) -> BodyState {
    let (before, after) = (epoch - 1.0 / 1024.0, epoch + 1.0 / 1024.0);
    BodyState {
        position: position(epoch),
        velocity: (position(after) - position(before)) * (1.0 / (after - before)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn normalized_finite_angles_stay_in_one_turn(angle in -1e12_f64..1e12_f64) {
            let wrapped = normalize_radians(angle);
            prop_assert!((0.0..TAU).contains(&wrapped));
            prop_assert_eq!(normalize_radians(wrapped), wrapped);
        }
    }

    #[test]
    fn normalize_radians_wraps_into_one_turn() {
        assert!((normalize_radians(-0.5) - (TAU - 0.5)).abs() < 1e-12);
        assert!((normalize_radians(TAU + 0.25) - 0.25).abs() < 1e-12);
        assert_eq!(normalize_radians(0.0), 0.0);
    }
}
