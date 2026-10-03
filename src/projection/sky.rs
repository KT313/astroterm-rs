//! Camera geometry only: maps an immutable observed sky to screen points and clipped segments. No simulation
//! evaluation or astronomical corrections occur here. Character rendering consumes these prepared primitives.
use super::{CartesianCamera, Polar, ScreenPoint, View, ViewCenter};
use crate::astro::{Horizontal, Vector3, offset_vector_towards};
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
    pub fn to_cell_cartesian(self, point: ScreenPoint) -> Cell {
        let snap = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
        let (ry, rx) = ((self.height as f64 - 1.0) / 2.0, (self.width as f64 - 1.0) / 2.0);
        (
            (-snap(point.y) * ry + ry).round() as i32,
            (snap(point.x) * rx + rx).round() as i32,
        )
    }

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
    pub maximum_magnitude: f64,
    pub arcs: Vec<ProjectedArc>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedSky<'a> {
    pub catalog_singular_count: usize,
    pub runtime_singular_count: usize,
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
    let camera = CartesianCamera::new(view);
    let cell = |position| {
        camera
            .project(position)
            .filter(|p| p.is_visible())
            .map(|p| viewport.to_cell_cartesian(p))
    };
    let mut stars: Vec<_> = sky
        .stars
        .iter()
        .filter(|star| star.drawable)
        .filter_map(|star| {
            cell(star.position).map(|point| ProjectedStar {
                star,
                cell: Some(point),
            })
        })
        .collect();
    stars.sort_unstable_by(|a, b| {
        if a.star.magnitude == b.star.magnitude {
            a.star.id.cmp(&b.star.id)
        } else {
            b.star.magnitude.total_cmp(&a.star.magnitude)
        }
    });
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
    let find_star = |index| {
        sky.stars
            .binary_search_by_key(&index, |star| star.source_index)
            .ok()
            .map(|i| &sky.stars[i])
    };
    let constellations = sky
        .constellations
        .iter()
        .filter_map(|figure| {
            let endpoints = figure
                .segments
                .iter()
                .flatten()
                .map(|&index| find_star(index))
                .collect::<Option<Vec<_>>>()?;
            let maximum_magnitude = endpoints
                .iter()
                .map(|star| star.magnitude)
                .fold(f64::NEG_INFINITY, f64::max);
            if maximum_magnitude > sky.magnitude_threshold {
                return None;
            }
            let arcs = endpoints
                .chunks_exact(2)
                .flat_map(|pair| project_constellation_segment(view, viewport, pair[0].position, pair[1].position))
                .collect();
            Some(ProjectedConstellation {
                maximum_magnitude,
                arcs,
            })
        })
        .collect();
    ProjectedSky {
        catalog_singular_count: sky.catalog.singular_count,
        runtime_singular_count: sky.runtime_singular_count,
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
fn is_lit_on_right(view: &View, moon: Vector3, sun: Vector3) -> bool {
    let camera = CartesianCamera::new(view);
    let offset = offset_vector_towards(moon, sun, 1_f64.to_radians());
    match (camera.project(moon), camera.project(offset)) {
        (Some(a), Some(b)) => b.x > a.x,
        _ => false,
    }
}

/// Draw the visible parts of the great-circle arc between two stars, each as a straight line between where it enters
/// and leaves the view, with markers on the stars that are in view.
pub fn project_constellation_segment(view: &View, viewport: Viewport, from: Vector3, to: Vector3) -> Vec<ProjectedArc> {
    let mut arcs = Vec::new();
    let camera = CartesianCamera::new(view);
    for part in view.find_visible_arc_parts_vectors(from, to) {
        // ends of the visible part, pulled onto the edge where rounding puts them just outside
        let project_on_arc = |angle| {
            camera
                .project(offset_vector_towards(from, to, angle))
                .map(|point| viewport.to_cell_cartesian(point.clamp_to_edge()))
        };
        let (Some(start_cell), Some(end_cell)) = (project_on_arc(part.start), project_on_arc(part.end)) else {
            continue;
        };

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
    let camera = CartesianCamera::new(view);
    let project_horizon = |azimuth: f64| camera.project(Horizontal { azimuth, altitude: 0.0 }.to_unit_vector());

    // join samples into segments once they are 4 columns or 2 rows apart (empirical), since the line functions can't
    // draw the slope of tiny segments
    let mut previous = project_horizon(start_azimuth);
    let mut segment_start: Option<(i32, i32)> = None;
    for index in 1..=sample_count {
        let current = project_horizon(start_azimuth + f64::from(index) * step);
        let previous_visible = previous.is_some_and(|point| point.radius() <= 1.0 + EDGE_TOLERANCE);
        let visible = current.is_some_and(|point| point.radius() <= 1.0 + EDGE_TOLERANCE);

        if (previous_visible || visible)
            && let (Some(previous_point), Some(current_point)) = (previous, current)
        {
            let start =
                *segment_start.get_or_insert_with(|| viewport.to_cell_cartesian(previous_point.clamp_to_edge()));
            let end = viewport.to_cell_cartesian(current_point.clamp_to_edge());
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
    let camera = CartesianCamera::new(view);
    const DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

    // the 8 main directions, except on the very edge where labels get cut off
    for (index, label) in DIRECTIONS.iter().enumerate() {
        let azimuth = index as f64 * TAU / DIRECTIONS.len() as f64;
        let Some(point) = camera.project(Horizontal { azimuth, altitude: 0.0 }.to_unit_vector()) else {
            continue;
        };
        if point.radius() >= 1.0 - EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = viewport.to_cell_cartesian(point);
        labels.push(((row, col - (label.len() as i32 - 1) / 2), *label));
    }

    // zenith and nadir, edge included
    for (label, altitude) in [("Zenith", FRAC_PI_2), ("Nadir", -FRAC_PI_2)] {
        if (altitude + view.tilt()).abs() < EDGE_TOLERANCE {
            continue; // directly behind the view (tilt ±90° at fov 360°), it would sit on an arbitrary edge point
        }
        let Some(point) = camera.project(Horizontal { azimuth: 0.0, altitude }.to_unit_vector()) else {
            continue;
        };
        if point.radius() > 1.0 + EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = viewport.to_cell_cartesian(point.clamp_to_edge());
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
