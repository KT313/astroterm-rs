//! Camera geometry only: maps an immutable observed sky to screen points and clipped segments. No simulation
//! evaluation or astronomical corrections occur here. Character rendering consumes these prepared primitives.
use super::{Polar, View, ViewCenter};
use crate::astro::{Horizontal, offset_towards};
use crate::catalog::StarNames;
use crate::sky::{ObservedSky, ObservedStar, PlanetKind};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const EDGE_TOLERANCE: f64 = 1e-6;
pub type Cell = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub height: usize,
    pub width: usize,
}
impl Viewport {
    pub fn to_cell(self, polar: Polar) -> Cell {
        let radius_y = (self.height as f64 - 1.0) / 2.0;
        let radius_x = (self.width as f64 - 1.0) / 2.0;
        let snap = |value: f64| if value.abs() < 1e-12 { 0.0 } else { value };
        let (s, c) = (snap(polar.theta.sin()), snap(polar.theta.cos()));
        (
            (polar.radius * -radius_y * s + radius_y).round() as i32,
            (polar.radius * radius_x * c + radius_x).round() as i32,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedStar<'a> {
    pub star: &'a ObservedStar,
    pub cell: Option<Cell>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedPlanet {
    pub kind: PlanetKind,
    pub cell: Option<Cell>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedMoon {
    pub illumination: crate::sky::MoonIllumination,
    pub phase: crate::astro::MoonPhase,
    pub cell: Option<Cell>,
    pub lit_on_right: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedArc {
    pub start: Cell,
    pub end: Cell,
    pub includes_start: bool,
    pub includes_end: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedConstellation {
    pub maximum_magnitude: f32,
    pub arcs: Vec<ProjectedArc>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedSky<'a> {
    pub stars: Vec<ProjectedStar<'a>>,
    pub planets: Vec<ProjectedPlanet>,
    pub moon: ProjectedMoon,
    pub constellations: Vec<ProjectedConstellation>,
    pub names: &'a StarNames,
    pub facing: bool,
    pub viewport: Viewport,
    pub horizon: Vec<[Cell; 2]>,
    pub horizon_labels: Vec<(Cell, &'static str)>,
}

pub fn project_sky<'a>(sky: &'a ObservedSky, view: &View, viewport: Viewport) -> ProjectedSky<'a> {
    let cell = |position| {
        let polar = view.project(position);
        is_on_disk(polar).then(|| viewport.to_cell(polar))
    };
    let stars = sky
        .stars
        .iter()
        .map(|star| ProjectedStar {
            star,
            cell: cell(star.position),
        })
        .collect();
    let planets = sky
        .planets
        .iter()
        .map(|planet| ProjectedPlanet {
            kind: planet.kind,
            cell: cell(planet.position),
        })
        .collect();
    let moon = ProjectedMoon {
        illumination: sky.moon.illumination,
        phase: sky.moon.phase,
        cell: cell(sky.moon.position),
        lit_on_right: is_lit_on_right(view, sky.moon.position, sky.sun().position),
    };
    let constellations = sky
        .constellations
        .iter()
        .filter_map(|figure| {
            if figure.segments.iter().flatten().any(|&index| index >= sky.stars.len()) {
                return None;
            }
            let maximum_magnitude = figure
                .segments
                .iter()
                .flatten()
                .map(|&index| sky.stars[index].magnitude)
                .fold(f32::NEG_INFINITY, f32::max);
            let arcs = figure
                .segments
                .iter()
                .flat_map(|&[a, b]| {
                    project_constellation_segment(view, viewport, sky.stars[a].position, sky.stars[b].position)
                })
                .collect();
            Some(ProjectedConstellation {
                maximum_magnitude,
                arcs,
            })
        })
        .collect();
    ProjectedSky {
        stars,
        planets,
        moon,
        constellations,
        names: &sky.names,
        facing: view.is_facing(),
        viewport,
        horizon: project_horizon_line(view, viewport),
        horizon_labels: project_horizon_labels(view, viewport),
    }
}

/// Whether, in this view, the direction from the Moon towards the Sun points to the right of the screen.
fn is_lit_on_right(view: &View, moon: Horizontal, sun: Horizontal) -> bool {
    let towards_sun = offset_towards(moon, sun, 1_f64.to_radians());
    let (moon_x, _) = view.project(moon).to_cartesian();
    let (towards_sun_x, _) = view.project(towards_sun).to_cartesian();
    towards_sun_x > moon_x
}

/// Whether a projected point is in view (on the unit disk).
fn is_on_disk(polar: Polar) -> bool {
    polar.radius.abs() <= 1.0
}

/// Draw the visible parts of the great-circle arc between two stars, each as a straight line between where it enters
/// and leaves the view, with markers on the stars that are in view.
pub fn project_constellation_segment(
    view: &View,
    viewport: Viewport,
    from: Horizontal,
    to: Horizontal,
) -> Vec<ProjectedArc> {
    let mut arcs = Vec::new();
    for part in view.find_visible_arc_parts(from, to) {
        // ends of the visible part, pulled onto the edge where rounding puts them just outside
        let project_on_arc = |angle: f64| {
            let polar = view.project(offset_towards(from, to, angle));
            viewport.to_cell(Polar {
                radius: polar.radius.min(1.0),
                ..polar
            })
        };
        let (start_cell, end_cell) = (project_on_arc(part.start), project_on_arc(part.end));

        arcs.push(ProjectedArc {
            start: start_cell,
            end: end_cell,
            includes_start: part.includes_start,
            includes_end: part.includes_end,
        });
    }
    arcs
}
/// Trace the visible part of the horizon in the facing view.
pub fn project_horizon_line(view: &View, viewport: Viewport) -> Vec<[Cell; 2]> {
    let mut lines = Vec::new();
    let ViewCenter::Facing {
        azimuth: facing_azimuth,
        tilt,
    } = view.center
    else {
        return lines;
    };
    let Some(half_range) = compute_visible_horizon_half_range(view.fov_degrees, tilt) else {
        return lines;
    };

    // sample the horizon at 4 points per column (empirical)
    let sample_count = 4 * viewport.width as i32;
    let start_azimuth = facing_azimuth - half_range;
    let step = 2.0 * half_range / f64::from(sample_count);
    let project_horizon = |azimuth: f64| view.project(Horizontal { azimuth, altitude: 0.0 });

    // join samples into segments once they are 4 columns or 2 rows apart (empirical), since the line functions can't
    // draw the slope of tiny segments
    let mut previous = project_horizon(start_azimuth);
    let mut segment_start: Option<(i32, i32)> = None;
    for index in 1..=sample_count {
        let current = project_horizon(start_azimuth + f64::from(index) * step);
        let previous_visible = previous.radius <= 1.0 + EDGE_TOLERANCE;
        let visible = current.radius <= 1.0 + EDGE_TOLERANCE;

        if previous_visible || visible {
            let start = *segment_start.get_or_insert_with(|| viewport.to_cell(clamp_to_edge(previous)));
            let end = viewport.to_cell(clamp_to_edge(current));
            let far_enough = (end.1 - start.1).abs() >= 4 || (end.0 - start.0).abs() >= 2;
            if !visible || index == sample_count || far_enough {
                if end != start {
                    lines.push([start, end]); // the line functions skip zero-length segments
                }
                segment_start = visible.then_some(end);
            }
        }
        previous = current;
    }
    lines
}

/// Label the compass directions on the horizon, and the zenith and nadir, where they are in view.
pub fn project_horizon_labels(view: &View, viewport: Viewport) -> Vec<(Cell, &'static str)> {
    if !view.is_facing() {
        return Vec::new();
    }
    let mut labels = Vec::new();
    const DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

    // the 8 main directions, except on the very edge where labels get cut off
    for (index, label) in DIRECTIONS.iter().enumerate() {
        let azimuth = index as f64 * TAU / DIRECTIONS.len() as f64;
        let polar = view.project(Horizontal { azimuth, altitude: 0.0 });
        if polar.radius >= 1.0 - EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = viewport.to_cell(polar);
        labels.push(((row, col - (label.len() as i32 - 1) / 2), *label));
    }

    // zenith and nadir, edge included
    for (label, altitude) in [("Zenith", FRAC_PI_2), ("Nadir", -FRAC_PI_2)] {
        if (altitude + view.tilt()).abs() < EDGE_TOLERANCE {
            continue; // directly behind the view (tilt ±90° at fov 360°), it would sit on an arbitrary edge point
        }
        let polar = view.project(Horizontal { azimuth: 0.0, altitude });
        if polar.radius > 1.0 + EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = viewport.to_cell(clamp_to_edge(polar));
        labels.push(((row, col - label.len() as i32 / 2), label));
    }
    labels
}

/// Half the azimuth range of the horizon that is in view, or `None` if the horizon is out of view.
///
/// A horizon point at azimuth offset Δ is c away from the view center, with cos(c) = cos(tilt)·cos(Δ). It is in view
/// while c <= fov/2.
pub(crate) fn compute_visible_horizon_half_range(fov_degrees: f64, tilt: f64) -> Option<f64> {
    let cos_half_fov = (fov_degrees.to_radians() / 2.0).cos();
    let cos_tilt = tilt.cos();
    if cos_tilt < 1e-9 {
        return Some(PI); // looking straight up or down: the horizon is a circle around the center
    }
    if cos_half_fov >= cos_tilt {
        return None;
    }
    // pad 5% so the line reaches the edge; clamped since cos(fov/2) < 0 above 180°, and kept off the point directly
    // behind, which has an arbitrary direction in the equidistant projection
    Some((PI - 1e-6).min((cos_half_fov / cos_tilt).max(-1.0).acos() * 1.05))
}

/// Pull points just outside the unit circle back onto it.
fn clamp_to_edge(polar: Polar) -> Polar {
    Polar {
        radius: polar.radius.min(1.0),
        ..polar
    }
}
