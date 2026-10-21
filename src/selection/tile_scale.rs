//! How large tiles appear on the screen: a cell's tile texels per screen pixel at the cell centre, the cheap
//! estimate that decides how far a region splits. Tile and screen are both azimuthal maps of the sky, so the ratio
//! is the tile's texels per radian over the screen's pixels per radian there: one dot product per cell.
use std::f64::consts::FRAC_PI_2;
use crate::formulas::Vector3;
use crate::model::{ProjectionKind, SkyCell, View};
use crate::observation::Observation;
use crate::projection::{prepare_camera, rotate_camera_into};
use crate::regions::TileGeometry;

/// The screen's side of the estimate for one frame: where the view looks and how many pixels a radian spans
/// before the projection's local factor.
#[derive(Clone, Copy, Debug)]
pub struct TileScale {
    view_centre: Vector3,                   // unit, catalog axes
    pixel_scale: f64,                       // disc radius in pixels × camera scale
    kind: ProjectionKind,
}

impl TileScale {
    /// The scale of a drawing square `size` pixels wide showing `view`.
    pub fn new(observation: &Observation, view: &View, size: usize) -> Self {
        let camera = rotate_camera_into(prepare_camera(view), observation.to_horizon);
        Self { view_centre: camera.forward, pixel_scale: (size as f64 - 1.0).max(1.0) * 0.5 * camera.scale, kind: camera.kind }
    }

    /// Texels of `cell`'s tile per screen pixel at the cell centre. Below 1 a texel spreads over more than one pixel.
    /// The stereographic screen has radius · scale / (1 + cos θ) pixels per radian at θ from the view centre in every
    /// direction; the equidistant one has radius · scale / (π/2) along the radius everywhere, and across the radius
    /// it stretches by θ / sin θ. Refraction and aberration are ignored.
    pub fn texels_per_pixel(&self, cell: SkyCell) -> f64 {
        let factor = match self.kind {
            ProjectionKind::Stereographic => 1.0 + self.view_centre.dot(cell.cap().center),
            ProjectionKind::Equidistant => FRAC_PI_2,
        };
        TileGeometry::density_of(cell) * factor / self.pixel_scale
    }
}

/// `(cell, texels per pixel)` for each cell in the order given: `TileScale::texels_per_pixel` over a list.
pub fn estimate_texels_per_pixel(cells: &[SkyCell], scale: &TileScale) -> Vec<(SkyCell, f64)> {
    cells.iter().map(|&cell| (cell, scale.texels_per_pixel(cell))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{CELL_COUNT, GRID_DEPTH};
    use crate::formulas::{J2000, Observer};
    use crate::model::{ViewCenter, hash_direction, region_caps};
    use crate::observation::observe;
    use crate::projection::project_camera;
    use crate::time::StepTimes;
    use ProjectionKind::{Equidistant, Stereographic};

    fn observation_at(view: &View) -> Observation {
        let tokyo = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
        let time = crate::formulas::utc_jd_to_time_ms(J2000).unwrap();
        observe(&tokyo, false, view, time, crate::formulas::evaluate_earth_state(time), &mut StepTimes::default())
    }

    /// The view centre in catalog axes.
    fn find_view_centre(view: &View, observation: &Observation) -> Vector3 {
        rotate_camera_into(prepare_camera(view), observation.to_horizon).forward
    }

    /// The regions whose centres lie within `reach` radians of the view centre.
    fn find_regions_within(view: &View, observation: &Observation, reach: impl Fn(usize) -> f64) -> Vec<SkyCell> {
        let centre = find_view_centre(view, observation);
        (0..CELL_COUNT).filter(|&region| centre.dot(region_caps()[region].center).clamp(-1.0, 1.0).acos() <= reach(region)).map(SkyCell::region).collect()
    }

    /// Screen pixels per texel at the tile centre through the forward chain: the smaller of the two principal
    /// stretches of half-texel steps, equal in every direction on the conformal stereographic screen and the
    /// radial one on the equidistant screen.
    fn measure_pixels_per_texel(cell: SkyCell, observation: &Observation, view: &View, size: usize) -> f64 {
        let (tile, camera, radius, middle) = (TileGeometry::of_cell(cell), prepare_camera(view), (size as f64 - 1.0) * 0.5, TileGeometry::middle());
        let pixel = |column: f64, row: f64| {
            let point = project_camera(camera, observation.apparent_star_direction(tile.direction_of(column, row))).unwrap();
            [point.x * radius, point.y * radius]
        };
        let [right, left, down, up] = [pixel(middle + 0.5, middle), pixel(middle - 0.5, middle), pixel(middle, middle + 0.5), pixel(middle, middle - 0.5)];
        let (a, b, c, d) = (right[0] - left[0], down[0] - up[0], right[1] - left[1], down[1] - up[1]);
        let squares = a * a + b * b + c * c + d * d;                                         // σ₁² + σ₂² of the 2 × 2 Jacobian
        let determinant = a * d - b * c;                                                     // σ₁ · σ₂
        ((squares - (squares * squares - 4.0 * determinant * determinant).max(0.0).sqrt()) / 2.0).sqrt()
    }

    #[test]
    fn estimates_match_the_forward_projected_texel_spacing() {
        let views = [(Stereographic, [300.0, 180.0, 90.0, 20.0, 5.0, 1.0]), (Equidistant, [360.0, 180.0, 90.0, 20.0, 5.0, 1.0])];
        for center in [ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 }, ViewCenter::Zenith] {
            for (projection, fovs) in views {
                for fov in fovs {
                    let view = View { center, projection, fov_degrees: fov };
                    let observation = observation_at(&view);
                    let scale = TileScale::new(&observation, &view, 1001);
                    let regions = find_regions_within(&view, &observation, |_| (fov / 2.0).min(170.0).to_radians().max(0.03));
                    assert!(!regions.is_empty(), "{projection:?} fov={fov}");
                    let children: Vec<SkyCell> = regions.iter().step_by(7).flat_map(|region| [region.children()[1], region.children()[2].children()[3]]).collect();
                    for (cell, ratio) in estimate_texels_per_pixel(&regions, &scale).into_iter().chain(estimate_texels_per_pixel(&children, &scale)) {
                        let measured = measure_pixels_per_texel(cell, &observation, &view, 1001);
                        assert!((ratio * measured - 1.0).abs() < 1e-3, "{projection:?} fov={fov} {cell:?}: {ratio} texels per pixel vs {}", 1.0 / measured);
                    }
                }
            }
        }
    }

    #[test]
    fn cells_keep_their_order_and_an_empty_list_gives_none() {
        let view = View { center: ViewCenter::Zenith, projection: Stereographic, fov_degrees: 90.0 };
        let scale = TileScale::new(&observation_at(&view), &view, 800);
        let cells = [SkyCell::region(17), SkyCell::region(3), SkyCell::region(4000).children()[2], SkyCell::region(3)];
        let ratios = estimate_texels_per_pixel(&cells, &scale);
        assert_eq!(ratios.iter().map(|&(cell, _)| cell).collect::<Vec<_>>(), cells);
        assert_eq!(ratios[1], ratios[3]);
        assert!(estimate_texels_per_pixel(&[], &scale).is_empty());
    }

    #[test]
    #[ignore = "overview only: prints the ratios of the region tiles in view at several fields of view, and the estimate's time"]
    fn print_texels_per_pixel_overview() {
        // In view: the bounding cap reaches into the view disc. The drawing square of the benchmark's 2400 × 1400 frame.
        let size = 1400;
        for projection in [Stereographic, Equidistant] {
            for fov in [180.0, 90.0, 20.0, 5.0, 1.0] {
                let view = View { center: ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 }, projection, fov_degrees: fov };
                let observation = observation_at(&view);
                let scale = TileScale::new(&observation, &view, size);
                let regions = find_regions_within(&view, &observation, |region| (fov / 2.0_f64).to_radians() + region_caps()[region].radius);
                let mut ratios: Vec<f64> = estimate_texels_per_pixel(&regions, &scale).into_iter().map(|(_, ratio)| ratio).collect();
                ratios.sort_by(f64::total_cmp);
                let quantile = |q: f64| ratios[((ratios.len() - 1) as f64 * q).round() as usize];
                let at_centre = scale.texels_per_pixel(SkyCell::region(hash_direction(GRID_DEPTH, find_view_centre(&view, &observation))));
                eprintln!(
                    "{projection:?} fov={fov:>5}: {:>4} tiles, texels per pixel min {:.4} p10 {:.4} median {:.4} p90 {:.4} max {:.4}, view centre {:.4} ({:.1} pixels per texel)",
                    ratios.len(), quantile(0.0), quantile(0.1), quantile(0.5), quantile(0.9), quantile(1.0), at_centre, 1.0 / at_centre,
                );
            }
        }

        // The whole sky is the longest list a caller can pass.
        let view = View { center: ViewCenter::Zenith, projection: Stereographic, fov_degrees: 180.0 };
        let scale = TileScale::new(&observation_at(&view), &view, size);
        let regions: Vec<SkyCell> = (0..CELL_COUNT).map(SkyCell::region).collect();
        let runs = 1000;
        let start = std::time::Instant::now();
        for _ in 0..runs { std::hint::black_box(estimate_texels_per_pixel(std::hint::black_box(&regions), &scale)); }
        eprintln!("{} regions: {:.1} µs per call", regions.len(), start.elapsed().as_secs_f64() * 1e6 / runs as f64);
    }
}
