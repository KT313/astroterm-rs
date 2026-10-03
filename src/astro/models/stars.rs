//! Straight-line J2000 stellar motion, f64. Directions and scaled velocities are dimensionless and per Julian year;
//! distances are parsecs. Bounds and brightness keys cover the shared TT computational interval, not physical
//! catalog accuracy. Missing/singular distances use tangential motion and constant brightness.
use crate::astro::{COMPUTATIONAL_INTERVAL, Equatorial, J2000, JULIAN_YEAR_DAYS, Vector3};

pub const SINGULAR_RATIO: f64 = 1e-3;
pub const ALWAYS_CHECKED_ANGLE: f64 = std::f64::consts::PI / 720.0; // 15 arcminutes

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StellarMotion {
    pub u0: Vector3,
    pub w: Vector3,
    pub distance_pc: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StellarSample {
    pub direction: Vector3,
    pub distance_ratio: f64,
    pub magnitude: f64,
    pub used_singular_fallback: bool,
}

pub fn years_since_j2000(julian_date_tt: f64) -> f64 {
    (julian_date_tt - J2000) / JULIAN_YEAR_DAYS
}

pub fn computational_years() -> (f64, f64) {
    (
        years_since_j2000(COMPUTATIONAL_INTERVAL.start_tt),
        years_since_j2000(COMPUTATIONAL_INTERVAL.end_tt),
    )
}

/// Tangential vector from dRA/dyear and dDec/dyear (the RA rate itself, not multiplied by cos declination).
pub fn compute_tangential_motion(direction: Equatorial, motion: Equatorial) -> Vector3 {
    let (sa, ca) = direction.right_ascension.sin_cos();
    let (sd, cd) = direction.declination.sin_cos();
    Vector3 {
        x: -sa * cd * motion.right_ascension - ca * sd * motion.declination,
        y: ca * cd * motion.right_ascension - sa * sd * motion.declination,
        z: cd * motion.declination,
    }
}

impl StellarMotion {
    /// Preserve a catalog's precise spherical direction while scaling its Cartesian velocity by validated distance.
    pub fn from_direction_velocity(direction: Equatorial, distance_pc: f64, velocity_pc_year: Vector3) -> Self {
        Self {
            u0: direction.to_unit_vector(),
            w: Vector3 {
                x: velocity_pc_year.x / distance_pc,
                y: velocity_pc_year.y / distance_pc,
                z: velocity_pc_year.z / distance_pc,
            },
            distance_pc: Some(distance_pc),
        }
    }

    /// Tangential proper motion as catalog µα* (dRA/dt · cos declination) and dDec/dt, including at the poles.
    pub fn from_sky_motion(direction: Equatorial, ra_cos_dec: f64, dec_motion: f64) -> Self {
        let (sa, ca) = direction.right_ascension.sin_cos();
        let (sd, cd) = direction.declination.sin_cos();
        let w = Vector3 {
            x: -sa * ra_cos_dec - ca * sd * dec_motion,
            y: ca * ra_cos_dec - sa * sd * dec_motion,
            z: cd * dec_motion,
        };
        Self {
            u0: direction.to_unit_vector(),
            w,
            distance_pc: None,
        }
    }

    pub fn from_angles(direction: Equatorial, proper_motion: Equatorial) -> Self {
        Self {
            u0: direction.to_unit_vector(),
            w: compute_tangential_motion(direction, proper_motion),
            distance_pc: None,
        }
    }

    /// Minimize the distance quadratic over a closed interval (including the computational interval's limiting
    /// upper endpoint gives a conservative key for its half-open runtime domain).
    pub fn closest_approach(self, start: f64, end: f64) -> (f64, f64) {
        let speed = length(self.w);
        let time = if speed == 0.0 {
            start
        } else {
            (-self.u0.dot(self.w * (1.0 / speed)) / speed).clamp(start, end)
        };
        (time, length(self.u0 + self.w * time))
    }

    fn tangential_velocity(self) -> Vector3 {
        self.w - self.u0 * (self.u0.dot(self.w) / self.u0.dot(self.u0))
    }

    /// Apply the single load-time singular policy to both direction and brightness. Returns whether it fired.
    pub fn remove_singular_distance(&mut self) -> bool {
        let (start, end) = computational_years();
        if self.distance_pc.is_some() && self.closest_approach(start, end).1 < SINGULAR_RATIO {
            self.w = self.tangential_velocity();
            self.distance_pc = None;
            true
        } else {
            false
        }
    }

    pub fn evaluate(self, years: f64, magnitude: f64) -> StellarSample {
        let mut q = self.u0 + self.w * years;
        let mut ratio = length(q);
        let singular = self.distance_pc.is_some() && ratio < SINGULAR_RATIO;
        if singular {
            q = self.u0 + self.tangential_velocity() * years;
            ratio = length(q);
        }
        let norm = ratio;
        if years == 0.0 || self.w == Vector3::default() {
            ratio = 1.0;
        }
        let current = if self.distance_pc.is_some() && !singular {
            magnitude + 5.0 * ratio.log10()
        } else {
            magnitude
        };
        StellarSample {
            direction: q * (1.0 / norm),
            distance_ratio: ratio,
            magnitude: current,
            used_singular_fallback: singular,
        }
    }

    pub fn brightest_magnitude(self, magnitude: f64) -> f64 {
        if self.distance_pc.is_none() {
            return magnitude;
        }
        let (start, end) = computational_years();
        (magnitude + 5.0 * self.closest_approach(start, end).1.log10())
            .min(magnitude)
            .next_down()
    }

    /// Exact endpoint angular bound for a non-singular straight line; atan2 is stable at poles and small angles.
    pub fn motion_bound(self) -> f64 {
        let (start, end) = computational_years();
        [start, end]
            .into_iter()
            .map(|t| {
                let q = self.u0 + self.w * t;
                length(self.u0.cross(q)).atan2(self.u0.dot(q))
            })
            .fold(0.0, f64::max)
            .next_up()
    }
}

fn length(v: Vector3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

/// Historical linear-RA/Dec reference, retained for comparison fixtures. Runtime uses StellarMotion.
/// Apply proper motion (radians per year) to a J2000 catalog position. The result is still in the J2000 frame.
/// Time is TT; the legacy 365.2425-day year is kept only in this historical comparison API.
pub fn compute_star_position(catalog: Equatorial, proper_motion: Equatorial, julian_date_tt: f64) -> Equatorial {
    let years_since_j2000 = (julian_date_tt - J2000) / 365.2425;
    Equatorial {
        right_ascension: catalog.right_ascension + proper_motion.right_ascension * years_since_j2000,
        declination: catalog.declination + proper_motion.declination * years_since_j2000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn motion(w: Vector3, distance: Option<f64>) -> StellarMotion {
        StellarMotion {
            u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w,
            distance_pc: distance,
        }
    }
    #[test]
    fn closest_approach_inside_before_and_after_interval() {
        let star = motion(
            Vector3 {
                x: -0.1,
                y: 0.01,
                z: 0.0,
            },
            Some(1.0),
        );
        let time = 0.1 / 0.0101;
        assert!((star.closest_approach(0.0, 20.0).0 - time).abs() < 1e-12);
        assert_eq!(star.closest_approach(11.0, 20.0).0, 11.0);
        assert_eq!(star.closest_approach(0.0, 5.0).0, 5.0);
    }
    #[test]
    fn singular_policy_applies_to_motion_and_brightness_once() {
        let mut star = motion(
            Vector3 {
                x: -0.01,
                y: 0.0,
                z: 0.0,
            },
            Some(10.0),
        );
        assert!(star.remove_singular_distance());
        assert!(!star.remove_singular_distance());
        assert_eq!(star.distance_pc, None);
        let sample = star.evaluate(100.0, 5.0);
        assert_eq!(sample.direction, star.u0);
        assert_eq!(sample.magnitude, 5.0);
        let mut outside = motion(
            Vector3 {
                x: -1.0 / 20000.0,
                y: 0.0,
                z: 0.0,
            },
            Some(10.0),
        );
        assert!(!outside.remove_singular_distance());
        let sample = outside.evaluate(20000.0, 5.0);
        assert!(sample.used_singular_fallback);
        assert_eq!(sample.magnitude, 5.0);
        assert_eq!(sample.direction, outside.u0);
    }
    #[test]
    fn normalized_motion_crosses_a_pole_without_ra_singularity() {
        let star = StellarMotion::from_angles(
            Equatorial {
                right_ascension: 0.0,
                declination: 89_f64.to_radians(),
            },
            Equatorial {
                right_ascension: 0.0,
                declination: 1_f64.to_radians(),
            },
        );
        let after = star.evaluate(2.0, 1.0).direction;
        assert!(after.x < 0.0 && after.z > 0.99);
        assert!((length(after) - 1.0).abs() < 1e-14);
    }
    #[test]
    fn fixed_distance_and_approaching_star_magnitudes() {
        let fixed = motion(Vector3::default(), Some(10000.0));
        assert_eq!(fixed.evaluate(10000.0, 5.0).magnitude, 5.0);
        let star = motion(
            Vector3 {
                x: -0.0001,
                y: 0.00001,
                z: 0.0,
            },
            Some(2.0),
        );
        assert!(star.evaluate(1000.0, 9.5).magnitude < 9.5);
        assert!(star.motion_bound() > ALWAYS_CHECKED_ANGLE);
    }
    proptest! {
        #[test]
        fn endpoint_bound_and_brightness_key_cover_sampled_trajectories(
            wx in -0.001_f64..0.001, wy in -0.001_f64..0.001,wz in -0.001_f64..0.001
        ) {
            let mut star=motion(Vector3{x:wx,y:wy,z:wz},Some(10.0));
            star.remove_singular_distance();
            let bound=star.motion_bound(); let key=star.brightest_magnitude(5.0);
            let (start,end)=computational_years();
            let mut sampled=0.0_f64;
            for i in 0..=100 {
                let sample=star.evaluate(start+(end-start)*i as f64/100.0,5.0);
                let angle=length(star.u0.cross(sample.direction)).atan2(star.u0.dot(sample.direction));
                sampled=sampled.max(angle);
                prop_assert!(angle<=bound+1e-12);
                prop_assert!(sample.magnitude>=key-1e-12);
            }
            prop_assert!((bound-sampled).abs()<1e-12);
        }
    }
}
