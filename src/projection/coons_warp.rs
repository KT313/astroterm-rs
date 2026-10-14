//! Stereographic Coons-patch prototype. Four projected corners and their analytic tangents define four cubic
//! Hermite boundary curves. The Coons blend fills the interior; it is an approximation of the sky mapping.
//! Only the patch polynomial is evaluated during subdivision, never extra sky positions or screen rays.
//! Replace the call in projection/mod.rs with tile_warp::project_whole_tiles to restore the straight warp.
use crate::astro::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Data, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, project_camera};
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::timing::StepTimes;
use super::raster::Point;
use super::patch_warp::{CurvedPatch, project_curved_tiles};
use super::tile_warp::{contains_antipode, is_convex};

/// Select the Coons fit while reusing the common forward mesh renderer.
#[allow(dead_code)] // retained as a one-call alternative to the selected Hermite backend
pub(super) fn project_coons_tiles(
    frame: &mut Frame, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes,
) {
    // The backend controls the fitted surface; coverage and region composition are shared for comparison.
    project_curved_tiles(frame, data, selected, observation, view, times, build_patch, "Projection.Coons mesh preparation");
}

#[derive(Clone, Copy, Debug)]
struct Corner {
    position: Point,
    along_u: Point,                         // screen change across one full tile width
    along_v: Point,                         // screen change across one full tile height
}

pub(super) struct CoonsPatch { corners: [Corner; 4] }  // top-left, top-right, bottom-right, bottom-left

/// Fit the boundary curves from values and tangents at exactly four tile positions.
pub(super) fn build_patch(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, sky: SkyArea) -> Option<CoonsPatch> {
    // Get the apparent directions and their derivatives at the four outer texture corners.
    let maximum = TILE_EDGE_TEXELS as f64 - 0.5;
    let coordinates = [[-0.5, -0.5], [maximum, -0.5], [maximum, maximum], [-0.5, maximum]];
    let mut directions = [Vector3::default(); 4];
    let mut derivatives = [[Vector3::default(); 2]; 4];
    for index in 0..4 {
        let (direction, tangents) = tile.direction_tangents(coordinates[index][0], coordinates[index][1]);
        (directions[index], derivatives[index]) = observation.apparent_star_tangents(direction, tangents)?;
    }
    if contains_antipode(directions, camera.forward) { return None; }

    // Project each corner once; differentiate the stereographic divide in the two tile directions.
    let radius = (sky.size as f64 - 1.0) * 0.5;
    let empty = Corner { position: [0.0; 2], along_u: [0.0; 2], along_v: [0.0; 2] };
    let mut corners = [empty; 4];
    for index in 0..4 {
        let direction = directions[index];
        let projected = project_camera(camera, direction)?;
        let denominator = 1.0 + camera.forward.dot(direction);
        let tangents = derivatives[index].map(|tangent| {
            let depth_change = camera.forward.dot(tangent);
            [
                radius * (camera.scale * camera.right.dot(tangent) - projected.x * depth_change) / denominator,
                -radius * (camera.scale * camera.up.dot(tangent) - projected.y * depth_change) / denominator,
            ]
        });
        corners[index] = Corner {
            position: [sky.x as f64 + radius + radius * projected.x, sky.y as f64 + radius - radius * projected.y],
            along_u: tangents[0],
            along_v: tangents[1],
        };
    }

    // Reject singular or folded outlines rather than drawing a giant or inverted patch.
    if !is_convex(corners.map(|corner| corner.position)) { return None; }
    if corners.iter().any(|corner| corner.along_u.into_iter().chain(corner.along_v).any(|value| !value.is_finite())) { return None; }
    Some(CoonsPatch { corners })
}

/// Position on a cubic edge whose endpoint derivatives are measured over its complete 0..1 interval.
fn evaluate_edge(start: Point, end: Point, start_tangent: Point, end_tangent: Point, fraction: f64) -> Point {
    // Hermite weights preserve both endpoints and the direction and speed of travel at those endpoints.
    let squared = fraction * fraction;
    let cubed = squared * fraction;
    let weights = [2.0 * cubed - 3.0 * squared + 1.0, -2.0 * cubed + 3.0 * squared, cubed - 2.0 * squared + fraction, cubed - squared];
    std::array::from_fn(|axis| {
        start[axis] * weights[0] + end[axis] * weights[1] + start_tangent[axis] * weights[2] + end_tangent[axis] * weights[3]
    })
}

impl CurvedPatch for CoonsPatch {
    /// Blend opposite curved edges, subtracting the bilinear corner interpolation counted in both blends.
    fn evaluate(&self, u: f64, v: f64) -> Point {
        // Evaluate the top, bottom, left and right boundaries at the matching tile coordinates.
        let [a, b, c, d] = self.corners;
        let top = evaluate_edge(a.position, b.position, a.along_u, b.along_u, u);
        let bottom = evaluate_edge(d.position, c.position, d.along_u, c.along_u, u);
        let left = evaluate_edge(a.position, d.position, a.along_v, d.along_v, v);
        let right = evaluate_edge(b.position, c.position, b.along_v, c.along_v, v);

        // Fill the interior while reproducing each supplied boundary exactly.
        std::array::from_fn(|axis| {
            let straight_top = (1.0 - u) * a.position[axis] + u * b.position[axis];
            let straight_bottom = (1.0 - u) * d.position[axis] + u * c.position[axis];
            (1.0 - v) * top[axis] + v * bottom[axis] + (1.0 - u) * left[axis] + u * right[axis]
                - ((1.0 - v) * straight_top + v * straight_bottom)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::CURVED_PATCH_MAX_SUBDIVISIONS;
    use crate::model::{ProjectionKind, Texel};
    use crate::projection::prepare_camera;
    use super::super::patch_warp::{build_mesh, RegionPixels, clip_pixel};
    use super::super::tile_sampling::TileSampler;
    use crate::astro::{FrameTime, J2000, Observer};
    use crate::model::{RenderOptions, ViewCenter, fixtures, hash_direction};
    use crate::observation::observe;
    use crate::regions::refresh_regions;

    fn flat_patch(corners: [Point; 4]) -> CoonsPatch {
        // Derivatives of a bilinear quadrilateral reproduce its straight edges exactly.
        let difference = |first: usize, second: usize| std::array::from_fn(|axis| corners[second][axis] - corners[first][axis]);
        CoonsPatch { corners: [
            Corner { position: corners[0], along_u: difference(0, 1), along_v: difference(0, 3) },
            Corner { position: corners[1], along_u: difference(0, 1), along_v: difference(1, 2) },
            Corner { position: corners[2], along_u: difference(3, 2), along_v: difference(1, 2) },
            Corner { position: corners[3], along_u: difference(3, 2), along_v: difference(0, 3) },
        ] }
    }

    fn paint(patch: &CoonsPatch, frame: &mut [Texel], texels: &[Texel]) {
        // Exercise the real mesh, filter, coverage and one-blend-per-region path on a small screen.
        let mut mesh = Vec::new();
        assert!(build_mesh(patch, &mut mesh));
        let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
        sampler.prepare(&super::super::tile_sampling::lit_rows_of(texels, TILE_EDGE_TEXELS));
        let mut pixels = RegionPixels::default();
        pixels.prepare(&mesh, 9, 9, SkyArea { x: 0, y: 0, size: 9 });
        for triangle in &mesh { pixels.draw_triangle(triangle, &sampler, texels); }
        pixels.composite(frame, 9);
    }

    #[test]
    fn coons_matches_curved_edges_and_reduces_to_a_rectangle_without_bending() {
        // A straight 10-pixel square must remain a straight square at every interior sample.
        let mut patch = flat_patch([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        for u in [0.0, 0.2, 0.5, 1.0] {
            for v in [0.0, 0.3, 0.5, 1.0] {
                let actual = patch.evaluate(u, v);
                assert!((actual[0] - 10.0 * u).abs() < 1e-12 && (actual[1] - 10.0 * v).abs() < 1e-12);
            }
        }

        // Bend only the top boundary upward; the patch must follow the curve and distribute its bend inside.
        patch.corners[0].along_u[1] = -8.0;
        patch.corners[1].along_u[1] = 8.0;
        assert_eq!(patch.evaluate(0.5, 0.0), [5.0, -2.0]);
        assert_eq!(patch.evaluate(0.5, 0.5), [5.0, 4.0]);
        assert_eq!(patch.evaluate(0.5, 1.0), [5.0, 10.0]);
        let mut mesh = Vec::new();
        assert!(build_mesh(&patch, &mut mesh));
        assert!(mesh.len() > 2 && mesh.len() <= 2 * CURVED_PATCH_MAX_SUBDIVISIONS.pow(2));

        // Equal endpoint slopes make an S-curve with zero midpoint error, but still require subdivision.
        patch.corners[1].along_u[1] = -8.0;
        assert_eq!(patch.evaluate(0.5, 0.0), [5.0, 0.0]);
        assert!(build_mesh(&patch, &mut mesh));
        assert!(mesh.len() > 2);
    }

    #[test]
    fn thin_coverage_diagonals_and_region_blending_keep_light() {
        // Even a strip between the old sub-row sample lines must contribute its 4% area.
        let patch = flat_patch([[3.8, 3.95], [4.2, 3.95], [4.2, 4.05], [3.8, 4.05]]);
        let texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        let mut frame = vec![Texel::default(); 81];
        paint(&patch, &mut frame, &texels);
        assert!((frame[40].opacity - 0.04).abs() < 1e-6);

        // A horizontal diagonal in a non-parallelogram must read the lower triangle's part of the texture.
        let patch = flat_patch([[2.0, 4.0], [4.0, 2.0], [6.0, 4.0], [4.0, 7.0]]);
        let mut mesh = Vec::new();
        assert!(build_mesh(&patch, &mut mesh));
        assert!(mesh.iter().any(|triangle| clip_pixel(triangle.screen, [4.0, 6.0]).is_some()));

        // Two overlapping region images blend once each, without a dark internal mesh seam.
        let mut patch = flat_patch([[2.5, 2.5], [5.5, 2.5], [5.5, 5.5], [2.5, 5.5]]);
        patch.corners[0].along_u[1] = -4.0;
        patch.corners[1].along_u[1] = 4.0;
        frame.fill(Texel::default());
        for color in [[0.5, 0.0, 0.0], [0.0, 0.0, 0.5]] {
            paint(&patch, &mut frame, &vec![Texel { rgb: color, opacity: 0.5 }; TILE_EDGE_TEXELS.pow(2)]);
        }
        assert_eq!(frame[40], Texel { rgb: [0.25, 0.0, 0.5], opacity: 0.75 });
        assert!(frame.iter().all(|texel| texel.opacity <= 0.75 + 1e-6));
    }

    #[test]
    fn reduced_tiles_keep_off_center_texels_and_both_windings() {
        // The whole texture fits into one screen pixel; its top-left star must survive area filtering.
        let corners = [[3.5, 3.5], [4.5, 3.5], [4.5, 4.5], [3.5, 4.5]];
        let mut texels = vec![Texel::default(); TILE_EDGE_TEXELS.pow(2)];
        texels[0] = Texel { rgb: [1.0; 3], opacity: 1.0 };
        for order in [[0, 1, 2, 3], [0, 3, 2, 1]] {
            let mut frame = vec![Texel::default(); 81];
            paint(&flat_patch(order.map(|index| corners[index])), &mut frame, &texels);
            assert!((frame[40].opacity - 1.0 / TILE_EDGE_TEXELS.pow(2) as f32).abs() < 1e-7);
        }
    }

    #[test]
    fn corner_tangents_match_the_full_formula_and_patches_follow_representative_views() {
        // Compare analytic derivatives against independent finite differences used only by this test.
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        let time = FrameTime::from_utc(J2000);
        let sky = SkyArea { x: 0, y: 0, size: 1101 };
        for atmosphere in [false, true] {
            for fov in [10.0, 60.0, 180.0] {
                let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.2 }, projection: ProjectionKind::Stereographic, fov_degrees: fov };
                let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
                let camera = prepare_camera(&view);
                let region = hash_direction(crate::constants::GRID_DEPTH, observation.patch.center);
                let tile = TileGeometry::new(region);
                let patch = build_patch(&tile, &observation, camera, sky).unwrap();
                let exact = |u: f64, v: f64| {
                    let direction = tile.direction_of(u * TILE_EDGE_TEXELS as f64 - 0.5, v * TILE_EDGE_TEXELS as f64 - 0.5);
                    let projected = project_camera(camera, observation.apparent_star_direction(direction)).unwrap();
                    [550.0 + 550.0 * projected.x, 550.0 - 550.0 * projected.y]
                };
                for (index, [u, v]) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].into_iter().enumerate() {
                    let delta = 1e-5;
                    for axis in 0..2 {
                        let (before, after) = if axis == 0 { (exact(u - delta, v), exact(u + delta, v)) } else { (exact(u, v - delta), exact(u, v + delta)) };
                        let tangent = if axis == 0 { patch.corners[index].along_u } else { patch.corners[index].along_v };
                        for coordinate in 0..2 { assert!(((after[coordinate] - before[coordinate]) / (2.0 * delta) - tangent[coordinate]).abs() < 1e-5); }
                    }
                }
                for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let actual = patch.evaluate(u, v);
                        let expected = exact(u, v);
                        assert!((actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 0.25, "{fov} atmosphere={atmosphere}: {actual:?} != {expected:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn pipeline_places_stars_clears_empty_views_and_preserves_equidistant() {
        // Render one real simulated star in each mode; the equidistant branch must be byte-for-byte unchanged.
        let time = FrameTime::from_utc(J2000);
        for projection in [ProjectionKind::Stereographic, ProjectionKind::Equidistant] {
            let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.2 }, projection, fov_degrees: 60.0 };
            let observation = observe(&Observer::default(), true, &view, time, &mut StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(crate::constants::GRID_DEPTH, direction);
            let mut data = fixtures::layout(&[(region, 0, fixtures::row(direction, 0.0, 3.0, 1))]);
            let selected = Selected { regions: vec![region], whole_sky: false };
            let settings = RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
            refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::square(201);
            project_coons_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
            assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));
            if projection == ProjectionKind::Equidistant {
                let expected = frame.stars().to_vec();
                super::super::tile_warp::project_whole_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                assert_eq!(frame.stars(), expected);
            }
            project_coons_tiles(&mut frame, &data, &Selected { regions: Vec::new(), whole_sky: false }, &observation, &view, &mut StepTimes::default());
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }
    #[test]
    #[ignore = "timing only; compares the three forward backends using embedded and synthetic tiles"]
    fn compare_coons_mapping() {
        // Prepare a real small catalog once; the dense variant fills the same selected tiles synthetically.
        let mut report = crate::progress::Report::quiet();
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut report).unwrap(), &mut report).unwrap();
        let mut data = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        let time = FrameTime::from_utc(J2000);
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: 180.0, ..View::default() };
        let observation = observe(&Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() }, true, &view, time, &mut StepTimes::default());
        let selected = crate::selection::select_regions(&data.star_regions, &observation, &mut StepTimes::default());
        let settings = RenderOptions { magnitude_threshold: 9.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
        refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
        let mut frame = Frame::new(1920, 1100, SkyArea::centred(1920, 1100), (10, 20));

        for dense in [false, true] {
            if dense {
                for &region in &selected.regions {
                    let image = &mut data.star_regions[region].merged_tile;
                    image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS * TILE_EDGE_TEXELS];
                    image.lit_rows = vec![u64::MAX >> (64 - TILE_EDGE_TEXELS); TILE_EDGE_TEXELS];
                }
            }

            // Alternate backend order and exclude the first group from the five-frame averages.
            let mut elapsed = [0.0; 3];
            for run in 0..6 {
                for backend in if run % 2 == 0 { [0, 1, 2] } else { [2, 1, 0] } {
                    let start = std::time::Instant::now();
                    if backend == 0 {
                        super::super::tile_warp::project_whole_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                    } else if backend == 1 {
                        project_coons_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                    } else {
                        super::super::hermite_warp::project_hermite_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                    }
                    if run > 0 { elapsed[backend] += start.elapsed().as_secs_f64(); }
                    std::hint::black_box(frame.stars());
                }
            }
            eprintln!("dense={dense}; selected={}; straight_ms={:.3}; coons_ms={:.3}; hermite_ms={:.3}", selected.regions.len(), elapsed[0] * 200.0, elapsed[1] * 200.0, elapsed[2] * 200.0);
        }
    }
}
