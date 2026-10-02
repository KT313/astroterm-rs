//! The view onto the sky: where it is centered, which projection is used, and how much of the sky it shows.

mod maps;

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::astro::{Horizontal, horizontal_to_spherical};

pub use maps::{
    Polar, polar_to_cell, project_equidistant_horizontal, project_stereographic_horizontal, project_stereographic_north,
};

/// Field of view that maps exactly onto the unit circle without scaling.
const DEFAULT_FOV_DEGREES: f64 = 180.0;

/// Smallest field of view that zooming in reaches.
const MIN_FOV_DEGREES: f64 = 1.0;

/// Where the center of the view points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewCenter {
    /// Lying on your back looking straight up, with North at the top.
    Zenith,
    /// Looking towards `azimuth`, tilted up by `tilt` from the horizon (radians).
    Facing { azimuth: f64, tilt: f64 },
}

/// Azimuthal projection used to flatten the sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionKind {
    Stereographic,
    Equidistant,
}

impl ProjectionKind {
    /// Largest field of view in degrees. The stereographic projection sends the point directly behind the center to
    /// infinity, so it can't show the full 360°.
    pub fn max_fov_degrees(self) -> f64 {
        match self {
            ProjectionKind::Stereographic => 359.0,
            ProjectionKind::Equidistant => 360.0,
        }
    }
}

/// A view of the sky. Projected points with radius > 1 are out of view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: ViewCenter,
    pub projection: ProjectionKind,
    /// Angular diameter of the rendered circle in degrees.
    pub fov_degrees: f64,
}

impl Default for View {
    fn default() -> View {
        View {
            center: ViewCenter::Zenith,
            projection: ProjectionKind::Stereographic,
            fov_degrees: DEFAULT_FOV_DEGREES,
        }
    }
}

impl View {
    /// Whether the view faces a direction (with the horizon across it) rather than the zenith.
    pub fn is_facing(&self) -> bool {
        matches!(self.center, ViewCenter::Facing { .. })
    }

    /// Tilt of the view above the horizon in radians, π/2 for the zenith view.
    pub fn tilt(&self) -> f64 {
        match self.center {
            ViewCenter::Zenith => FRAC_PI_2,
            ViewCenter::Facing { tilt, .. } => tilt,
        }
    }

    /// Turn the view right by `azimuth_delta` and up by `tilt_delta` (radians), keeping the tilt within [-90°, 90°].
    /// The zenith view first becomes the identical facing view: South, tilted up 90°.
    pub fn pan(&mut self, azimuth_delta: f64, tilt_delta: f64) {
        let (azimuth, tilt) = match self.center {
            ViewCenter::Zenith => (PI, FRAC_PI_2),
            ViewCenter::Facing { azimuth, tilt } => (azimuth, tilt),
        };
        self.center = ViewCenter::Facing {
            azimuth: (azimuth + azimuth_delta).rem_euclid(TAU),
            tilt: (tilt + tilt_delta).clamp(-FRAC_PI_2, FRAC_PI_2),
        };
    }

    /// Zoom in by `factor` (zoom out for factors below 1), keeping the field of view within what the projection can
    /// show.
    pub fn zoom(&mut self, factor: f64) {
        let max_fov = self.projection.max_fov_degrees();
        self.fov_degrees = (self.fov_degrees / factor).clamp(MIN_FOV_DEGREES, max_fov);
    }

    /// Project horizontal coordinates onto the view plane, scaled so that fov/2 from the center lands on the unit
    /// circle.
    pub fn project(&self, position: Horizontal) -> Polar {
        match self.projection {
            ProjectionKind::Equidistant => self.project_equidistant(position),
            ProjectionKind::Stereographic => self.project_stereographic(position),
        }
    }

    fn project_equidistant(&self, position: Horizontal) -> Polar {
        let center = match self.center {
            ViewCenter::Zenith => Horizontal {
                azimuth: PI,
                altitude: FRAC_PI_2,
            }, // same as facing S tilted up 90°
            ViewCenter::Facing { azimuth, tilt } => Horizontal {
                azimuth,
                altitude: tilt,
            },
        };
        let mut polar = project_equidistant_horizontal(position, center);
        polar.radius *= DEFAULT_FOV_DEGREES / self.fov_degrees;
        polar
    }

    fn project_stereographic(&self, position: Horizontal) -> Polar {
        let mut polar = match self.center {
            ViewCenter::Zenith => {
                let (theta, phi) = horizontal_to_spherical(position);
                project_stereographic_north(1.0, theta, phi)
            }
            ViewCenter::Facing { azimuth, tilt } => project_stereographic_horizontal(
                position,
                Horizontal {
                    azimuth,
                    altitude: tilt,
                },
            ),
        };
        if self.fov_degrees != DEFAULT_FOV_DEGREES {
            polar.radius /= (self.fov_degrees.to_radians() / 4.0).tan();
        }
        polar
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TO_RAD: f64 = PI / 180.0;

    fn facing(azimuth: f64, tilt: f64, projection: ProjectionKind, fov_degrees: f64) -> View {
        View {
            center: ViewCenter::Facing { azimuth, tilt },
            projection,
            fov_degrees,
        }
    }

    fn radius_at(view: &View, azimuth_degrees: f64, altitude_degrees: f64) -> f64 {
        view.project(Horizontal {
            azimuth: azimuth_degrees * TO_RAD,
            altitude: altitude_degrees * TO_RAD,
        })
        .radius
    }

    #[test]
    fn panning_turns_and_tilts_the_view() {
        let mut view = facing(350_f64.to_radians(), 0.0, ProjectionKind::Stereographic, 180.0);
        view.pan(20_f64.to_radians(), 100_f64.to_radians());
        let ViewCenter::Facing { azimuth, tilt } = view.center else {
            panic!("facing view expected")
        };
        assert!((azimuth - 10_f64.to_radians()).abs() < 1e-12); // wrapped past North
        assert_eq!(tilt, FRAC_PI_2); // clamped at the zenith
    }

    #[test]
    fn panning_the_zenith_view_starts_from_the_identical_facing_view() {
        let mut view = View::default();
        let position = Horizontal {
            azimuth: 1.0,
            altitude: 0.7,
        };
        let before = view.project(position);
        view.pan(0.0, 0.0);
        let after = view.project(position);
        assert!(view.is_facing());
        assert!((before.radius - after.radius).abs() < 1e-9);
        assert!(
            (before.theta - after.theta)
                .rem_euclid(TAU)
                .min((after.theta - before.theta).rem_euclid(TAU))
                < 1e-9
        );
    }

    #[test]
    fn zoom_stays_within_the_projection_limits() {
        let mut view = View::default();
        view.zoom(2.0);
        assert_eq!(view.fov_degrees, 90.0);
        view.zoom(1e-6);
        assert_eq!(view.fov_degrees, 359.0);
        view.projection = ProjectionKind::Equidistant;
        view.zoom(1e-6);
        assert_eq!(view.fov_degrees, 360.0);
        view.zoom(1e6);
        assert_eq!(view.fov_degrees, 1.0);
    }

    #[test]
    fn stereographic_fov_scales_edge_of_view() {
        let narrow = facing(0.0, 0.0, ProjectionKind::Stereographic, 90.0);
        assert!((radius_at(&narrow, 45.0, 0.0) - 1.0).abs() < 1e-9);
        assert!(radius_at(&narrow, 0.0, 0.0).abs() < 1e-9);

        let wide = facing(0.0, 0.0, ProjectionKind::Stereographic, 270.0);
        assert!((radius_at(&wide, 135.0, 0.0) - 1.0).abs() < 1e-9);

        let overhead = View {
            fov_degrees: 90.0,
            ..View::default()
        };
        assert!((radius_at(&overhead, 123.0, 45.0) - 1.0).abs() < 1e-9);
        assert!(radius_at(&overhead, 123.0, 90.0).abs() < 1e-9);
    }

    #[test]
    fn stereographic_default_fov_is_unscaled() {
        let view = facing(1.0, 0.2, ProjectionKind::Stereographic, 180.0);
        let position = Horizontal {
            azimuth: 2.0,
            altitude: 0.5,
        };
        let raw = project_stereographic_horizontal(
            position,
            Horizontal {
                azimuth: 1.0,
                altitude: 0.2,
            },
        );
        assert_eq!(view.project(position), raw);
    }

    #[test]
    fn equidistant_fov_scales_edge_of_view() {
        let full = facing(0.0, 0.0, ProjectionKind::Equidistant, 360.0);
        assert!((radius_at(&full, 180.0, 0.0) - 1.0).abs() < 1e-9);
        assert!((radius_at(&full, 90.0, 0.0) - 0.5).abs() < 1e-9);

        let half = facing(0.0, 0.0, ProjectionKind::Equidistant, 180.0);
        assert!((radius_at(&half, 90.0, 0.0) - 1.0).abs() < 1e-9);

        let overhead = View {
            projection: ProjectionKind::Equidistant,
            fov_degrees: 360.0,
            ..View::default()
        };
        assert!((radius_at(&overhead, 123.0, -90.0) - 1.0).abs() < 1e-9);
        assert!((radius_at(&overhead, 123.0, 0.0) - 0.5).abs() < 1e-9);
    }
}
