//! Replaceable stereographic bicubic Hermite warp. Four corners provide screen position, both first
//! derivatives and the mixed derivative. These 16 vector constraints define the entire polynomial patch.
//! All derivatives are analytic: tessellation evaluates the patch, not additional sky projections.
use crate::astro::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Data, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, project_camera};
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::timing::StepTimes;
use super::patch_warp::{CurvedPatch, project_curved_tiles};
use super::raster::Point;
use super::tile_warp::{contains_antipode, is_convex};

/// Select Hermite everywhere in stereographic views; the shared renderer retains the straight -e backend.
#[allow(dead_code)] // each experimental backend stays available by changing the call in projection/mod.rs
pub(super) fn project_hermite_tiles(
    frame: &mut Frame, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes,
) {
    // Compare patch formulas through the same tessellation, filtering, coverage and composition code.
    project_curved_tiles(frame, data, selected, observation, view, times, build_patch, "Hermite mesh preparation");
}

#[derive(Clone, Copy, Debug, Default)]
struct Corner {
    position: Point,
    along_u: Point,
    along_v: Point,
    mixed: Point,                           // change of along_u with v, over normalized tile coordinates
}

struct HermitePatch { corners: [Corner; 4] } // top-left, top-right, bottom-right, bottom-left

fn build_patch(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, sky: SkyArea) -> Option<HermitePatch> {
    // Evaluate the direction and analytic derivatives at just the four outer texture corners.
    let maximum = TILE_EDGE_TEXELS as f64 - 0.5;
    let coordinates = [[-0.5, -0.5], [maximum, -0.5], [maximum, maximum], [-0.5, maximum]];
    let mut directions = [Vector3::default(); 4];
    let mut corners = [Corner::default(); 4];
    let radius = (sky.size as f64 - 1.0) * 0.5;
    for (index, [column, row]) in coordinates.into_iter().enumerate() {
        let direction = observation.apparent_star_derivatives(tile.direction_derivatives(column, row))?;
        directions[index] = direction.value;
        let projected = project_camera(camera, direction.value)?;
        let denominator = 1.0 + camera.forward.dot(direction.value);
        let depth_u = camera.forward.dot(direction.along_u);
        let depth_v = camera.forward.dot(direction.along_v);
        let depth_mixed = camera.forward.dot(direction.mixed);

        // Differentiate the stereographic quotient for x and y, including the cross term.
        let values = [projected.x, projected.y];
        let axes = [camera.right, camera.up];
        let scales = [radius, -radius];
        let mut corner = Corner {
            position: [sky.x as f64 + radius + radius * projected.x, sky.y as f64 + radius - radius * projected.y],
            ..Corner::default()
        };
        for axis in 0..2 {
            let along_u = (camera.scale * axes[axis].dot(direction.along_u) - values[axis] * depth_u) / denominator;
            let along_v = (camera.scale * axes[axis].dot(direction.along_v) - values[axis] * depth_v) / denominator;
            let mixed = (camera.scale * axes[axis].dot(direction.mixed) - along_u * depth_v
                - along_v * depth_u - values[axis] * depth_mixed) / denominator;
            corner.along_u[axis] = along_u * scales[axis];
            corner.along_v[axis] = along_v * scales[axis];
            corner.mixed[axis] = mixed * scales[axis];
        }
        corners[index] = corner;
    }

    // Keep the same outline/singularity rules as Coons so changing the backend changes only the fitted surface.
    if contains_antipode(directions, camera.forward) || !is_convex(corners.map(|corner| corner.position)) { return None; }
    if corners.iter().any(|corner| {
        corner.along_u.into_iter().chain(corner.along_v).chain(corner.mixed).any(|value| !value.is_finite())
    }) { return None; }
    Some(HermitePatch { corners })
}

fn hermite_weights(fraction: f64) -> [f64; 4] {
    // The first two weights interpolate endpoint values; the last two interpolate endpoint derivatives.
    let squared = fraction * fraction;
    let cubed = squared * fraction;
    [2.0 * cubed - 3.0 * squared + 1.0, -2.0 * cubed + 3.0 * squared, cubed - 2.0 * squared + fraction, cubed - squared]
}

impl CurvedPatch for HermitePatch {
    fn evaluate(&self, u: f64, v: f64) -> Point {
        // Combine the one-dimensional bases into a tensor product over the tile square.
        let horizontal = hermite_weights(u);
        let vertical = hermite_weights(v);
        let mut position = [0.0; 2];
        for (corner, (u_end, v_end)) in self.corners.iter().zip([(0, 0), (1, 0), (1, 1), (0, 1)]) {
            for (axis, value) in position.iter_mut().enumerate() {
                *value += corner.position[axis] * horizontal[u_end] * vertical[v_end]
                    + corner.along_u[axis] * horizontal[u_end + 2] * vertical[v_end]
                    + corner.along_v[axis] * horizontal[u_end] * vertical[v_end + 2]
                    + corner.mixed[axis] * horizontal[u_end + 2] * vertical[v_end + 2];
            }
        }
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, Observer};
    use crate::constants::{CURVED_PATCH_MAX_SUBDIVISIONS, GRID_DEPTH};
    use crate::model::{Texel, ViewCenter, region_caps};
    use crate::observation::observe;
    use crate::projection::prepare_camera;
    use super::super::patch_warp::build_mesh;

    #[test]
    fn tensor_product_reproduces_a_polynomial_with_nonzero_mixed_derivatives() {
        // Both coordinates are bicubic polynomials. Zeroing mixed derivatives would fail the interior samples.
        let exact = |u: f64, v: f64| [10.0 * u + u * v + 2.0 * u * u * v * v * v, 10.0 * v + 3.0 * u * u * u * v * v];
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| Corner {
            position: exact(u, v),
            along_u: [10.0 + v + 4.0 * u * v * v * v, 9.0 * u * u * v * v],
            along_v: [u + 6.0 * u * u * v * v, 10.0 + 6.0 * u * u * u * v],
            mixed: [1.0 + 12.0 * u * v * v, 18.0 * u * u * v],
        });
        let patch = HermitePatch { corners };
        for u in [0.0, 0.2, 0.5, 0.9, 1.0] {
            for v in [0.0, 0.3, 0.5, 0.7, 1.0] {
                let expected = exact(u, v);
                let actual = patch.evaluate(u, v);
                assert!((actual[0] - expected[0]).abs() < 1e-12 && (actual[1] - expected[1]).abs() < 1e-12);
            }
        }
    }

    fn fixture(fov: f64, atmosphere: bool) -> (View, Observation, CartesianCamera, SkyArea) {
        // Match the screenshot's camera and date, including the extreme zoom-out view.
        let site = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, ..View::default() };
        let time = FrameTime::from_utc(crate::astro::datetime_to_julian_date(&crate::astro::parse_utc_datetime("2025-03-01T11:00:00").unwrap()));
        let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
        (view, observation, prepare_camera(&view), SkyArea::centred(1917, 1116))
    }

    fn exact_position(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, sky: SkyArea, u: f64, v: f64) -> Point {
        // Test oracle: evaluate the real mapping at an arbitrary tile position, never used by the renderer.
        let direction = tile.direction_of(u * TILE_EDGE_TEXELS as f64 - 0.5, v * TILE_EDGE_TEXELS as f64 - 0.5);
        let projected = project_camera(camera, observation.apparent_star_direction(direction)).unwrap();
        let radius = (sky.size as f64 - 1.0) * 0.5;
        [sky.x as f64 + radius + radius * projected.x, sky.y as f64 + radius - radius * projected.y]
    }

    #[test]
    fn analytic_mixed_derivatives_match_differences_of_the_real_mapping() {
        // Include directions near the extreme view's rim as well as the center, with and without refraction.
        for atmosphere in [false, true] {
            let (_, observation, camera, sky) = fixture(351.6, atmosphere);
            for offset in [0.0_f64, 60.0, 150.0, 175.0] {
                let direction = observation.to_catalog.apply(camera.forward * offset.to_radians().cos() + camera.up * offset.to_radians().sin());
                let region = crate::model::hash_direction(GRID_DEPTH, direction);
                let tile = TileGeometry::new(region);
                let patch = build_patch(&tile, &observation, camera, sky).unwrap();
                let step = 1e-3;
                for (index, [u, v]) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].into_iter().enumerate() {
                    let pp = exact_position(&tile, &observation, camera, sky, u + step, v + step);
                    let pm = exact_position(&tile, &observation, camera, sky, u + step, v - step);
                    let mp = exact_position(&tile, &observation, camera, sky, u - step, v + step);
                    let mm = exact_position(&tile, &observation, camera, sky, u - step, v - step);
                    for axis in 0..2 {
                        let numerical = (pp[axis] - pm[axis] - mp[axis] + mm[axis]) / (4.0 * step * step);
                        let analytic = patch.corners[index].mixed[axis];
                        assert!((numerical - analytic).abs() < 2e-3 * analytic.abs().max(1.0), "atmosphere={atmosphere}, angle={offset}: {analytic} vs {numerical}");
                    }
                }
            }
        }
    }

    #[test]
    fn normal_views_match_real_positions_and_preserve_coons_boundaries() {
        // Smooth, unrefracted mapping must stay subpixel at normal zoom. Refraction clamp crossings are
        // measured separately: a smooth polynomial cannot exactly reproduce a derivative discontinuity.
        for fov in [10.0, 60.0, 180.0] {
            for atmosphere in [false, true] {
                let (_, observation, camera, sky) = fixture(fov, atmosphere);
                let region = crate::model::hash_direction(GRID_DEPTH, observation.patch.center);
                let tile = TileGeometry::new(region);
                let patch = build_patch(&tile, &observation, camera, sky).unwrap();
                let coons = super::super::coons_warp::build_patch(&tile, &observation, camera, sky).unwrap();
                let mut worst: f64 = 0.0;
                for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let expected = exact_position(&tile, &observation, camera, sky, u, v);
                        let actual = patch.evaluate(u, v);
                        assert!(actual.into_iter().all(f64::is_finite));
                        worst = worst.max((expected[0] - actual[0]).hypot(expected[1] - actual[1]));
                        if u == 0.0 || u == 1.0 || v == 0.0 || v == 1.0 {
                            let boundary = coons.evaluate(u, v);
                            assert!((boundary[0] - actual[0]).hypot(boundary[1] - actual[1]) < 1e-8);
                        }
                    }
                }
                if !atmosphere { assert!(worst < 0.25); }
                eprintln!("Hermite normal fov={fov} atmosphere={atmosphere}: worst fitted error={worst:.6}px");
            }
        }
    }

    #[test]
    fn extreme_view_has_bounded_meshes_and_finite_output() {
        // Exercise every geometric region at 351.6 degrees, recording approximation errors rather than claiming
        // that four-corner patches solve the singularity.
        for atmosphere in [false, true] {
            let (_, observation, camera, sky) = fixture(351.6, atmosphere);
            let mut mesh = Vec::new();
            let (mut rejected, mut worst_error, mut checked) = (0, 0.0_f64, 0);
            let mut coons_worst_error: f64 = 0.0;
            let mut shared_checked = 0;
            let mut shared_hermite_worst: f64 = 0.0;
            for region in 0..region_caps().len() {
                let tile = TileGeometry::new(region);
                let Some(patch) = build_patch(&tile, &observation, camera, sky) else { rejected += 1; continue; };
                if !build_mesh(&patch, &mut mesh) { rejected += 1; continue; }
                assert!(mesh.len() <= 2 * CURVED_PATCH_MAX_SUBDIVISIONS.pow(2));
                let coons = super::super::coons_warp::build_patch(&tile, &observation, camera, sky);
                for u in [0.25, 0.5, 0.75] {
                    for v in [0.25, 0.5, 0.75] {
                        let expected = exact_position(&tile, &observation, camera, sky, u, v);
                        let actual = patch.evaluate(u, v);
                        assert!(actual.into_iter().all(f64::is_finite));
                        let radius = (sky.size as f64 - 1.0) * 0.5;
                        if (expected[0] - sky.x as f64 - radius).hypot(expected[1] - sky.y as f64 - radius) <= radius {
                            checked += 1;
                            let error = (expected[0] - actual[0]).hypot(expected[1] - actual[1]);
                            worst_error = worst_error.max(error);
                            if let Some(coons) = &coons {
                                let other = coons.evaluate(u, v);
                                coons_worst_error = coons_worst_error.max((expected[0] - other[0]).hypot(expected[1] - other[1]));
                                shared_hermite_worst = shared_hermite_worst.max(error);
                                shared_checked += 1;
                            }
                        }
                    }
                }
            }
            assert!(checked > 50_000 && rejected < 100);
            eprintln!("Hermite 351.6deg atmosphere={atmosphere}: rejected regions={rejected}; visible probes={checked}; worst fitted error={worst_error:.3}px");

            eprintln!("atmosphere={atmosphere}; shared probes={shared_checked}: Coons worst={coons_worst_error:.3}px; Hermite worst={shared_hermite_worst:.3}px (fitted positions, before mesh approximation)");

        }

        let (_, observation, _, _) = fixture(351.6, false);
        // The public backend must clear degenerate and empty frames, and keep -e on the prior renderer.
        let data = crate::model::Data::default();
        let selected = Selected { regions: vec![], whole_sky: false };
        for size in [0, 1, 9] {
            let mut frame = Frame::square(size);
            frame.stars_mut().fill(Texel { rgb: [0.5; 3], opacity: 0.5 });
            project_hermite_tiles(&mut frame, &data, &selected, &observation, &View::default(), &mut StepTimes::default());
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }
    #[test]
    fn lit_tiles_render_safely_at_the_extreme_rim_and_equidistant_is_unchanged() {
        // Uniform translucent tiles isolate the geometry and detect double coverage or invalid channel values.
        let (view, observation, camera, _) = fixture(351.6, true);
        let mut data = Data::default();
        let mut regions = Vec::new();
        for angle in [0.0_f64, 170.0, 175.0] {
            let direction = observation.to_catalog.apply(camera.forward * angle.to_radians().cos() + camera.up * angle.to_radians().sin());
            let region = crate::model::hash_direction(GRID_DEPTH, direction);
            let image = &mut data.star_regions[region].merged_tile;
            image.tile.texels = vec![Texel { rgb: [0.1, 0.2, 0.3], opacity: 0.4 }; TILE_EDGE_TEXELS.pow(2)];
            image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
            regions.push(region);
        }
        let selected = Selected { regions, whole_sky: false };
        let mut frame = Frame::square(201);
        project_hermite_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
        assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0
            && texel.rgb.iter().all(|&value| value.is_finite() && value >= 0.0 && value <= texel.opacity)));

        // Changing the experimental backend must not change the equidistant rendering path.
        let equidistant = View { projection: crate::model::ProjectionKind::Equidistant, ..view };
        project_hermite_tiles(&mut frame, &data, &selected, &observation, &equidistant, &mut StepTimes::default());
        let expected = frame.stars().to_vec();
        super::super::tile_warp::project_whole_tiles(&mut frame, &data, &selected, &observation, &equidistant, &mut StepTimes::default());
        assert_eq!(frame.stars(), expected);
    }

}
