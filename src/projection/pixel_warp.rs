//! Step 5 for the stereographic view: every screen pixel inside a region's image is pulled back to the region's
//! tile and sampled there. Tile and screen are both stereographic, so a rotation of the sky maps one onto the
//! other by a closed formula: undo the screen projection (one division), undo refraction (a table) and
//! aberration (a constant), rotate into the tile's frame (a 3 × 3 matrix prepared per region) and apply the
//! tile projection (one division). No mesh, no triangles, nothing retained between frames. Every pixel is
//! exact; the only approximations are the filter box of the sampler and the first-order aberration.
use crate::astro::{LIGHT_SPEED_AU_DAY, Matrix3, Vector3, apply_refraction, unrefract_direction, Horizontal};
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Data, ProjectionKind, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, prepare_camera, project_camera};
use crate::regions::{TileGeometry, blend_star};
use crate::selection::Selected;
use crate::time::StepTimes;
use super::mesh::Point;
use super::tile_sampling::TileSampler;

pub(super) fn project_tiles_by_pixel(size: usize, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes) -> Vec<Texel> {
    // Allocate a square image and prepare the current camera.
    assert_eq!(view.projection, ProjectionKind::Stereographic, "the pixel warp inverts the stereographic camera only");
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size.checked_mul(size).expect("drawing area pixel count overflow")];
    if size < 2 { return stars; }
    times.measure("Projection.Camera preparation");
    let camera = prepare_camera(view);

    // One sampler for every region; no projected result is cached between frames.
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);

    // Map each region once, then pull every pixel of its screen image back to the tile.
    for &region in &selected.regions {
        let image = &data.star_regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(region);
        times.measure("Projection.Region mapping");
        let Some(mapping) = RegionMapping::new(&geometry, observation, camera, size) else { continue; };
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows());
        times.measure("Projection.Tile rasterization");
        draw_region(&mut stars, size, &mapping, &sampler, image.texels());
    }
    times.measure("Projection");
    stars
}

/// The screen area a region's tile can cover: a circle (the stereographic image of the tile's bounding cap),
/// or the whole disc when the cap reaches the point behind the camera, whose image is unbounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Image {
    Circle { centre: Point, radius: f64 },
    WholeDisc,
}

/// Screen pixel to tile texel for one region and frame.
pub(super) struct RegionMapping {
    to_tile: Matrix3,                       // horizon axes → tile axes (east, north, centre), aberration undone separately
    aberration: Vector3,                    // the observer's velocity over c in tile axes: subtracted before projecting
    aberration_horizon: Vector3,            // the same in horizon axes, for the length shortcut
    density: f64,                           // texels per radian at the tile centre
    atmosphere: bool,
    camera: CartesianCamera,
    radius: f64,                            // the sky disc: (size − 1) / 2 pixels around its centre
    pub(super) image: Image,
}

impl RegionMapping {
    /// `None` when the region's image misses the sky disc.
    pub(super) fn new(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, size: usize) -> Option<Self> {
        // Fold the rotation to catalog axes and the tile's axes into one matrix; aberration is a shift before it.
        let (east, north, centre) = (tile.east(), tile.north(), tile.center());
        let tile_axes = Matrix3([[east.x, east.y, east.z], [north.x, north.y, north.z], [centre.x, centre.y, centre.z]]);
        let to_tile = tile_axes.compose(observation.to_catalog);
        let aberration_horizon = observation.to_horizon.apply(observation.observer.velocity * (1.0 / LIGHT_SPEED_AU_DAY));
        let aberration = to_tile.apply(aberration_horizon);

        // Bound the tile's sky by the cap of its corners, widened by how much refraction can move its edges.
        let corner = 2.0 * (TileGeometry::middle() * std::f64::consts::SQRT_2 / (2.0 * tile.density())).atan();
        let apparent = observation.apparent_star_direction(centre);
        let altitude = apparent.z.clamp(-1.0, 1.0).asin();
        let spread = if observation.atmosphere {
            let lift = |altitude: f64| apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - altitude;
            (lift((altitude - corner).max(-std::f64::consts::FRAC_PI_2)) - lift((altitude + corner).min(std::f64::consts::FRAC_PI_2))).abs()
        } else { 0.0 };
        let reach = corner + spread;

        // A cap projects to a circle whose diameter lies on the line through the view centre; a cap around the
        // point behind the camera has no bounded image.
        let radius = (size as f64 - 1.0) * 0.5;
        let depth = camera.forward.dot(apparent);
        if reach >= std::f64::consts::PI || depth <= -reach.cos() { return Some(Self { to_tile, aberration, aberration_horizon, density: tile.density(), atmosphere: observation.atmosphere, camera, radius, image: Image::WholeDisc }); }
        let towards_centre = {
            let tangent = camera.forward - apparent * depth;
            let length = tangent.length();
            if length < 1e-12 { camera.up - apparent * apparent.dot(camera.up) } else { tangent }
        }.normalized();
        let (sine, cosine) = reach.sin_cos();
        let mut ends = [[0.0; 2]; 2];
        for (end, sign) in ends.iter_mut().zip([1.0, -1.0]) {
            let edge = apparent * cosine + towards_centre * (sine * sign);
            let point = project_camera(camera, edge)?;
            *end = [radius + point.x * radius, radius - point.y * radius];
        }
        let centre = [(ends[0][0] + ends[1][0]) * 0.5, (ends[0][1] + ends[1][1]) * 0.5];
        let image_radius = (ends[0][0] - ends[1][0]).hypot(ends[0][1] - ends[1][1]) * 0.5;
        if !image_radius.is_finite() { return None; }
        if (centre[0] - radius).hypot(centre[1] - radius) > image_radius + radius + 0.5 { return None; } // the circle misses the disc entirely
        Some(Self { to_tile, aberration, aberration_horizon, density: tile.density(), atmosphere: observation.atmosphere, camera, radius, image: Image::Circle { centre, radius: image_radius } })
    }

    /// The tile coordinates under the centre of pixel (`column`, `row`) and the pixel's footprint in texels, or
    /// `None` when the pixel lies outside the tile: the per-pixel reference the row stepping is checked against.
    #[cfg(test)]
    pub(super) fn tile_position(&self, column: f64, row: f64) -> Option<(Point, f64)> {
        self.tile_of_inside(self.ray_numerator(column, row), self.ray_spread(column, row))
    }

    /// Undo the stereographic screen projection: the apparent direction of the pixel centre in horizon axes is
    /// `ray_numerator / ray_spread`. Both are quadratic in the column, so a row can step through them by
    /// forward differences (`RowRays`).
    #[inline]
    fn ray_numerator(&self, column: f64, row: f64) -> Vector3 {
        let (x, y) = self.ray_tangents(column, row);
        self.camera.right * (2.0 * x) + self.camera.up * (2.0 * y) + self.camera.forward * (1.0 - x * x - y * y)
    }
    #[inline]
    fn ray_spread(&self, column: f64, row: f64) -> f64 {
        let (x, y) = self.ray_tangents(column, row);
        1.0 + x * x + y * y                                                                 // 1 + tan²(θ/2): the camera's 2 / (1 + c)
    }
    /// The pixel centre in the camera's tangent units: tan(θ/2) along the screen axes.
    #[inline]
    fn ray_tangents(&self, column: f64, row: f64) -> (f64, f64) {
        ((column - self.radius) / (self.radius * self.camera.scale), (self.radius - row) / (self.radius * self.camera.scale))
    }

    /// The apparent direction `numerator / spread` in the tile's axes, unnormalized, with its length and the
    /// denominator of the tile's stereographic projection; `None` opposite the tile centre, where that
    /// projection is singular.
    #[inline]
    fn tile_frame(&self, numerator: Vector3, spread: f64) -> Option<(Vector3, f64, f64)> {
        // Undo refraction on the unit direction when there is an atmosphere, then aberration, and rotate into
        // the tile's axes. Without an atmosphere the scale by `spread` is left in: the projection is
        // homogeneous, so only the aberration shift has to be scaled to match, and the length follows from
        // the identity |numerator| = spread to first order in the aberration.
        let (w, length) = if self.atmosphere {
            let w = self.to_tile.apply(unrefract_direction(numerator * (1.0 / spread))) - self.aberration;
            (w, w.length())
        } else {
            (self.to_tile.apply(numerator) - self.aberration * spread, spread - numerator.dot(self.aberration_horizon))
        };
        let denominator = length + w.z;
        (denominator > 1e-9 * length).then_some((w, length, denominator))
    }

    /// The tile coordinates and footprint of the apparent direction `numerator / spread`, or `None` opposite the
    /// tile centre. The result may lie outside the tile.
    #[inline]
    fn tile_of(&self, numerator: Vector3, spread: f64) -> Option<(Point, f64)> {
        let (w, length, denominator) = self.tile_frame(numerator, spread)?;
        Some(self.tile_coordinates(w, length, denominator, spread))
    }

    /// `tile_of` restricted to positions inside the tile, decided before the division.
    #[inline]
    fn tile_of_inside(&self, numerator: Vector3, spread: f64) -> Option<(Point, f64)> {
        let (w, length, denominator) = self.tile_frame(numerator, spread)?;
        let reach = (TileGeometry::middle() + 0.5) * denominator;                            // |2·density·w| ≤ reach ⟺ the position is within the tile
        let (x, y) = (2.0 * self.density * w.x, 2.0 * self.density * w.y);
        if x.abs() > reach || y.abs() > reach { return None; }
        Some(self.tile_coordinates(w, length, denominator, spread))
    }

    /// The tile's stereographic projection, and texels per pixel: the tile's local scale over the screen's.
    #[inline]
    fn tile_coordinates(&self, w: Vector3, length: f64, denominator: f64, spread: f64) -> (Point, f64) {
        let scale = 2.0 * self.density / denominator;
        let middle = TileGeometry::middle();
        let footprint = scale * length * (2.0 / spread) / (self.radius * self.camera.scale);
        ([middle + w.x * scale, middle - w.y * scale], footprint)
    }

    #[cfg(test)]
    fn inside_tile(position: Point) -> bool {
        let limit = TILE_EDGE_TEXELS as f64 - 0.5;
        position[0] >= -0.5 && position[0] <= limit && position[1] >= -0.5 && position[1] <= limit
    }

    /// The tile rows that screen row `row` can touch between the candidate columns `first..=last`, or `None`
    /// when the row misses the tile. A screen row maps into the tile plane as a circle arc (the mapping is a
    /// Möbius transformation up to refraction's small bend), so the arc through the positions of the first,
    /// middle and last column bounds every pixel between: the circle's whole extent when it is small, else the
    /// three positions widened by the sagitta of the half chords. A texel plus half the filter box covers
    /// refraction and rounding; the tests check the bound against every pixel. Rows through the singular
    /// point, whose images reach infinity, fall back to the whole tile.
    pub(super) fn tile_rows_of(&self, row: usize, first: usize, last: usize) -> Option<(usize, usize)> {
        let whole = Some((0, TILE_EDGE_TEXELS - 1));
        let mut points = [[0.0; 2]; 3];
        let mut footprint: f64 = 0.0;
        for (point, column) in points.iter_mut().zip([first as f64, (first + last) as f64 * 0.5, last as f64]) {
            let Some((position, size)) = self.tile_of(self.ray_numerator(column, row as f64), self.ray_spread(column, row as f64)) else { return whole; };
            if !position.iter().all(|value| value.abs() < 1e6) { return whole; }
            *point = position;
            footprint = footprint.max(size);
        }

        // The circle through the three points: the vertical reach of the arc between the first and the last.
        let [a, b, c] = points;
        let (ab, ac) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
        let determinant = 2.0 * (ab[0] * ac[1] - ab[1] * ac[0]);
        let (mut low, mut high) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
        let (ab_squared, ac_squared) = (ab[0] * ab[0] + ab[1] * ab[1], ac[0] * ac[0] + ac[1] * ac[1]);
        if determinant.abs() > 1e-12 * (ab_squared + ac_squared) {
            let centre_y = a[1] + (ab[0] * ac_squared - ac[0] * ab_squared) / determinant;
            let centre_x = a[0] + (ac[1] * ab_squared - ab[1] * ac_squared) / determinant;
            let radius = (centre_x - a[0]).hypot(centre_y - a[1]);
            if radius < 2.0 * TILE_EDGE_TEXELS as f64 {
                low = low.min(centre_y - radius);
                high = high.max(centre_y + radius);
            } else {
                let bc_squared = (c[0] - b[0]).powi(2) + (c[1] - b[1]).powi(2);
                let half_chord = (ab_squared.max(bc_squared) * 0.25).min(radius * radius);
                let sagitta = radius - (radius * radius - half_chord).sqrt();
                low -= sagitta;
                high += sagitta;
            }
        }
        let margin = 1.0 + footprint.max(1.0) * 0.5;
        let (low, high) = (low - margin, high + margin);
        if low > TILE_EDGE_TEXELS as f64 - 0.5 || high < -0.5 { return None; }
        Some((low.max(0.0) as usize, (high.max(0.0) as usize).min(TILE_EDGE_TEXELS - 1)))
    }

    /// The pixel columns of `row` inside the region's image and the sky disc, or `None` for none.
    pub(super) fn columns_of(&self, row: usize, size: usize) -> Option<(usize, usize)> {
        let y = row as f64;
        let disc = self.radius * self.radius + 1e-9 - (y - self.radius).powi(2);
        if disc < 0.0 { return None; }
        let half = disc.sqrt();
        let (mut first, mut last) = (self.radius - half, self.radius + half);
        if let Image::Circle { centre, radius } = self.image {
            let reach = radius * radius - (y - centre[1]).powi(2);
            if reach < 0.0 { return None; }
            let half = reach.sqrt();
            first = first.max(centre[0] - half);
            last = last.min(centre[0] + half);
        }
        let (first, last) = (first.ceil().max(0.0), last.floor().min(size as f64 - 1.0));
        (first <= last).then_some((first as usize, last as usize))
    }

    /// The pixel rows the region's image can touch inside the square.
    fn rows(&self, size: usize) -> (usize, usize) {
        let (top, bottom) = match self.image {
            Image::Circle { centre, radius } => ((centre[1] - radius).ceil().max(0.0), (centre[1] + radius).floor().min(size as f64 - 1.0)),
            Image::WholeDisc => (0.0, size as f64 - 1.0),
        };
        if top > bottom { (1, 0) } else { (top as usize, bottom as usize) }
    }
}

/// Pull every candidate pixel back to the tile, sample it and blend it source-over onto the star layer. Rows
/// whose tile rows hold no light are skipped before any pixel is mapped, which is what makes sparse tiles cheap.
fn draw_region(stars: &mut [Texel], size: usize, mapping: &RegionMapping, sampler: &TileSampler, texels: &[Texel]) {
    let (top, bottom) = mapping.rows(size);
    for row in top..=bottom {
        let Some((first, last)) = mapping.columns_of(row, size) else { continue; };
        let Some((first_tile_row, last_tile_row)) = mapping.tile_rows_of(row, first, last) else { continue; };
        if !sampler.any_light_in_rows(first_tile_row, last_tile_row) { continue; }
        let pixels = &mut stars[row * size..][first..=last];
        let mut rays = RowRays::new(mapping, first as f64, row as f64);
        for texel in pixels {
            let (numerator, spread) = rays.current();
            rays.advance();
            let Some((position, footprint)) = mapping.tile_of_inside(numerator, spread) else { continue; };
            if footprint <= 3.0 && !sampler.may_hold_light_near(position) { continue; }
            let Some(rgba) = sampler.sample(texels, position, [footprint, footprint]) else { continue; };
            blend_star(texel, [rgba[0], rgba[1], rgba[2]], rgba[3]);
            texel.rgb = texel.rgb.map(|value| value.min(1.0));                              // absorb last-bit rounding of the blend at opaque pixels
            texel.opacity = texel.opacity.min(1.0);
        }
    }
}

/// The apparent directions along one screen row as the quadratics `ray_numerator(column)` and
/// `ray_spread(column)`, stepped one column at a time by forward differences (two additions per component).
struct RowRays {
    numerator: Vector3,
    numerator_step: Vector3,
    numerator_curve: Vector3,
    spread: f64,
    spread_step: f64,
    spread_curve: f64,
}

impl RowRays {
    fn new(mapping: &RegionMapping, column: f64, row: f64) -> Self {
        // Second differences of a quadratic are constant; the first differences start from the first column.
        let numerator = mapping.ray_numerator(column, row);
        let next = mapping.ray_numerator(column + 1.0, row);
        let after = mapping.ray_numerator(column + 2.0, row);
        let spread = mapping.ray_spread(column, row);
        let spread_next = mapping.ray_spread(column + 1.0, row);
        let spread_after = mapping.ray_spread(column + 2.0, row);
        Self {
            numerator,
            numerator_step: next - numerator,
            numerator_curve: after - next * 2.0 + numerator,
            spread,
            spread_step: spread_next - spread,
            spread_curve: spread_after - 2.0 * spread_next + spread,
        }
    }
    #[inline]
    fn current(&self) -> (Vector3, f64) { (self.numerator, self.spread) }
    #[inline]
    fn advance(&mut self) {
        self.numerator = self.numerator + self.numerator_step;
        self.numerator_step = self.numerator_step + self.numerator_curve;
        self.spread += self.spread_step;
        self.spread_step += self.spread_curve;
    }
}

#[cfg(test)]
mod tests {
    use crate::projection::Frame;
    use super::*;
    use crate::astro::{J2000, Observer};
    use crate::constants::GRID_DEPTH;
    use crate::model::{RenderOptions, ViewCenter, fixtures, hash_direction, region_caps};
    use crate::observation::observe;
    use crate::regions::refresh_regions;

    fn tokyo() -> Observer { Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() } }
    fn observation_at(fov: f64, atmosphere: bool, center: ViewCenter) -> (View, Observation) {
        let view = View { center, fov_degrees: fov, projection: ProjectionKind::Stereographic };
        let date = crate::astro::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
        let time = crate::astro::utc_jd_to_time_ms(crate::astro::datetime_to_julian_date(&date)).unwrap();
        (view, observe(&tokyo(), atmosphere, &view, time, &mut StepTimes::default()))
    }
    /// The screen pixel of a texel position through the forward chain: the oracle for the inverse mapping.
    fn forward(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, size: usize, position: Point) -> Option<Point> {
        let apparent = observation.apparent_star_direction(tile.direction_of(position[0], position[1]));
        let point = project_camera(camera, apparent)?;
        let radius = (size as f64 - 1.0) * 0.5;
        Some([radius + point.x * radius, radius - point.y * radius])
    }
    fn settings() -> RenderOptions { RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false } }

    #[test]
    fn every_pixel_maps_back_to_the_direction_the_forward_chain_projects() {
        // Regions across the sky at every zoom: the pixel's texel, pushed forward again, lands on the pixel.
        let size = 1116;
        let mut checked = 0;
        for (fov, atmosphere, center) in [(10.0, false, ViewCenter::Facing { azimuth: 5.83, tilt: 0.0 }), (60.0, true, ViewCenter::Facing { azimuth: 5.83, tilt: 0.3 }),
            (180.0, true, ViewCenter::Zenith), (300.0, false, ViewCenter::Facing { azimuth: 1.0, tilt: -0.2 }), (359.0, true, ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 })] {
            let (view, observation) = observation_at(fov, atmosphere, center);
            let camera = prepare_camera(&view);
            let regions: Vec<usize> = (0..region_caps().len()).step_by(97).collect();
            for region in regions {
                let tile = TileGeometry::new(region);
                let Some(mapping) = RegionMapping::new(&tile, &observation, camera, size) else { continue; };
                let (top, bottom) = mapping.rows(size);
                for row in (top..=bottom).step_by(7) {
                    let Some((first, last)) = mapping.columns_of(row, size) else { continue; };
                    for column in (first..=last).step_by(11) {
                        let Some((position, footprint)) = mapping.tile_position(column as f64, row as f64) else { continue; };
                        let back = forward(&tile, &observation, camera, size, position).unwrap();
                        let error = (back[0] - column as f64).hypot(back[1] - row as f64);
                        assert!(error < 0.02, "fov={fov} atmosphere={atmosphere} region={region} pixel=({column}, {row}): back at {back:?}, {error} px off");

                        // The footprint is the texel distance covered by one pixel step, to a few percent.
                        if let (Some((right, _)), Some((left, _))) = (mapping.tile_position(column as f64 + 0.5, row as f64), mapping.tile_position(column as f64 - 0.5, row as f64)) {
                            let measured = (right[0] - left[0]).hypot(right[1] - left[1]);
                            assert!((measured - footprint).abs() < 0.05 * footprint + 1e-6, "fov={fov} region={region}: footprint {footprint} vs {measured}");
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 2000, "{checked}");
    }

    #[test]
    fn stepped_rows_match_direct_evaluation_and_tile_row_estimates_hold_at_every_pixel() {
        // Forward differences must agree with the per-pixel formula to well under a thousandth of a texel, and
        // the three-point tile-row estimate must contain the tile rows of every pixel of the screen row.
        let size = 1116;
        for (fov, atmosphere, center) in [(10.0, true, ViewCenter::Facing { azimuth: 5.83, tilt: 0.0 }), (60.0, false, ViewCenter::Facing { azimuth: 5.83, tilt: 0.3 }),
            (180.0, true, ViewCenter::Zenith), (359.0, false, ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 })] {
            let (view, observation) = observation_at(fov, atmosphere, center);
            let camera = prepare_camera(&view);
            let mut rows_checked = 0;
            for region in (0..region_caps().len()).step_by(13) {
                let tile = TileGeometry::new(region);
                let Some(mapping) = RegionMapping::new(&tile, &observation, camera, size) else { continue; };
                let (top, bottom) = mapping.rows(size);
                for row in top..=bottom {
                    let Some((first, last)) = mapping.columns_of(row, size) else { continue; };
                    let estimate = mapping.tile_rows_of(row, first, last);
                    let mut rays = RowRays::new(&mapping, first as f64, row as f64);
                    let (mut lowest, mut highest) = (f64::INFINITY, f64::NEG_INFINITY);
                    for column in first..=last {
                        let (numerator, spread) = rays.current();
                        rays.advance();
                        let stepped = mapping.tile_of(numerator, spread);
                        let direct = mapping.tile_of(mapping.ray_numerator(column as f64, row as f64), mapping.ray_spread(column as f64, row as f64));
                        match (stepped, direct) {
                            (Some((_, _)), Some((b, _))) if b[0].abs().max(b[1].abs()) > 1000.0 => {}   // near the singular pixel the magnified positions are far outside anyway
                            (Some((a, fa)), Some((b, fb))) => {
                                let scale = if b[0].abs().max(b[1].abs()) < 100.0 { 100.0 } else { 10.0 * b[0].abs().max(b[1].abs()) }; // a ten-thousandth of a texel after a row of steps; far outside the tile only the magnified positions' relative agreement matters
                                assert!((a[0] - b[0]).abs() < 1e-6 * scale && (a[1] - b[1]).abs() < 1e-6 * scale && (fa - fb).abs() < 1e-6 * fb.max(1.0) * scale, "fov={fov} region={region} ({column}, {row}): {a:?} vs {b:?}, footprints {fa} vs {fb}");
                                if RegionMapping::inside_tile(b) {
                                    lowest = lowest.min(b[1] - fb.max(1.0) * 0.5);
                                    highest = highest.max(b[1] + fb.max(1.0) * 0.5);
                                }
                            }
                            (None, None) => {}
                            other => panic!("fov={fov} region={region} ({column}, {row}): {other:?}"),
                        }
                    }
                    if lowest <= highest {
                        let (low, high) = estimate.unwrap_or_else(|| panic!("fov={fov} region={region} row {row}: tile rows {lowest}..{highest} estimated as none"));
                        assert!(low as f64 <= lowest.max(0.0) + 1e-9 && high as f64 + 1.0 >= highest.min(63.0), "fov={fov} region={region} row {row}: tile rows {lowest}..{highest} estimated as {low}..={high}");
                        rows_checked += 1;
                    }
                }
            }
            assert!(rows_checked > 100, "fov={fov}: {rows_checked}");
        }
    }

    #[test]
    fn the_candidate_circle_contains_every_covered_pixel() {
        // Mapping the whole disc finds the covered pixels; all of them lie inside the circle, which is not wasteful.
        let size = 301;
        for (fov, atmosphere, angle) in [(60.0, true, 0.2), (60.0, true, 0.45), (180.0, false, 1.2), (300.0, true, 2.2)] {
            let (view, observation) = observation_at(fov, atmosphere, ViewCenter::Facing { azimuth: 5.83, tilt: 0.1 });
            let camera = prepare_camera(&view);
            let apparent = camera.forward * f64::cos(angle) + (camera.up * 0.6 + camera.right * 0.8) * f64::sin(angle); // `angle` from the view centre
            let region = hash_direction(GRID_DEPTH, observation.to_catalog.apply(apparent));
            let tile = TileGeometry::new(region);
            let mapping = RegionMapping::new(&tile, &observation, camera, size).unwrap();
            let Image::Circle { centre, radius } = mapping.image else { panic!("bounded region") };
            let everywhere = RegionMapping { image: Image::WholeDisc, ..mapping };
            let mut covered = 0;
            for row in 0..size {
                let Some((first, last)) = everywhere.columns_of(row, size) else { continue; };
                for column in first..=last {
                    if everywhere.tile_position(column as f64, row as f64).is_none() { continue; }
                    covered += 1;
                    let distance = (column as f64 - centre[0]).hypot(row as f64 - centre[1]);
                    assert!(distance <= radius + 1e-6, "fov={fov}: covered pixel ({column}, {row}) outside the circle by {}", distance - radius);
                }
            }
            assert!(covered > 50, "fov={fov}: {covered}");
            assert!(std::f64::consts::PI * radius * radius < 3.0 * covered as f64, "fov={fov}: circle area {} for {covered} covered pixels", std::f64::consts::PI * radius * radius);
        }
    }

    #[test]
    fn a_star_lands_where_the_point_renderer_puts_it_and_an_empty_selection_clears() {
        let time = crate::astro::utc_jd_to_time_ms(J2000).unwrap();
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        for (fov, atmosphere) in [(180.0, false), (60.0, true), (10.0, true)] {
            let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.02 }, projection: ProjectionKind::Stereographic, fov_degrees: fov };
            let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(GRID_DEPTH, direction);
            let star = fixtures::row(direction, 0.0, 3.0, 1);
            let mut data = fixtures::layout(&[(region, 0, star)]);
            let selected = Selected { regions: vec![region], whole_sky: false };
            refresh_regions(&mut data, &selected, time, &view, &settings(), &mut StepTimes::default());
            let mut frame = Frame::square(401);
            let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);

            let expected = project_camera(prepare_camera(&view), observation.apparent_star_direction(direction)).unwrap();
            let expected = [200.0 + expected.x * 200.0, 200.0 - expected.y * 200.0];
            let mut total = 0.0;
            let mut center = [0.0; 2];
            for (index, texel) in frame.stars().iter().enumerate() {
                let weight = texel.opacity as f64;
                total += weight;
                center[0] += (index % 401) as f64 * weight;
                center[1] += (index / 401) as f64 * weight;
            }
            assert!(total > 0.0);
            let tile = TileGeometry::new(region);                                           // the glyph sits at the rounded texel: allow half a texel
            let camera = prepare_camera(&view);
            let (a, b) = (forward(&tile, &observation, camera, 401, [31.5, 31.5]).unwrap(), forward(&tile, &observation, camera, 401, [32.5, 31.5]).unwrap());
            let tolerance = 0.5 * (a[0] - b[0]).hypot(a[1] - b[1]) + 1.0;
            assert!((center[0] / total - expected[0]).abs() < tolerance && (center[1] / total - expected[1]).abs() < tolerance,
                "{fov}: center {center:?} / {total}, expected {expected:?}, tolerance {tolerance}");
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));

            let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &Selected { regions: Vec::new(), whole_sky: false }, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }

    #[test]
    fn region_images_blend_source_over_once_per_pixel() {
        // Two half-opaque regions over the same sky give 0.75, not four half-blends; an opaque one stays bounded.
        let (view, observation) = observation_at(60.0, false, ViewCenter::Facing { azimuth: 5.83, tilt: 0.1 });
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let neighbour = hash_direction(GRID_DEPTH, (observation.patch.center + TileGeometry::new(region).east() * 0.001).normalized());
        let mut data = Data::default();
        for (index, (color, which)) in [([0.5, 0.0, 0.0], region), ([0.0, 0.0, 0.5], if neighbour == region { region } else { neighbour })].into_iter().enumerate() {
            let image = &mut data.star_regions[which].merged_tile;
            image.tile.texels = vec![Texel { rgb: color, opacity: 0.5 }; TILE_EDGE_TEXELS.pow(2)];
            image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
            if index == 1 && which == region { image.tile.texels = vec![Texel { rgb: color, opacity: 0.5 }; TILE_EDGE_TEXELS.pow(2)]; }
        }
        let regions = if neighbour == region { vec![region] } else { vec![region.min(neighbour), region.max(neighbour)] };
        let selected = Selected { regions, whole_sky: false };
        let mut frame = Frame::square(201);
        let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        let centre = frame.star(100, 100);
        if neighbour != region {
            assert!((centre.opacity - 0.75).abs() < 1e-6 && (centre.rgb[0] - 0.25).abs() < 1e-6 && (centre.rgb[2] - 0.5).abs() < 1e-6, "{centre:?}");
        } else {
            assert!((centre.opacity - 0.5).abs() < 1e-6, "{centre:?}");
        }
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0 && texel.rgb.iter().all(|&c| c <= 1.0)));

        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        assert_eq!(frame.star(100, 100).opacity, 1.0);
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0 && texel.rgb.iter().all(|&c| c <= 1.0)));
    }

    #[test]
    fn a_region_behind_the_camera_draws_only_at_the_rim() {
        // At 359° the region around the view's antipode has an unbounded image: it is tested over the whole disc
        // and lights only the outer ring beyond its nearest corner, never the centre. (Stereographic stretches
        // the last degrees over most of the disc: the ring starts about a third of the way out.)
        let (view, observation) = observation_at(359.0, false, ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 });
        let camera = prepare_camera(&view);
        let region = hash_direction(GRID_DEPTH, observation.to_catalog.apply(camera.forward * -1.0));
        let tile = TileGeometry::new(region);
        let size = 201;
        let mapping = RegionMapping::new(&tile, &observation, camera, size).unwrap();
        assert_eq!(mapping.image, Image::WholeDisc);
        let mut data = Data::default();
        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: true };
        let mut frame = Frame::square(size);
        let projected_stars = project_tiles_by_pixel(size, &data, &selected, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        let corner = 2.0 * (TileGeometry::middle() * std::f64::consts::SQRT_2 / (2.0 * tile.density())).atan();
        let offset = observation.apparent_star_direction(tile.center()).dot(camera.forward * -1.0).clamp(-1.0, 1.0).acos();
        let inner = 100.0 * camera.scale * ((std::f64::consts::PI - corner - offset) / 2.0).tan(); // the screen radius of the tile's nearest point
        assert!(inner > 3.0 && inner < 60.0, "{inner}");
        let mut lit = 0;
        for (index, texel) in frame.stars().iter().enumerate() {
            assert!(texel.opacity.is_finite() && texel.opacity <= 1.0);
            if texel.opacity > 0.0 {
                lit += 1;
                let distance = ((index % size) as f64 - 100.0).hypot((index / size) as f64 - 100.0);
                assert!(distance > inner - 1.0, "rear tile drawn inside the disc at {index}, {distance} from the centre");
            }
        }
        assert!(lit > 0 && frame.star(100, 100).is_empty());
    }

    #[test]
    #[ignore = "timing only; uses embedded BSC5 and synthetic dense tiles, never external datasets"]
    fn time_pixel_mapping() {
        // Use one prepared catalog and the screenshot's camera; only projection is timed.
        let mut report = crate::progress::Report::quiet();
        let catalog = crate::catalog::load_embedded_catalog(&mut report).unwrap();
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut report).unwrap();
        let original = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        for fov in [180.0, 351.6] {
            let (view, observation) = observation_at(fov, false, ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 });
            let mut data = original.clone();
            let selected = crate::selection::select_regions(&data.star_regions, &observation, &mut StepTimes::default());
            let settings = RenderOptions { magnitude_threshold: 9.0, ..settings() };
            refresh_regions(&mut data, &selected, observation.time_ms, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::new(1917, 1116, 1116, (10, 20));
            for dense in [false, true] {
                if dense {
                    for &region in &selected.regions {
                        let image = &mut data.star_regions[region].merged_tile;
                        image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS.pow(2)];
                        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
                    }
                }
                let mut elapsed = 0.0;
                for run in 0..4 {
                    let start = std::time::Instant::now();
                    let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
                    frame.insert_projected_stars(&projected_stars);
                    if run > 0 { elapsed += start.elapsed().as_secs_f64(); }
                    std::hint::black_box(frame.stars());
                }
                eprintln!("Pixel warp fov={fov}, dense={dense}, regions={}: {:.3}ms; three frames after warmup", selected.regions.len(), elapsed * 1000.0 / 3.0);
                let mut detail = StepTimes::with_trace(true);
                let projected_stars = project_tiles_by_pixel(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut detail);
                frame.insert_projected_stars(&projected_stars);
                detail.measure("END");
                for step in detail.steps().iter().filter(|step| matches!(step.name, "Region mapping" | "Tile filter preparation" | "Tile rasterization")) {
                    eprintln!("  {}: {:.3}ms (separate traced frame)", step.name, step.average_seconds * 1000.0);
                }
            }
        }
    }
}
