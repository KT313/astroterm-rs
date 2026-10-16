//! The records the projection geometry produces: points on the unit view disc, polar view coordinates, the
//! camera basis, visible parts of a great-circle arc and a sampled arc in viewport cells.
use crate::formulas::Vector3;
use crate::model::{Cell, ProjectionKind};

/// A visible interval of a great-circle arc, as fractions of the arc from its start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcPart {
    pub start: f64,
    pub end: f64,
    pub includes_start: bool,
    pub includes_end: bool,
}

/// A point on the projection plane; radius 1 is the edge of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub fn radius(self) -> f64 { self.x.hypot(self.y) }
    pub fn is_visible(self) -> bool { self.x * self.x + self.y * self.y <= 1.0 + 8.0 * f64::EPSILON }
    /// The same direction, pulled back onto the edge of the view when it lies outside.
    pub fn clamp_to_edge(self) -> Self {
        let radius = self.radius();
        if radius > 1.0 { Self { x: self.x / radius, y: self.y / radius } } else { self }
    }
}

/// The camera basis of a view in the horizon frame, with the scale that maps the field edge to radius 1.
#[derive(Clone, Copy, Debug)]
pub struct CartesianCamera {
    pub(crate) right: Vector3,
    pub(crate) up: Vector3,
    pub(crate) forward: Vector3,
    pub(crate) scale: f64,
    pub(crate) kind: ProjectionKind,
}

/// Polar view coordinates: the radius from the view centre (1 at the edge) and the angle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Polar {
    pub radius: f64,
    pub theta: f64,
}

impl Polar {
    /// Cartesian (x right, y up) coordinates.
    pub fn to_cartesian(self) -> (f64, f64) { (self.radius * self.theta.cos(), self.radius * self.theta.sin()) }
    /// Polar coordinates of a cartesian point.
    pub fn from_cartesian(x: f64, y: f64) -> Polar { Polar { radius: x.hypot(y), theta: y.atan2(x) } }
}

/// A visible, sampled part of a constellation segment in viewport cells.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedArc {
    pub start: Cell,
    pub end: Cell,
    /// Sampled projected great-circle vertices, in this viewport (cells or pixels).
    pub points: Vec<Cell>,
    pub includes_start: bool,
    pub includes_end: bool,
}
