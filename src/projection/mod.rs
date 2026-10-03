//! The view onto the sky: where it is centered, which projection is used, and how much of the sky it shows.

mod cartesian;
mod maps;
pub use cartesian::{CartesianCamera, ScreenPoint};
mod sky;
pub use sky::{ProjectedArc, ProjectedMoon, ProjectedPlanet, ProjectedSky, ProjectedStar, Viewport, project_sky};
#[cfg(test)]
pub(crate) use sky::{
    compute_visible_horizon_half_range, project_constellation_segment, project_horizon_labels, project_horizon_line,
};

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::astro::{Horizontal, horizontal_to_spherical};

pub use maps::{Polar, project_equidistant_horizontal, project_stereographic_horizontal, project_stereographic_north};

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

/// A visible part of a great-circle arc: angles (radians) from the arc's start, and whether it reaches the arc's ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcPart {
    pub start: f64,
    pub end: f64,
    pub includes_start: bool,
    pub includes_end: bool,
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
    /// Renderer-neutral observation region. The observer rotates it into the inertial catalog frame.
    pub fn sky_region(&self) -> crate::sky::SkyRegion {
        crate::sky::SkyRegion::Cone {
            center: self.center_direction().to_unit_vector(),
            radius: self.fov_degrees.to_radians() / 2.0,
        }
    }

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

    /// The direction the center of the view points at.
    pub fn center_direction(&self) -> Horizontal {
        match self.center {
            ViewCenter::Zenith => Horizontal {
                azimuth: PI,
                altitude: FRAC_PI_2,
            },
            ViewCenter::Facing { azimuth, tilt } => Horizontal {
                azimuth,
                altitude: tilt,
            },
        }
    }

    /// The parts of the shorter great-circle arc from `from` to `to` that are in view (at most fov/2 from the center).
    ///
    /// Solved exactly: along the arc, the cosine of the distance to the center is `R·cos(t - φ)`, so each full turn
    /// has one visible window `|t - φ| <= acos(cos(fov/2) / R)`. Within an arc that is at most two parts, e.g. when
    /// the arc dips into the hidden cap behind a view wider than 180°.
    pub fn find_visible_arc_parts(&self, from: Horizontal, to: Horizontal) -> Vec<ArcPart> {
        self.find_visible_arc_parts_vectors(from.to_unit_vector(), to.to_unit_vector())
    }

    pub fn find_visible_arc_parts_vectors(&self, a: crate::astro::Vector3, b: crate::astro::Vector3) -> Vec<ArcPart> {
        // the arc as p(t) = a·cos(t) + u·sin(t), t in [0, length]
        let length = a.dot(b).clamp(-1.0, 1.0).acos();
        let tangent = b - a * a.dot(b);
        let (center, cos_half_fov) = (
            self.center_direction().to_unit_vector(),
            (self.fov_degrees.to_radians() / 2.0).cos(),
        );
        if tangent.length() < 1e-12 {
            let visible = a.dot(center) >= cos_half_fov; // a point (or a degenerate arc)
            return if visible {
                vec![ArcPart {
                    start: 0.0,
                    end: length,
                    includes_start: true,
                    includes_end: true,
                }]
            } else {
                vec![]
            };
        }
        let u = tangent * (1.0 / tangent.length());

        // the visible window of angles, repeated every full turn
        let (p, q) = (a.dot(center), u.dot(center));
        let amplitude = p.hypot(q);
        let ratio = cos_half_fov / amplitude;
        let (phase, half_width) = match ratio {
            r if r <= -1.0 => (0.0, PI), // the whole great circle is in view
            r if r > 1.0 => return vec![],
            r => (q.atan2(p), r.acos()),
        };

        // intersect the windows with the arc
        let mut parts = Vec::new();
        for turn in [-TAU, 0.0, TAU] {
            let start = (phase - half_width + turn).max(0.0);
            let end = (phase + half_width + turn).min(length);
            if start <= end {
                parts.push(ArcPart {
                    start,
                    end,
                    includes_start: start == 0.0,
                    includes_end: end == length,
                });
            }
        }
        if half_width >= PI {
            parts.truncate(1); // the windows of neighboring turns touch; one part covers the arc
        }
        parts
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

    fn horizontal(azimuth_degrees: f64, altitude_degrees: f64) -> Horizontal {
        Horizontal {
            azimuth: azimuth_degrees.to_radians(),
            altitude: altitude_degrees.to_radians(),
        }
    }

    fn degrees(part: &ArcPart) -> (f64, f64) {
        (
            (part.start.to_degrees() * 1e6).round() / 1e6,
            (part.end.to_degrees() * 1e6).round() / 1e6,
        )
    }

    #[test]
    fn arc_fully_in_view_is_one_part_with_both_ends() {
        let view = facing(0.0, 0.0, ProjectionKind::Stereographic, 90.0);
        let parts = view.find_visible_arc_parts(horizontal(-10.0, 0.0), horizontal(20.0, 0.0));
        assert_eq!(parts.len(), 1);
        assert_eq!(degrees(&parts[0]), (0.0, 30.0));
        assert!(parts[0].includes_start && parts[0].includes_end);
    }

    #[test]
    fn arc_leaving_the_view_is_cut_at_the_edge() {
        let view = facing(0.0, 0.0, ProjectionKind::Stereographic, 90.0);
        let parts = view.find_visible_arc_parts(horizontal(0.0, 0.0), horizontal(90.0, 0.0));
        assert_eq!(parts.len(), 1);
        assert_eq!(degrees(&parts[0]), (0.0, 45.0));
        assert!(parts[0].includes_start && !parts[0].includes_end);
    }

    #[test]
    fn arc_crossing_the_view_keeps_the_middle() {
        let view = facing(0.0, 0.0, ProjectionKind::Stereographic, 90.0);
        let parts = view.find_visible_arc_parts(horizontal(-60.0, 0.0), horizontal(60.0, 0.0));
        assert_eq!(parts.len(), 1);
        assert_eq!(degrees(&parts[0]), (15.0, 105.0));
        assert!(!parts[0].includes_start && !parts[0].includes_end);
    }

    #[test]
    fn arc_inside_the_hidden_cap_of_a_wide_view_is_invisible() {
        // overhead at 270°, the 45° cap around the nadir is hidden; this arc straddles the nadir
        let view = View {
            fov_degrees: 270.0,
            ..View::default()
        };
        assert!(
            view.find_visible_arc_parts(horizontal(0.0, -85.0), horizontal(180.0, -85.0))
                .is_empty()
        );
    }

    #[test]
    fn arc_dipping_into_the_hidden_cap_has_two_parts() {
        let view = View {
            fov_degrees: 270.0,
            ..View::default()
        };
        let parts = view.find_visible_arc_parts(horizontal(0.0, -40.0), horizontal(180.0, -40.0));
        let ranges: Vec<_> = parts.iter().map(degrees).collect();
        assert_eq!(ranges, [(0.0, 5.0), (95.0, 100.0)]);
        assert!(parts[0].includes_start && !parts[0].includes_end);
        assert!(!parts[1].includes_start && parts[1].includes_end);
    }

    #[test]
    fn arc_in_a_full_360_view_is_whole() {
        let view = View {
            projection: ProjectionKind::Equidistant,
            fov_degrees: 360.0,
            ..View::default()
        };
        let parts = view.find_visible_arc_parts(horizontal(0.0, -40.0), horizontal(180.0, -40.0));
        assert_eq!(parts.len(), 1);
        assert_eq!(degrees(&parts[0]), (0.0, 100.0));
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
