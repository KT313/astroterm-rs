//! Camera geometry only: maps an immutable observed sky to screen points and clipped segments. No simulation
//! evaluation or astronomical corrections occur here. Character rendering consumes these prepared primitives.
use crate::model::{CartesianCamera, Polar, ScreenPoint, View, ViewCenter};
use super::draw_order::prepare_draw_order;
use crate::astro::{Horizontal, Vector3, offset_vector_towards};
use crate::model::ObservedSky;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const EDGE_TOLERANCE: f64 = 1e-6;
use crate::model::{
    Cell, ProjectedArc, ProjectedConstellation, ProjectedMoon, ProjectedPlanet, ProjectionViewport as Viewport,
    ProjectionViewport,
};

/// Map Cartesian projection coordinates to signed row/column cells.
pub fn project_to_cell(viewport: ProjectionViewport, point: ScreenPoint) -> Cell {
    let snap = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
    let (ry, rx) = ((viewport.height as f64 - 1.0) / 2.0, (viewport.width as f64 - 1.0) / 2.0);
    (
        (-snap(point.y) * ry + ry).round() as i32,
        (snap(point.x) * rx + rx).round() as i32,
    )
}

/// Map polar projection coordinates to signed row/column cells.
pub fn polar_to_cell(viewport: ProjectionViewport, polar: Polar) -> Cell {
    let radius_y = (viewport.height as f64 - 1.0) / 2.0;
    let radius_x = (viewport.width as f64 - 1.0) / 2.0;
    let snap = |value: f64| if value.abs() < 1e-12 { 0.0 } else { value };
    let (s, c) = (snap(polar.theta.sin()), snap(polar.theta.cos()));
    (
        (polar.radius * -radius_y * s + radius_y).round() as i32,
        (polar.radius * radius_x * c + radius_x).round() as i32,
    )
}

/// Project without retaining timing diagnostics (reference fixtures and library callers).
pub fn project_sky(sky: &ObservedSky, view: &View, viewport: Viewport) -> crate::model::ProjectionData {
    project_sky_with_times(sky, view, viewport, &mut crate::timing::StepTimes::default())
}

/// Camera-stage coordinator. Exact visibility and draw order stay separate from observation's conservative filters.
pub fn project_sky_with_times(
    sky: &ObservedSky,
    view: &View,
    viewport: Viewport,
    times: &mut crate::timing::StepTimes,
) -> crate::model::ProjectionData {
    let camera = crate::projection::prepare_camera(view);
    let stars = times.measure("Star projection", || project_visible_stars(sky, &camera, viewport));
    let order = times.measure("Star draw order", || {
        let mut records = Vec::new();
        prepare_draw_order(&mut records, stars.iter().map(|&(index, _)| {
            let star = sky.star_view(index);
            (star.magnitude, star.id())
        }));
        records.into_iter().map(|record| record.projected_index).collect()
    });
    let (planets, moon) = times.measure("Body projection", || project_bodies(sky, view, &camera, viewport));
    let constellations = times.measure("Constellation projection", || {
        project_constellations(sky.constellations(), &sky.stars, sky.magnitude_threshold, view, viewport)
    });
    let (horizon, horizon_labels) = times.measure("Horizon projection", || {
        (
            project_horizon_line(view, viewport),
            project_horizon_labels(view, viewport),
        )
    });
    crate::model::ProjectionData {
        outside_accuracy_range: sky.outside_accuracy_range,
        selection: sky.selection,
        evaluated_stars: sky.corrections.evaluated,
        correction_stats: sky.corrections,
        catalog_singular_count: sky.catalog.singular_count,
        runtime_singular_count: sky.runtime_singular_count,
        stars,
        order,
        planets,
        moon,
        constellations,
        facing: view.is_facing(),
        viewport,
        horizon,
        horizon_labels,
    }
}

pub(super) fn project_visible_cell(camera: &CartesianCamera, viewport: Viewport, position: Vector3) -> Option<Cell> {
    crate::projection::project_camera(*camera, position)
        .filter(|p| p.is_visible())
        .map(|p| crate::projection::project_to_cell(viewport, p))
}

fn project_visible_stars(sky: &ObservedSky, camera: &CartesianCamera, viewport: Viewport) -> Vec<(usize, Cell)> {
    sky.star_views().enumerate().filter(|(_, star)| star.drawable).filter_map(|(index, star)| {
        project_visible_cell(camera, viewport, star.position).map(|cell| (index, cell))
    }).collect()
}

pub(in crate::projection) fn project_bodies(
    sky: &ObservedSky,
    view: &View,
    camera: &CartesianCamera,
    viewport: Viewport,
) -> (Vec<ProjectedPlanet>, ProjectedMoon) {
    let cell = |position| project_visible_cell(camera, viewport, position);
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
        light_direction: project_light_direction(view, sky.moon.position, sky.sun().position),
    };
    (planets, moon)
}

pub(in crate::projection) fn project_constellations(
    figures: &[crate::model::Constellation],
    stars: &[crate::model::ObservedStar],
    magnitude_threshold: f64,
    view: &View,
    viewport: Viewport,
) -> Vec<ProjectedConstellation> {
    let find_star = |index| {
        stars
            .binary_search_by_key(&index, |star| star.source_index)
            .ok()
            .map(|i| &stars[i])
    };
    figures
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
            if maximum_magnitude > magnitude_threshold {
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
        .collect()
}

/// Screen direction towards the Sun, shared by lunar raster lighting and character glyph orientation.
pub fn project_light_direction(view: &View, moon: Vector3, sun: Vector3) -> Option<ScreenPoint> {
    let camera = crate::projection::prepare_camera(view);
    let offset = offset_vector_towards(moon, sun, 1_f64.to_radians());
    let (a, b) = (crate::projection::project_camera(camera, moon)?, crate::projection::project_camera(camera, offset)?);
    let (x, y) = (b.x - a.x, b.y - a.y);
    let length = x.hypot(y);
    (length.is_finite() && length > 0.0).then(|| ScreenPoint {
        x: x / length,
        y: y / length,
    })
}

/// Sample clipped great-circle arcs with at most a quarter-viewport-unit midpoint deviation. Both renderers
/// consume this geometry; endpoints keep their star markers, while inserted vertices never create markers.
pub fn project_constellation_segment(view: &View, viewport: Viewport, from: Vector3, to: Vector3) -> Vec<ProjectedArc> {
    let mut arcs = Vec::new();
    let camera = crate::projection::prepare_camera(view);
    let project = |angle| {
        crate::projection::project_camera(camera, offset_vector_towards(from, to, angle))
            .map(ScreenPoint::clamp_to_edge)
    };
    for part in crate::projection::find_visible_arc_parts_vectors(view, from, to) {
        let (Some(start), Some(end)) = (project(part.start), project(part.end)) else {
            continue;
        };
        let mut points = vec![crate::projection::project_to_cell(viewport, start)];
        sample_arc(&project, viewport, (part.start, start), (part.end, end), 0, &mut points);
        let start = crate::projection::project_to_cell(viewport, start);
        let end = crate::projection::project_to_cell(viewport, end);
        points.dedup();
        if points.len() == 1 {
            points.push(end);
        }
        arcs.push(ProjectedArc {
            start,
            end,
            points,
            includes_start: part.includes_start,
            includes_end: part.includes_end,
        });
    }
    arcs
}

fn sample_arc(
    project: &impl Fn(f64) -> Option<ScreenPoint>,
    viewport: Viewport,
    start: (f64, ScreenPoint),
    end: (f64, ScreenPoint),
    depth: u8,
    points: &mut Vec<Cell>,
) {
    let angle = (start.0 + end.0) * 0.5;
    if let Some(mid) = project(angle) {
        let error_x = (mid.x - (start.1.x + end.1.x) * 0.5) * viewport.width.saturating_sub(1) as f64 * 0.5;
        let error_y = (mid.y - (start.1.y + end.1.y) * 0.5) * viewport.height.saturating_sub(1) as f64 * 0.5;
        if depth < 12 && (error_x.hypot(error_y) > 0.25 || end.0 - start.0 > 10_f64.to_radians()) {
            sample_arc(project, viewport, start, (angle, mid), depth + 1, points);
            sample_arc(project, viewport, (angle, mid), end, depth + 1, points);
            return;
        }
    }
    points.push(crate::projection::project_to_cell(viewport, end.1));
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
    let camera = crate::projection::prepare_camera(view);
    let project_horizon = |azimuth: f64| crate::projection::project_camera(camera, Horizontal { azimuth, altitude: 0.0 }.to_unit_vector());

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
                *segment_start.get_or_insert_with(|| crate::projection::project_to_cell(viewport, previous_point.clamp_to_edge()));
            let end = crate::projection::project_to_cell(viewport, current_point.clamp_to_edge());
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
    let camera = crate::projection::prepare_camera(view);
    const DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

    // the 8 main directions, except on the very edge where labels get cut off
    for (index, label) in DIRECTIONS.iter().enumerate() {
        let azimuth = index as f64 * TAU / DIRECTIONS.len() as f64;
        let Some(point) = crate::projection::project_camera(camera, Horizontal { azimuth, altitude: 0.0 }.to_unit_vector()) else {
            continue;
        };
        if point.radius() >= 1.0 - EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = crate::projection::project_to_cell(viewport, point);
        labels.push(((row, col - (label.len() as i32 - 1) / 2), *label));
    }

    // zenith and nadir, edge included
    for (label, altitude) in [("Zenith", FRAC_PI_2), ("Nadir", -FRAC_PI_2)] {
        if (altitude + view.tilt()).abs() < EDGE_TOLERANCE {
            continue; // directly behind the view (tilt ±90° at fov 360°), it would sit on an arbitrary edge point
        }
        let Some(point) = crate::projection::project_camera(camera, Horizontal { azimuth: 0.0, altitude }.to_unit_vector()) else {
            continue;
        };
        if point.radius() > 1.0 + EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = crate::projection::project_to_cell(viewport, point.clamp_to_edge());
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

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn sampled_arc_follows_the_great_circle_midpoint_instead_of_its_chord() {
        let view = View::default();
        let viewport = Viewport {
            height: 801,
            width: 801,
        };
        let direction = |azimuth: f64| {
            Horizontal {
                azimuth: azimuth.to_radians(),
                altitude: 20_f64.to_radians(),
            }
            .to_unit_vector()
        };
        let (a, b) = (direction(30.0), direction(150.0));
        let arcs = project_constellation_segment(&view, viewport, a, b);
        assert_eq!(arcs.len(), 1);
        let arc = &arcs[0];
        assert!(arc.includes_start && arc.includes_end);
        assert!(arc.points.len() > 2);
        let midpoint = crate::projection::project_to_cell(viewport, crate::projection::project_camera(crate::projection::prepare_camera(&view), (a + b).normalized()).unwrap());
        assert!(
            arc.points
                .iter()
                .any(|p| (p.0 - midpoint.0).abs() <= 1 && (p.1 - midpoint.1).abs() <= 1)
        );
        let chord_midpoint = ((arc.start.0 + arc.end.0) / 2, (arc.start.1 + arc.end.1) / 2);
        assert!((midpoint.0 - chord_midpoint.0).abs() + (midpoint.1 - chord_midpoint.1).abs() > 20);
    }

    #[test]
    fn lunar_light_direction_keeps_screen_sign_and_unit_length() {
        let moon = Horizontal {
            azimuth: PI,
            altitude: 0.5,
        }
        .to_unit_vector();
        let sun = Horizontal {
            azimuth: PI + 0.7,
            altitude: 0.5,
        }
        .to_unit_vector();
        let direction = project_light_direction(&View::default(), moon, sun).unwrap();
        assert!(direction.x > 0.0);
        assert!((direction.x.hypot(direction.y) - 1.0).abs() < 1e-12);
    }
}
