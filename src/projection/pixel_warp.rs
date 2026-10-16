//! Step 5: every screen pixel inside a region's image is pulled back to the region's tile and sampled there.
//! Tile and screen are both azimuthal images of the sky, the tile stereographic around its region centre and
//! the screen stereographic or equidistant around the view centre, so a pixel's direction follows from a closed
//! formula: undo the screen projection (one division for the stereographic view, a per-frame table for the
//! equidistant one), undo refraction (a table) and aberration (a constant), rotate into the tile's frame (a
//! 3 × 3 matrix prepared per region) and apply the tile projection (one division). No mesh or triangles;
//! region mappings are rebuilt each frame. Pixel positions follow the inverse formulas, with table interpolation and
//! first-order aberration approximations. Sampling uses box footprints; portions outside a tile are transparent.
use std::f64::consts::{FRAC_PI_2, PI, SQRT_2, TAU};
use crate::formulas::{LIGHT_SPEED_AU_DAY, Matrix3, Vector3, apply_refraction, unrefract_direction, Horizontal};
use crate::constants::{STEREOGRAPHIC_ROW_REBASE_PIXELS, TILE_EDGE_TEXELS};
use crate::model::{ProjectionKind, Texel};
#[cfg(test)]
use crate::model::{Data, Region, View};
use crate::observation::Observation;
use crate::projection::CartesianCamera;
#[cfg(test)]
use crate::projection::prepare_camera;
#[cfg(test)]
use super::executor::project_test_image;
use crate::regions::TileGeometry;
use super::screen_blend::blend_screen_pixel;
#[cfg(test)]
use crate::selection::Selected;
#[cfg(test)]
use crate::time::StepTimes;
use super::tile_sampling::TileSampler;

type Point = [f64; 2];

/// Use cached image support only where it is qualified; unknown or invalid data keeps the full tile.
fn select_content_radius(content_radius: Option<f64>, atmosphere: bool) -> f64 {
    // Refraction can bend the image outside the tighter cap; retain its original conservative bounds.
    let full_radius = (TileGeometry::middle() + 1.0) * SQRT_2;
    if atmosphere { return full_radius; }

    // Empty images are skipped by the caller. A missing or unusable radius never means an empty image.
    match content_radius {
        Some(radius) if radius.is_finite() && radius > 0.0 && radius < full_radius => radius,
        _ => full_radius,
    }
}

/// The camera over the drawing square: the sky disc's radius and, for the equidistant view, cos θ and
/// sin θ / |p| over |p|² (|p| the pixel's distance from the centre in disc radii, θ its angle from the view
/// centre), 4097 entries built per frame so that no pixel needs trigonometry. Both are smooth in |p|², and the
/// linear interpolation stays within 0.02″ of the formula at 360°.
pub(super) struct Screen {
    camera: CartesianCamera,
    size: usize,
    radius: f64,                            // the sky disc: (size − 1) / 2 pixels around its centre
    inverse_pixel_scale: f64,               // 1 / (radius × camera scale), reused by every pixel footprint
    equidistant: Vec<[f64; 2]>,             // empty for the stereographic view
}

const EQUIDISTANT_STEPS: usize = 4096;

impl Screen {
    pub(super) fn new(camera: CartesianCamera, size: usize) -> Self {
        let radius = (size as f64 - 1.0) * 0.5;
        let equidistant = match camera.kind {
            ProjectionKind::Stereographic => Vec::new(),
            ProjectionKind::Equidistant => (0..=EQUIDISTANT_STEPS).map(|index| {
                let distance = (index as f64 / EQUIDISTANT_STEPS as f64).sqrt();
                let angle = distance * FRAC_PI_2 / camera.scale;                        // the camera's radius is θ / (π / 2) · scale
                let (sine, cosine) = angle.sin_cos();
                [cosine, if distance > 0.0 { sine / distance } else { FRAC_PI_2 / camera.scale }]
            }).collect(),
        };
        Self { camera, size, radius, inverse_pixel_scale: 1.0 / (radius * camera.scale), equidistant }
    }

    /// The apparent direction of the pixel centre (`column`, `row`) in horizon axes as `numerator / spread`,
    /// and the screen's radial scale factor for the footprint: its pixels per radian are `radius · scale /
    /// factor`. For the stereographic view both are quadratic in the column, which `RowRays` exploits.
    #[inline]
    fn ray(&self, column: f64, row: f64) -> (Vector3, f64, f64) {
        let (x, y) = ((column - self.radius) / self.radius, (self.radius - row) / self.radius);
        match self.camera.kind {
            ProjectionKind::Stereographic => {
                let (x, y) = (x / self.camera.scale, y / self.camera.scale);               // tan(θ/2) along the screen axes
                let spread = 1.0 + x * x + y * y;                                           // 1 + tan²(θ/2): the camera's 2 / (1 + c)
                (self.camera.right * (2.0 * x) + self.camera.up * (2.0 * y) + self.camera.forward * (1.0 - x * x - y * y), spread, 2.0 / spread)
            }
            ProjectionKind::Equidistant => (self.equidistant_ray(x, y, x * x + y * y), 1.0, FRAC_PI_2),
        }
    }

    /// The unit direction of the equidistant pixel at (`x`, `y`) disc radii from the centre, `squared` = x² + y².
    #[inline]
    fn equidistant_ray(&self, x: f64, y: f64, squared: f64) -> Vector3 {
        let position = squared.clamp(0.0, 1.0) * EQUIDISTANT_STEPS as f64;
        let index = (position as usize).min(EQUIDISTANT_STEPS - 1);
        let fraction = position - index as f64;
        let [[cosine_low, scale_low], [cosine_high, scale_high]] = [self.equidistant[index], self.equidistant[index + 1]];
        let cosine = cosine_low + (cosine_high - cosine_low) * fraction;
        let scale = scale_low + (scale_high - scale_low) * fraction;
        self.camera.forward * cosine + self.camera.right * (x * scale) + self.camera.up * (y * scale)
    }
}

/// Candidate pixel centres whose filter footprints can overlap a region's tile. Stereographic caps map to
/// circles, circle exteriors, or a half-plane when an edge passes through the antipode. Equidistant:
/// the cap lies between two angles from the view centre, an annulus on the screen, within the box of its sector
/// of directions around the centre.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Image {
    Circle { centre: Point, radius: f64 },
    OutsideCircle { centre: Point, radius: f64 }, // a cap containing the stereographic antipode covers the circle's exterior
    HalfPlane { normal: Point, minimum: f64 },   // normal · pixel >= minimum when the cap edge passes through the antipode
    Annulus { inner: f64, outer: f64, left: f64, right: f64, top: f64, bottom: f64 },
    WholeDisc,
}

/// Screen pixel to tile texel for one region and frame.
#[derive(Clone, Copy)]
pub(super) struct RegionMapping<'a> {
    to_tile: Matrix3,                       // horizon axes → tile axes (east, north, centre), aberration undone separately
    aberration: Vector3,                    // the observer's velocity over c in tile axes: subtracted before projecting
    aberration_horizon: Vector3,            // the same in horizon axes, for the length shortcut
    density: f64,                           // texels per radian at the tile centre
    atmosphere: bool,
    screen: &'a Screen,
    image: Image,
}

impl<'a> RegionMapping<'a> {
    /// `None` when the region's image misses the sky disc.
    pub(super) fn new(tile: &TileGeometry, observation: &Observation, screen: &'a Screen, content_radius: Option<f64>) -> Option<Self> {
        // Fold the rotation to catalog axes and the tile's axes into one matrix; aberration is a shift before it.
        let (east, north, centre) = (tile.east(), tile.north(), tile.center());
        let tile_axes = Matrix3([[east.x, east.y, east.z], [north.x, north.y, north.z], [centre.x, centre.y, centre.z]]);
        let to_tile = tile_axes.compose(observation.to_catalog);
        let aberration_horizon = observation.to_horizon.apply(observation.observer.velocity * (1.0 / LIGHT_SPEED_AU_DAY));
        let aberration = to_tile.apply(aberration_horizon);
        let mapping = |image| Self { to_tile, aberration, aberration_horizon, density: tile.density(), atmosphere: observation.atmosphere, screen, image };

        // Bound the light's cached support, keeping the whole tile for refraction or missing metadata.
        let support_radius = select_content_radius(content_radius, observation.atmosphere);
        let corner = 2.0 * (support_radius / (2.0 * tile.density())).atan();
        let apparent = observation.apparent_star_direction(centre);
        let altitude = apparent.z.clamp(-1.0, 1.0).asin();
        let spread = if observation.atmosphere {
            let lift = |altitude: f64| apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - altitude;
            (lift((altitude - corner).max(-FRAC_PI_2)) - lift((altitude + corner).min(FRAC_PI_2))).abs()
        } else { 0.0 };
        let (camera, radius) = (screen.camera, screen.radius);
        let pixel_margin = SQRT_2 * 0.5;
        let reach = corner + spread + if camera.kind == ProjectionKind::Equidistant {
            pixel_margin * FRAC_PI_2 * screen.inverse_pixel_scale // radial footprint also bounds the wider equidistant filter
        } else { 0.0 };
        let depth = camera.forward.dot(apparent);
        if reach >= PI { return Some(mapping(Image::WholeDisc)); }

        match camera.kind {
            ProjectionKind::Stereographic => {
                // Solve the cap's stereographic image directly, avoiding extra point projections.
                let (sine, cosine) = reach.sin_cos();
                let denominator = depth + cosine;
                let horizontal = camera.right.dot(apparent);
                let vertical = camera.up.dot(apparent);
                let pixel_scale = radius * camera.scale;

                // A cap edge through the antipode is a line. Avoid subtracting enormous circle radii there.
                if denominator.abs() < 1e-8 {
                    let normal = [horizontal, -vertical];
                    let minimum = 0.5 * (cosine - depth) * pixel_scale + radius * (normal[0] + normal[1])
                        - pixel_margin * normal[0].hypot(normal[1]);
                    return Some(mapping(Image::HalfPlane { normal, minimum }));
                }
                let outside_circle = denominator < 0.0;
                let centre = [radius + pixel_scale * horizontal / denominator, radius - pixel_scale * vertical / denominator];
                let image_radius = pixel_scale * sine / denominator.abs();
                let distance = (centre[0] - radius).hypot(centre[1] - radius);
                if outside_circle {
                    let image_radius = (image_radius - pixel_margin).max(0.0);
                    if distance + radius < image_radius { return None; }
                    Some(mapping(Image::OutsideCircle { centre, radius: image_radius }))
                } else {
                    let image_radius = image_radius + pixel_margin;
                    if distance > image_radius + radius { return None; }
                    Some(mapping(Image::Circle { centre, radius: image_radius }))
                }
            }
            ProjectionKind::Equidistant => {
                // The cap spans the angles `angle ± reach` from the view centre, which the equidistant screen
                // maps to radii in proportion, and the directions around the centre within `spread` of the
                // cap's own, unless the cap contains the centre or the point behind the camera.
                let angle = depth.clamp(-1.0, 1.0).acos();
                let pixels_per_radian = radius * camera.scale / FRAC_PI_2;
                let inner = ((angle - reach).max(0.0) * pixels_per_radian - 0.5).max(0.0);
                let outer = (angle + reach).min(PI) * pixels_per_radian + 0.5;
                if inner > radius + 0.5 { return None; }
                let limit = screen.size as f64 - 1.0;
                let (left, right, top, bottom) = if angle - reach <= 0.0 || angle + reach >= PI { (0.0, limit, 0.0, limit) } else {
                    let spread = (reach.sin() / angle.sin()).min(1.0).asin();
                    let around = camera.up.dot(apparent).atan2(camera.right.dot(apparent));
                    let within = |direction: f64| { let turn = (direction - around).rem_euclid(TAU); turn <= spread || turn >= TAU - spread };
                    let mut points = Vec::with_capacity(8);
                    for distance in [inner, outer] {
                        for direction in [around - spread, around + spread] { points.push([distance * direction.cos(), distance * direction.sin()]); }
                    }
                    for axis in 0..4 {
                        let direction = axis as f64 * FRAC_PI_2;
                        if within(direction) { points.push([outer * direction.cos(), outer * direction.sin()]); }
                    }
                    let (mut left, mut right, mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
                    for [x, y] in points {
                        left = left.min(radius + x); right = right.max(radius + x);
                        top = top.min(radius - y); bottom = bottom.max(radius - y);
                    }
                    ((left - 0.5).max(0.0), (right + 0.5).min(limit), (top - 0.5).max(0.0), (bottom + 0.5).min(limit))
                };
                Some(mapping(Image::Annulus { inner, outer, left, right, top, bottom }))
            }
        }
    }

    /// The tile coordinates under the centre of pixel (`column`, `row`) and the pixel's footprint in texels, or
    /// `None` when the pixel lies outside the tile: the per-pixel reference the row stepping is checked against.
    #[cfg(test)]
    fn tile_position(&self, column: f64, row: f64) -> Option<(Point, f64)> {
        let (numerator, spread, factor) = self.screen.ray(column, row);
        self.tile_of(numerator, spread, factor).filter(|(position, _)| Self::inside_tile(*position))
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
    fn tile_of(&self, numerator: Vector3, spread: f64, factor: f64) -> Option<(Point, f64)> {
        let (w, length, denominator) = self.tile_frame(numerator, spread)?;
        Some(self.tile_coordinates(w, length, denominator, factor))
    }

    /// Reject footprints entirely outside the tile before dividing, while retaining partial overlaps.
    #[inline]
    fn tile_of_overlapping_footprint(&self, numerator: Vector3, spread: f64, factor: f64) -> Option<(Point, f64)> {
        let (w, length, denominator) = self.tile_frame(numerator, spread)?;
        self.project_overlapping_footprint(w, length, denominator, factor)
    }

    /// Project an already transformed direction, keeping the same filter and early rejection in both paths.
    #[inline]
    fn project_overlapping_footprint(&self, direction: Vector3, length: f64, denominator: f64, factor: f64) -> Option<(Point, f64)> {
        // Reject a pixel whose entire filter lies outside the tile, before taking a reciprocal.
        let half_footprint = self.density * length * factor * self.screen.inverse_pixel_scale;
        let reach = (TileGeometry::middle() + 0.5) * denominator + half_footprint.max(0.5 * denominator);
        let scale_numerator = 2.0 * self.density;
        let (x, y) = (scale_numerator * direction.x, scale_numerator * direction.y);
        if x.abs() >= reach || y.abs() >= reach { return None; }

        // One reciprocal supplies both texture coordinates and the full filter width.
        let inverse = 1.0 / denominator;
        let middle = TileGeometry::middle();
        Some(([middle + x * inverse, middle - y * inverse], 2.0 * half_footprint * inverse))
    }

    /// The tile's stereographic projection, and texels per pixel: the tile's local scale over the screen's
    /// radial one (the equidistant screen stretches the other direction less, so its box errs towards blur).
    #[inline]
    fn tile_coordinates(&self, w: Vector3, length: f64, denominator: f64, factor: f64) -> (Point, f64) {
        let scale = 2.0 * self.density / denominator;
        let middle = TileGeometry::middle();
        let footprint = scale * length * factor * self.screen.inverse_pixel_scale;
        ([middle + w.x * scale, middle - w.y * scale], footprint)
    }

    #[cfg(test)]
    fn inside_tile(position: Point) -> bool {
        let limit = TILE_EDGE_TEXELS as f64 - 0.5;
        position[0] >= -0.5 && position[0] <= limit && position[1] >= -0.5 && position[1] <= limit
    }

    /// The tile rows that screen row `row` can touch between the candidate columns `first..=last`, or `None`
    /// when the row misses the tile. Under the stereographic view a screen row maps into the tile plane as a
    /// circle arc (the mapping is a Möbius transformation up to refraction's small bend), so the arc through
    /// the positions of the first, middle and last column bounds every pixel between: the circle's whole
    /// extent when it is small, else the three positions widened by the sagitta of the half chords. A texel
    /// plus half the filter box covers refraction and rounding; the tests check the bound against every pixel.
    /// Rows through the singular point, whose images reach infinity, fall back to the whole tile, and so does
    /// the equidistant view, whose rows do not map to arcs.
    fn tile_rows_of(&self, row: usize, first: usize, last: usize) -> Option<(usize, usize)> {
        let whole = Some((0, TILE_EDGE_TEXELS - 1));
        if self.screen.camera.kind == ProjectionKind::Equidistant { return whole; }
        let mut points = [[0.0_f64; 2]; 3];
        let mut footprint: f64 = 0.0;
        for (point, column) in points.iter_mut().zip([first as f64, (first + last) as f64 * 0.5, last as f64]) {
            let (numerator, spread, factor) = self.screen.ray(column, row as f64);
            let Some((position, size)) = self.tile_of(numerator, spread, factor) else { return whole; };
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

    /// The pixel columns of `row` inside the region's image and the sky disc: up to two runs, because an
    /// annulus leaves a hole in the middle of a row.
    fn columns_of(&self, row: usize) -> [Option<(usize, usize)>; 2] {
        let y = row as f64;
        let radius = self.screen.radius;
        let disc = radius * radius + 1e-9 - (y - radius).powi(2);
        if disc < 0.0 { return [None; 2]; }
        let half = disc.sqrt();
        let (mut first, mut last) = (radius - half, radius + half);
        let mut hole = None;
        match self.image {
            Image::Circle { centre, radius: image_radius } => {
                let reach = image_radius * image_radius - (y - centre[1]).powi(2);
                if reach < 0.0 { return [None; 2]; }
                let half = reach.sqrt();
                first = first.max(centre[0] - half);
                last = last.min(centre[0] + half);
            }
            Image::HalfPlane { normal, minimum } => {
                let boundary = minimum - normal[1] * y;
                if normal[0] > 0.0 { first = first.max(boundary / normal[0]); }
                else if normal[0] < 0.0 { last = last.min(boundary / normal[0]); }
                else if boundary > 0.0 { return [None; 2]; }
            }
            Image::OutsideCircle { centre, radius: image_radius } => {
                let gap = image_radius * image_radius - (y - centre[1]).powi(2);
                if gap > 0.0 {
                    let half = gap.sqrt();
                    hole = Some((centre[0] - half, centre[0] + half));
                }
            }
            Image::Annulus { inner, outer, left, right, top, bottom } => {
                if y < top || y > bottom { return [None; 2]; }
                let reach = outer * outer - (y - radius).powi(2);
                if reach < 0.0 { return [None; 2]; }
                let half = reach.sqrt();
                first = first.max(radius - half).max(left);
                last = last.min(radius + half).min(right);
                let gap = inner * inner - (y - radius).powi(2);
                if gap > 0.0 { let half = gap.sqrt(); hole = Some((radius - half, radius + half)); }
            }
            Image::WholeDisc => {}
        }
        let (first, last) = (first.ceil().max(0.0), last.floor().min(self.screen.size as f64 - 1.0));
        if first > last { return [None; 2]; }
        match hole {
            None => [Some((first as usize, last as usize)), None],
            Some((hole_left, hole_right)) => {
                // Pixels strictly inside the hole are skipped; a centre on the inner circle is kept, once.
                let before = (first, hole_left.floor().min(last));
                let after = (hole_right.ceil().max(first).max(before.1 + 1.0), last);
                [(before.0 <= before.1).then_some((before.0 as usize, before.1 as usize)), (after.0 <= after.1).then_some((after.0 as usize, after.1 as usize))]
            }
        }
    }

    /// Candidate rows including filter support around the region's image, clipped to the square.
    fn rows(&self) -> (usize, usize) {
        let (radius, limit) = (self.screen.radius, self.screen.size as f64 - 1.0);
        let (top, bottom) = match self.image {
            Image::Circle { centre, radius: image_radius } => ((centre[1] - image_radius).ceil().max(0.0), (centre[1] + image_radius).floor().min(limit)),
            Image::Annulus { outer, top, bottom, .. } => (top.max(radius - outer).ceil().max(0.0), bottom.min(radius + outer).floor().min(limit)),
            Image::WholeDisc | Image::OutsideCircle { .. } | Image::HalfPlane { .. } => (0.0, limit),
        };
        if top > bottom { (1, 0) } else { (top as usize, bottom as usize) }
    }
}

/// Pull candidate pixel footprints back to the tile, retaining partial overlaps, and screen-blend. Rows
/// whose tile rows hold no light are skipped before any pixel is mapped, which is what makes sparse tiles cheap.
pub(super) fn draw_region(stars: &mut [Texel], mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>, texels: &[Texel]) {
    // Select the row calculation once; refraction and equidistant views keep their general mapping.
    let screen = mapping.screen;
    let use_transformed_rows = screen.camera.kind == ProjectionKind::Stereographic && !mapping.atmosphere;
    let (top, bottom) = mapping.rows();

    // Retain the same candidate spans and empty-row rejection before drawing any pixels.
    for row in top..=bottom {
        for (first, last) in mapping.columns_of(row).into_iter().flatten() {
            let Some((first_tile_row, last_tile_row)) = mapping.tile_rows_of(row, first, last) else { continue; };
            if !sampler.any_light_in_rows(first_tile_row, last_tile_row) { continue; }
            let pixels = &mut stars[row * screen.size..][first..=last];

            // Keep the choice outside the pixel loop so each loop has only the state it needs.
            if use_transformed_rows {
                draw_transformed_span(pixels, mapping, sampler, texels, first, row);
            } else {
                draw_screen_span(pixels, mapping, sampler, texels, first, row);
            }
        }
    }
}

/// Draw a span whose tile-space direction and length can be advanced by additions alone.
fn draw_transformed_span(pixels: &mut [Texel], mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>, texels: &[Texel], first_column: usize, row: usize) {
    // Rebase long spans from the formula periodically, keeping accumulated roundoff below the tested limit.
    for (chunk, pixels) in pixels.chunks_mut(STEREOGRAPHIC_ROW_REBASE_PIXELS).enumerate() {
        let column = first_column + chunk * STEREOGRAPHIC_ROW_REBASE_PIXELS;
        let mut rays = TransformedRow::new(mapping, column as f64, row as f64);

        // No matrix multiplication or rebase decision is needed inside this pixel loop.
        for pixel in pixels {
            let sample = rays.project_footprint(mapping);
            rays.advance();
            if let Some((position, footprint)) = sample {
                sample_and_blend_pixel(pixel, sampler, texels, position, footprint);
            }
        }
    }
}

/// Draw a span using the general screen-ray path, including nonlinear refraction or equidistant mapping.
fn draw_screen_span(pixels: &mut [Texel], mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>, texels: &[Texel], first_column: usize, row: usize) {
    // Preserve the existing per-pixel transform and its numerical evaluation order.
    let mut rays = RowRays::new(mapping.screen, first_column as f64, row as f64);
    for pixel in pixels {
        let (numerator, spread, factor) = rays.current(mapping.screen);
        rays.advance();
        if let Some((position, footprint)) = mapping.tile_of_overlapping_footprint(numerator, spread, factor) {
            sample_and_blend_pixel(pixel, sampler, texels, position, footprint);
        }
    }
}

/// Apply the common light rejection, coordinate convention, box filter and screen composition.
#[inline]
fn sample_and_blend_pixel(pixel: &mut Texel, sampler: &TileSampler<'_>, texels: &[Texel], position: Point, footprint: f64) {
    // Convert integer-centred tile coordinates to the sampler's edge coordinates exactly once.
    if footprint <= 3.0 && !sampler.may_hold_light_near(position) { return; }
    let sample_position = position.map(|value| value + 0.5);
    let Some(rgba) = sampler.sample(texels, sample_position, [footprint, footprint]) else { return; };

    // Every region uses the same commutative blend as completed worker images.
    blend_screen_pixel(pixel, Texel { rgb: [rgba[0], rgba[1], rgba[2]], opacity: rgba[3] });
}

/// A stereographic row already rotated into a tile's axes, with first-order aberration included.
/// All three sequences are quadratic in screen column. Refraction would break that property.
/// Each step is the change to the next pixel; each curve is the change in that step.
struct TransformedRow {
    direction: Vector3,
    direction_step: Vector3,
    direction_curve: Vector3,
    length: f64,
    length_step: f64,
    length_curve: f64,
    spread: f64,
    spread_step: f64,
    spread_curve: f64,
}

impl TransformedRow {
    fn new(mapping: &RegionMapping<'_>, first_column: f64, row: f64) -> Self {
        // Use the existing screen-row differences, transforming their coefficients only once per span.
        debug_assert!(!mapping.atmosphere);
        let RowRays::Stereographic { numerator, numerator_step, numerator_curve, spread, spread_step, spread_curve } =
            RowRays::new(mapping.screen, first_column, row)
        else {
            unreachable!("transformed rows require stereographic projection");
        };

        // Keep the general mapper's first-order length approximation; do not normalize the direction.
        Self {
            direction: mapping.to_tile.apply(numerator) - mapping.aberration * spread,
            direction_step: mapping.to_tile.apply(numerator_step) - mapping.aberration * spread_step,
            direction_curve: mapping.to_tile.apply(numerator_curve) - mapping.aberration * spread_curve,
            length: spread - numerator.dot(mapping.aberration_horizon),
            length_step: spread_step - numerator_step.dot(mapping.aberration_horizon),
            length_curve: spread_curve - numerator_curve.dot(mapping.aberration_horizon),
            spread,
            spread_step,
            spread_curve,
        }
    }

    #[inline]
    fn project_footprint(&self, mapping: &RegionMapping<'_>) -> Option<(Point, f64)> {
        // Match the general mapper's singularity guard before computing tile coordinates or filter size.
        let denominator = self.length + self.direction.z;
        if denominator > 1e-9 * self.length {
            mapping.project_overlapping_footprint(self.direction, self.length, denominator, 2.0 / self.spread)
        } else {
            None
        }
    }

    #[inline]
    fn advance(&mut self) {
        // Consume the current values first, then advance each quadratic sequence to the next pixel.
        self.direction = self.direction + self.direction_step;
        self.direction_step = self.direction_step + self.direction_curve;
        self.length += self.length_step;
        self.length_step += self.length_curve;
        self.spread += self.spread_step;
        self.spread_step += self.spread_curve;
    }
}

/// The apparent directions along one screen row, stepped one column at a time by forward differences: the
/// stereographic numerator and spread are quadratics in the column, the equidistant pixel's squared distance
/// from the centre is one too (its direction then comes from the table).
enum RowRays {
    Stereographic { numerator: Vector3, numerator_step: Vector3, numerator_curve: Vector3, spread: f64, spread_step: f64, spread_curve: f64 },
    Equidistant { x: f64, x_step: f64, y: f64, squared: f64, squared_step: f64, squared_curve: f64 },
}

impl RowRays {
    fn new(screen: &Screen, column: f64, row: f64) -> Self {
        // Second differences of a quadratic are constant; the first differences start from the first column.
        match screen.camera.kind {
            ProjectionKind::Stereographic => {
                let (numerator, spread, _) = screen.ray(column, row);
                let (next, spread_next, _) = screen.ray(column + 1.0, row);
                let (after, spread_after, _) = screen.ray(column + 2.0, row);
                Self::Stereographic {
                    numerator,
                    numerator_step: next - numerator,
                    numerator_curve: after - next * 2.0 + numerator,
                    spread,
                    spread_step: spread_next - spread,
                    spread_curve: spread_after - 2.0 * spread_next + spread,
                }
            }
            ProjectionKind::Equidistant => {
                let x_step = 1.0 / screen.radius;
                let (x, y) = ((column - screen.radius) * x_step, (screen.radius - row) * x_step);
                Self::Equidistant { x, x_step, y, squared: x * x + y * y, squared_step: 2.0 * x * x_step + x_step * x_step, squared_curve: 2.0 * x_step * x_step }
            }
        }
    }
    #[inline]
    fn current(&self, screen: &Screen) -> (Vector3, f64, f64) {
        match *self {
            Self::Stereographic { numerator, spread, .. } => (numerator, spread, 2.0 / spread),
            Self::Equidistant { x, y, squared, .. } => (screen.equidistant_ray(x, y, squared), 1.0, FRAC_PI_2),
        }
    }
    #[inline]
    fn advance(&mut self) {
        match self {
            Self::Stereographic { numerator, numerator_step, numerator_curve, spread, spread_step, spread_curve } => {
                *numerator = *numerator + *numerator_step;
                *numerator_step = *numerator_step + *numerator_curve;
                *spread += *spread_step;
                *spread_step += *spread_curve;
            }
            Self::Equidistant { x, x_step, squared, squared_step, squared_curve, .. } => {
                *x += *x_step;
                *squared += *squared_step;
                *squared_step += *squared_curve;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::projection::{Frame, project_camera};
    use super::*;
    use crate::formulas::{J2000, Observer};
    use crate::constants::GRID_DEPTH;
    use crate::model::{RenderOptions, ViewCenter, fixtures, hash_direction, region_caps};
    use crate::observation::observe;
    use crate::regions::refresh_regions;
    use ProjectionKind::{Equidistant, Stereographic};

    fn tokyo() -> Observer { Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() } }
    pub(super) fn observation_at(fov: f64, atmosphere: bool, center: ViewCenter, projection: ProjectionKind) -> (View, Observation) {
        let view = View { center, fov_degrees: fov, projection };
        let date = crate::formulas::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
        let time = crate::formulas::utc_jd_to_time_ms(crate::formulas::datetime_to_julian_date(&date)).unwrap();
        (view, observe(&tokyo(), atmosphere, &view, time, crate::formulas::evaluate_earth_state(time), &mut StepTimes::default()))
    }
    /// The screen pixel of a texel position through the forward chain: the oracle for the inverse mapping.
    fn forward(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, size: usize, position: Point) -> Option<Point> {
        let apparent = observation.apparent_star_direction(tile.direction_of(position[0], position[1]));
        let point = project_camera(camera, apparent)?;
        let radius = (size as f64 - 1.0) * 0.5;
        Some([radius + point.x * radius, radius - point.y * radius])
    }
    fn settings() -> RenderOptions { RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false } }
    pub(super) fn facing(azimuth: f64, tilt: f64) -> ViewCenter { ViewCenter::Facing { azimuth, tilt } }

    #[test]
    fn every_pixel_maps_back_to_the_direction_the_forward_chain_projects() {
        // Regions across the sky at every zoom and both projections: the pixel's texel, pushed forward again,
        // lands on the pixel.
        let size = 1116;
        let mut checked = 0;
        for (fov, atmosphere, center, projection) in [(10.0, false, facing(5.83, 0.0), Stereographic), (60.0, true, facing(5.83, 0.3), Stereographic),
            (180.0, true, ViewCenter::Zenith, Stereographic), (300.0, false, facing(1.0, -0.2), Stereographic), (359.0, true, facing(2.0, 0.5), Stereographic),
            (10.0, true, facing(5.83, 0.0), Equidistant), (180.0, false, ViewCenter::Zenith, Equidistant), (360.0, true, facing(2.0, 0.5), Equidistant), (360.0, false, facing(1.0, -0.2), Equidistant)] {
            let (view, observation) = observation_at(fov, atmosphere, center, projection);
            let camera = prepare_camera(&view);
            let screen = Screen::new(camera, size);
            for region in (0..region_caps().len()).step_by(97) {
                let tile = TileGeometry::new(region);
                let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue; };
                let (top, bottom) = mapping.rows();
                for row in (top..=bottom).step_by(7) {
                    for (first, last) in mapping.columns_of(row).into_iter().flatten() {
                        for column in (first..=last).step_by(11) {
                            let Some((position, footprint)) = mapping.tile_position(column as f64, row as f64) else { continue; };
                            let back = forward(&tile, &observation, camera, size, position).unwrap();
                            let error = (back[0] - column as f64).hypot(back[1] - row as f64);
                            let from_centre = (column as f64 - screen.radius).hypot(row as f64 - screen.radius);
                            let tolerance = if projection == Equidistant && from_centre > screen.radius - 2.0 { 0.5 } else { 0.02 }; // the rim is the antipode: direction errors turn into screen angles there
                            assert!(error < tolerance, "fov={fov} {projection:?} atmosphere={atmosphere} region={region} pixel=({column}, {row}): back at {back:?}, {error} px off");

                            // The footprint is the texel distance covered by one pixel step, to a few percent;
                            // the equidistant screen covers less across the radius than along it.
                            if let (Some((right, _)), Some((left, _))) = (mapping.tile_position(column as f64 + 0.5, row as f64), mapping.tile_position(column as f64 - 0.5, row as f64)) {
                                let measured = (right[0] - left[0]).hypot(right[1] - left[1]);
                                let angle = from_centre / screen.radius * FRAC_PI_2 / camera.scale;
                                let least = if projection == Equidistant && angle > 0.0 { angle.sin() / angle } else { 1.0 };
                                assert!(measured < 1.05 * footprint + 1e-6 && measured > 0.95 * least * footprint - 1e-6, "fov={fov} {projection:?} region={region}: footprint {footprint} vs {measured}");
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 4000, "{checked}");
    }

    #[test]
    fn stepped_rows_match_direct_evaluation_and_tile_row_estimates_hold_at_every_pixel() {
        // Forward differences must agree with the per-pixel formula to a ten-thousandth of a texel, and the
        // tile-row estimate must contain the tile rows of every pixel of the screen row.
        let size = 1116;
        for (fov, atmosphere, center, projection) in [(10.0, true, facing(5.83, 0.0), Stereographic), (60.0, false, facing(5.83, 0.3), Stereographic),
            (180.0, true, ViewCenter::Zenith, Stereographic), (359.0, false, facing(2.0, 0.5), Stereographic), (60.0, true, facing(5.83, 0.3), Equidistant), (360.0, false, facing(2.0, 0.5), Equidistant)] {
            let (view, observation) = observation_at(fov, atmosphere, center, projection);
            let screen = Screen::new(prepare_camera(&view), size);
            let mut rows_checked = 0;
            for region in (0..region_caps().len()).step_by(13) {
                let tile = TileGeometry::new(region);
                let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue; };
                let (top, bottom) = mapping.rows();
                for row in top..=bottom {
                    for (first, last) in mapping.columns_of(row).into_iter().flatten() {
                        let estimate = mapping.tile_rows_of(row, first, last);
                        let mut rays = RowRays::new(&screen, first as f64, row as f64);
                        let (mut lowest, mut highest) = (f64::INFINITY, f64::NEG_INFINITY);
                        for column in first..=last {
                            let (numerator, spread, factor) = rays.current(&screen);
                            rays.advance();
                            let stepped = mapping.tile_of(numerator, spread, factor);
                            let (numerator, spread, factor) = screen.ray(column as f64, row as f64);
                            let direct = mapping.tile_of(numerator, spread, factor);
                            match (stepped, direct) {
                                (Some((_, _)), Some((b, _))) if b[0].abs().max(b[1].abs()) > 1000.0 => {}   // near the singular pixel the magnified positions are far outside anyway
                                (Some((a, fa)), Some((b, fb))) => {
                                    let scale = if b[0].abs().max(b[1].abs()) < 100.0 { 100.0 } else { 10.0 * b[0].abs().max(b[1].abs()) }; // far outside the tile only the magnified positions' relative agreement matters
                                    assert!((a[0] - b[0]).abs() < 1e-6 * scale && (a[1] - b[1]).abs() < 1e-6 * scale && (fa - fb).abs() < 1e-6 * fb.max(1.0) * scale, "fov={fov} {projection:?} region={region} ({column}, {row}): {a:?} vs {b:?}, footprints {fa} vs {fb}");
                                    if RegionMapping::inside_tile(b) {
                                        lowest = lowest.min(b[1] - fb.max(1.0) * 0.5);
                                        highest = highest.max(b[1] + fb.max(1.0) * 0.5);
                                    }
                                }
                                (None, None) => {}
                                other => panic!("fov={fov} {projection:?} region={region} ({column}, {row}): {other:?}"),
                            }
                        }
                        if lowest <= highest {
                            let (low, high) = estimate.unwrap_or_else(|| panic!("fov={fov} region={region} row {row}: tile rows {lowest}..{highest} estimated as none"));
                            assert!(low as f64 <= lowest.max(0.0) + 1e-9 && high as f64 + 1.0 >= highest.min(63.0), "fov={fov} region={region} row {row}: tile rows {lowest}..{highest} estimated as {low}..={high}");
                            rows_checked += 1;
                        }
                    }
                }
            }
            assert!(rows_checked > 100, "fov={fov}: {rows_checked}");
        }
    }

    #[test]
    fn the_candidate_area_contains_every_covered_pixel() {
        // Mapping the whole disc finds the covered pixels; all of them lie inside the region's candidate area,
        // which is not wasteful: the circle at most three times the covered count, the annulus sector six.
        let size = 301;
        for (fov, atmosphere, angle, projection) in [(60.0, true, 0.2, Stereographic), (60.0, true, 0.45, Stereographic), (180.0, false, 1.2, Stereographic), (300.0, true, 2.2, Stereographic),
            (60.0, false, 0.0, Equidistant), (60.0, true, 0.45, Equidistant), (360.0, false, 1.5, Equidistant), (360.0, true, 3.0, Equidistant)] {
            let (view, observation) = observation_at(fov, atmosphere, facing(5.83, 0.1), projection);
            let camera = prepare_camera(&view);
            let screen = Screen::new(camera, size);
            let apparent = camera.forward * f64::cos(angle) + (camera.up * 0.6 + camera.right * 0.8) * f64::sin(angle); // `angle` from the view centre
            let region = hash_direction(GRID_DEPTH, observation.to_catalog.apply(apparent));
            let tile = TileGeometry::new(region);
            let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
            assert_eq!(matches!(mapping.image, Image::Circle { .. }), projection == Stereographic, "{projection:?} {angle}: {:?}", mapping.image);
            let everywhere = RegionMapping { image: Image::WholeDisc, ..mapping };
            let (mut covered, mut candidates) = (0, 0);
            let (top, bottom) = mapping.rows();
            for row in 0..size {
                let runs = mapping.columns_of(row);
                candidates += runs.into_iter().flatten().map(|(first, last)| last - first + 1).sum::<usize>();
                for (first, last) in everywhere.columns_of(row).into_iter().flatten() {
                    for column in first..=last {
                        if everywhere.tile_position(column as f64, row as f64).is_none() { continue; }
                        covered += 1;
                        let inside = (top..=bottom).contains(&row) && runs.into_iter().flatten().any(|(first, last)| (first..=last).contains(&column));
                        assert!(inside, "fov={fov} {projection:?} {angle}: covered pixel ({column}, {row}) outside the candidate area {:?} rows {top}..={bottom} / {runs:?}", mapping.image);
                    }
                }
            }
            let limit = if projection == Stereographic { 3 } else { 6 };
            assert!(covered > 20 && candidates < limit * covered + 60, "fov={fov} {projection:?} {angle}: {candidates} candidates for {covered} covered pixels"); // plus the half-pixel margins of a tiny region
        }
    }

    #[test]
    fn a_star_lands_where_the_point_renderer_puts_it_and_an_empty_selection_clears() {
        let time = crate::formulas::utc_jd_to_time_ms(J2000).unwrap();
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        for (fov, atmosphere, projection) in [(180.0, false, Stereographic), (60.0, true, Stereographic), (10.0, true, Stereographic), (10.0, true, Equidistant), (300.0, false, Equidistant)] {
            let view = View { center: facing(1.0, 0.02), projection, fov_degrees: fov };
            let observation = observe(&site, atmosphere, &view, time, crate::formulas::evaluate_earth_state(time), &mut StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(GRID_DEPTH, direction);
            let star = fixtures::row(direction, 0.0, 3.0, 1);
            let mut data = fixtures::layout(&[(region, 0, star)]);
            let selected = Selected { regions: vec![region], whole_sky: false };
            refresh_regions(
                &mut data.star_regions, &data.constellation_stars,
                &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
                &mut data.star_opacities, &mut data.tile_bytes,
                &selected.regions, time, view.fov_degrees, settings().magnitude_threshold,
                &mut StepTimes::default(),
            );
            let mut frame = Frame::square(401);
            let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
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
                "{fov} {projection:?}: center {center:?} / {total}, expected {expected:?}, tolerance {tolerance}");
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));

            let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &(Selected { regions: Vec::new(), whole_sky: false }).regions, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }

    #[test]
    fn texel_centres_render_at_their_forward_projected_positions() {
        // Aim a narrow view at one region so a half-texel error becomes several screen pixels.
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: -0.2 });
        let tile = TileGeometry::new(region);
        let size = 501;
        for projection in [Stereographic, Equidistant] {
            let (_, initial) = observation_at(2.0, false, facing(0.0, 0.0), projection);
            let centre = Horizontal::from_vector(initial.apparent_star_direction(tile.center()));
            let (view, observation) = observation_at(2.0, false, facing(centre.azimuth, centre.altitude), projection);

            // One lit texel has a known centre at integer tile coordinates (32, 32).
            let mut data = Data::default();
            let image = &mut data.star_regions[region].merged_tile;
            image.tile.texels = vec![Texel::default(); TILE_EDGE_TEXELS.pow(2)];
            image.tile.texels[32 * TILE_EDGE_TEXELS + 32] = Texel { rgb: [1.0; 3], opacity: 1.0 };
            image.lit_rows = vec![0; TILE_EDGE_TEXELS];
            image.lit_rows[32] = 1 << 32;
            let selected = Selected { regions: vec![region], whole_sky: false };
            let stars = project_test_image(size, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());

            // Compare the rendered light's centre with the independent forward geometry, not the inverse mapper.
            let expected = forward(&tile, &observation, prepare_camera(&view), size, [32.0, 32.0]).unwrap();
            let mut opacity_sum = 0.0;
            let mut weighted_position = [0.0; 2];
            for (index, texel) in stars.iter().enumerate() {
                let opacity = f64::from(texel.opacity);
                opacity_sum += opacity;
                weighted_position[0] += (index % size) as f64 * opacity;
                weighted_position[1] += (index / size) as f64 * opacity;
            }
            assert!(opacity_sum > 0.0);
            let actual = weighted_position.map(|value| value / opacity_sum);
            let error = (actual[0] - expected[0]).hypot(actual[1] - expected[1]);
            assert!(error < 0.05, "{projection:?}: expected {expected:?}, rendered {actual:?}, error {error} pixels");
        }
    }

    #[test]
    fn candidate_bounds_include_the_outer_texel_edges() {
        // Magnify each tile corner enough to distinguish its last texel centre from its outer edge.
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: -0.2 });
        let tile = TileGeometry::new(region);
        let size = 2001;
        for projection in [Stereographic, Equidistant] {
            let (_, initial) = observation_at(10.0, false, facing(0.0, 0.0), projection);
            let centre = Horizontal::from_vector(initial.apparent_star_direction(tile.center()));
            let (view, observation) = observation_at(10.0, false, facing(centre.azimuth, centre.altitude), projection);
            let screen = Screen::new(prepare_camera(&view), size);
            let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
            let (first_row, last_row) = mapping.rows();

            // These points lie in the outer quarter of each corner texel, still inside the tile.
            for tile_row in [-0.25, 63.25] {
                for tile_column in [-0.25, 63.25] {
                    let projected = forward(&tile, &observation, screen.camera, size, [tile_column, tile_row]).unwrap();
                    let (column, row) = (projected[0].round() as usize, projected[1].round() as usize);
                    let (position, _) = mapping.tile_position(column as f64, row as f64).expect("the pixel centre remains inside the tile");
                    assert!((first_row..=last_row).contains(&row), "{projection:?}: row {row} excluded for {position:?}");
                    let included = mapping.columns_of(row).into_iter().flatten().any(|(first, last)| (first..=last).contains(&column));
                    assert!(included, "{projection:?}: pixel ({column}, {row}) excluded for tile position {position:?}");
                }
            }
        }
    }

    #[test]
    fn boundary_candidates_and_row_skips_match_a_full_pixel_scan() {
        // In nonsingular views, compare the optimized candidate spans against every pixel in the disc.
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: -0.2 });
        let tile = TileGeometry::new(region);
        let texels = vec![Texel { rgb: [0.5, 0.25, 0.125], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
        sampler.prepare(&[u64::MAX; TILE_EDGE_TEXELS], None);
        for projection in [Stereographic, Equidistant] {
            for (fov, size, atmosphere) in [(60.0, 23, false), (180.0, 101, false), (60.0, 201, true)] {
                let (_, initial) = observation_at(fov, atmosphere, facing(0.0, 0.0), projection);
                let centre = Horizontal::from_vector(initial.apparent_star_direction(tile.center()));
                let (view, observation) = observation_at(fov, atmosphere, facing(centre.azimuth, centre.altitude), projection);
                let screen = Screen::new(prepare_camera(&view), size);
                let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
                let mut actual = vec![Texel::default(); size * size];
                draw_region(&mut actual, &mapping, &sampler, &texels);

                // This reference bypasses candidate bounds, row masks and the pre-division overlap check.
                for row in 0..size {
                    for column in 0..size {
                        let distance = (column as f64 - screen.radius).powi(2) + (row as f64 - screen.radius).powi(2);
                        if distance > screen.radius * screen.radius + 1e-9 { continue; }
                        let (numerator, spread, factor) = screen.ray(column as f64, row as f64);
                        let expected = mapping.tile_of(numerator, spread, factor).and_then(|(position, footprint)| {
                            sampler.sample(&texels, position.map(|value| value + 0.5), [footprint; 2])
                        }).unwrap_or([0.0; 4]);
                        let pixel = actual[row * size + column];
                        for (value, expected) in pixel.rgb.into_iter().chain([pixel.opacity]).zip(expected) {
                            assert!((value - expected).abs() < 1e-5, "{projection:?} fov={fov} size={size} pixel=({column},{row}): {value} != {expected}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn subpixel_tiles_keep_partial_light_as_the_camera_moves() {
        // Reproduce the review's region: a dense tile smaller than a pixel must retain partial light.
        let region = 3000;
        let tile = TileGeometry::new(region);
        let mut data = Data::default();
        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [0.5, 0.25, 0.125], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: false };
        for (projection, fov, size) in [(Stereographic, 351.6, 401), (Equidistant, 360.0, 51)] {
            let (_, initial) = observation_at(fov, false, facing(0.0, 0.0), projection);
            let centre = Horizontal::from_vector(initial.apparent_star_direction(tile.center()));
            for offset in [0.0, 0.05, 0.1, 0.2] {
                let (view, observation) = observation_at(fov, false, facing(centre.azimuth + offset, centre.altitude), projection);
                let stars = project_test_image(size, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
                let total: f64 = stars.iter().map(|texel| f64::from(texel.opacity)).sum();
                assert!(total > 0.0 && total < 0.5, "{projection:?} offset={offset}: {total}");

                // The forward outline provides an independent area check; allow the existing box-filter approximation.
                let camera = prepare_camera(&view);
                let corners = [[-0.5, -0.5], [63.5, -0.5], [63.5, 63.5], [-0.5, 63.5]]
                    .map(|position| forward(&tile, &observation, camera, size, position).unwrap());
                let area = (0..4).map(|index| {
                    let next = (index + 1) % 4;
                    corners[index][0] * corners[next][1] - corners[index][1] * corners[next][0]
                }).sum::<f64>().abs() * 0.5;
                assert!((total - area).abs() < area * 0.15, "{projection:?} offset={offset}: opacity {total}, projected area {area}");
                for texel in stars {
                    assert!((texel.rgb[0] - texel.opacity * 0.5).abs() < 1e-7);
                    assert!((texel.rgb[1] - texel.opacity * 0.25).abs() < 1e-7);
                }
            }
        }
    }

    #[test]
    fn neighboring_regions_screen_blend_once_per_pixel() {
        // Two half-opaque regions over the same sky give 0.75, not four half-blends; an opaque one stays bounded.
        for projection in [Stereographic, Equidistant] {
            let (_, observation) = observation_at(60.0, false, facing(5.83, 0.1), projection);
            let region = hash_direction(GRID_DEPTH, observation.patch.center);
            let geometry = TileGeometry::new(region);
            let neighbour = (1..=20).map(|step| {
                hash_direction(GRID_DEPTH, (geometry.center() + geometry.east() * (step as f64 * 0.01)).normalized())
            }).find(|&candidate| candidate != region).expect("a distinct neighboring region");
            let boundary = (geometry.center() + TileGeometry::new(neighbour).center()).normalized();
            let horizontal = Horizontal::from_vector(observation.apparent_star_direction(boundary));
            let (view, observation) = observation_at(60.0, false, facing(horizontal.azimuth, horizontal.altitude), projection);
            let mut data = Data::default();
            for (color, which) in [([0.5, 0.0, 0.0], region), ([0.0, 0.0, 0.5], neighbour)] {
                let image = &mut data.star_regions[which].merged_tile;
                image.tile.texels = vec![Texel { rgb: color, opacity: 0.5 }; TILE_EDGE_TEXELS.pow(2)];
                image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
            }
            let regions = vec![region, neighbour];
            let selected = Selected { regions, whole_sky: false };
            let mut frame = Frame::square(201);
            let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);
            let centre = frame.star(100, 100);
            assert!((centre.opacity - 0.75).abs() < 1e-6 && (centre.rgb[0] - 0.5).abs() < 1e-6
                && (centre.rgb[2] - 0.5).abs() < 1e-6, "{projection:?}: {centre:?}");
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0 && texel.rgb.iter().all(|&c| c <= 1.0)));

            let image = &mut data.star_regions[region].merged_tile;
            image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
            let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);
            assert_eq!(frame.star(100, 100).opacity, 1.0);
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0 && texel.rgb.iter().all(|&c| c <= 1.0)));
        }
    }

    #[test]
    fn a_region_behind_the_camera_draws_only_at_the_rim() {
        // At 359° the region around the view's antipode covers the exterior of a circle:
        // only the outer ring beyond its nearest corner is tested, never the centre. (Stereographic stretches
        // the last degrees over most of the disc: the ring starts about a third of the way out.)
        let (view, observation) = observation_at(359.0, false, facing(2.0, 0.5), Stereographic);
        let camera = prepare_camera(&view);
        let region = hash_direction(GRID_DEPTH, observation.to_catalog.apply(camera.forward * -1.0));
        let tile = TileGeometry::new(region);
        let size = 201;
        let screen = Screen::new(camera, size);
        let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
        assert!(matches!(mapping.image, Image::OutsideCircle { .. }));
        let mut data = Data::default();
        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: true };
        let mut frame = Frame::square(size);
        let projected_stars = project_test_image(size, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        let corner = 2.0 * (TileGeometry::middle() * SQRT_2 / (2.0 * tile.density())).atan();
        let offset = observation.apparent_star_direction(tile.center()).dot(camera.forward * -1.0).clamp(-1.0, 1.0).acos();
        let inner = 100.0 * camera.scale * ((PI - corner - offset) / 2.0).tan();             // the screen radius of the tile's nearest point
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
    fn a_cap_edge_through_the_antipode_keeps_the_front_of_the_view_empty() {
        // Aim exactly at a cap boundary's antipode: its unbounded image must be a half-plane, not the full disc.
        let (_, observation) = observation_at(359.0, false, facing(2.0, 0.5), Stereographic);
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let tile = TileGeometry::new(region);
        let apparent = observation.apparent_star_direction(tile.center());
        let tangent = apparent.cross(Vector3 { x: 0.0, y: 0.0, z: 1.0 }).normalized();
        let reach = 2.0 * ((TileGeometry::middle() + 1.0) * SQRT_2 / (2.0 * tile.density())).atan();
        let behind_edge = Horizontal::from_vector(-(apparent * reach.cos() + tangent * reach.sin()));
        let view = View { center: facing(behind_edge.azimuth, behind_edge.altitude), projection: Stereographic, fov_degrees: 359.0 };
        let screen = Screen::new(prepare_camera(&view), 201);
        let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
        assert!(matches!(mapping.image, Image::HalfPlane { .. }));

        // An opaque rear tile can light its visible side but must not leak into the centre via a huge filter.
        let texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
        sampler.prepare(&[u64::MAX; TILE_EDGE_TEXELS], None);
        let mut stars = vec![Texel::default(); 201 * 201];
        draw_region(&mut stars, &mapping, &sampler, &texels);
        assert!(stars[100 * 201 + 100].is_empty());
        assert!(stars.iter().any(|texel| texel.opacity > 0.0));
    }

    #[test]
    fn an_equidistant_antipode_tile_draws_a_ring_at_the_rim_only() {
        // Aim the camera so one tile corner lies exactly behind it: the equidistant rim is that one direction,
        // and the tile around it becomes a ring along the rim, never a chord through the disc.
        let (_, observation) = observation_at(360.0, false, facing(2.0, 0.5), Equidistant);
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let tile = TileGeometry::new(region);
        let apparent = observation.apparent_star_direction(tile.direction_of(-0.5, -0.5));
        let behind = crate::formulas::Horizontal::from_vector(-apparent);
        let view = View { center: facing(behind.azimuth, behind.altitude), projection: Equidistant, fov_degrees: 360.0 };
        let camera = prepare_camera(&view);
        let size = 201;
        let screen = Screen::new(camera, size);
        let mapping = RegionMapping::new(&tile, &observation, &screen, None).unwrap();
        let Image::Annulus { inner, outer, .. } = mapping.image else { panic!("{:?}", mapping.image) };
        assert!(inner > 85.0 && outer >= 100.0, "{inner} {outer}");
        let mut data = Data::default();
        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: true };
        let mut frame = Frame::square(size);
        let projected_stars = project_test_image(size, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        let mut lit = 0;
        for (index, texel) in frame.stars().iter().enumerate() {
            assert!(texel.opacity.is_finite() && texel.opacity <= 1.0);
            if texel.opacity > 0.0 {
                lit += 1;
                let distance = ((index % size) as f64 - 100.0).hypot((index / size) as f64 - 100.0);
                assert!(distance > inner - 1.0, "rear tile drawn inside the disc at {index}, {distance} from the centre");
            }
        }
        assert!(lit > 200 && frame.star(100, 100).is_empty(), "{lit}");
    }

    #[test]
    #[ignore = "timing only; uses embedded BSC5 and synthetic dense tiles, never external datasets"]
    fn time_pixel_mapping() {
        // Use one prepared catalog and the screenshot's camera; only projection is timed.
        let mut report = crate::progress::Report::quiet();
        let catalog = crate::catalog::load_embedded_catalog(&mut report).unwrap();
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut report).unwrap();
        let original = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        for (fov, projection) in [(180.0, Stereographic), (351.6, Stereographic), (360.0, Equidistant)] {
            let (view, observation) = observation_at(fov, false, facing(334_f64.to_radians(), 0.0), projection);
            let mut data = original.clone();
            let selected = crate::selection::select_regions(&data.star_regions, &observation.patch, observation.time_ms, &mut StepTimes::default());
            let settings = RenderOptions { magnitude_threshold: 9.0, ..settings() };
            refresh_regions(
                &mut data.star_regions, &data.constellation_stars,
                &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
                &mut data.star_opacities, &mut data.tile_bytes,
                &selected.regions, observation.time_ms, view.fov_degrees, settings.magnitude_threshold,
                &mut StepTimes::default(),
            );
            let mut frame = Frame::new(1917, 1116, 1116, (10, 20));
            for dense in [false, true] {
                if dense {
                    for &region in &selected.regions {
                        let image = &mut data.star_regions[region].merged_tile;
                        image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS.pow(2)];
                        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
                        image.content_radius = None;
                        image.filter_summary = None;
                    }
                }
                let mut elapsed = 0.0;
                for run in 0..4 {
                    let start = std::time::Instant::now();
                    let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
                    frame.insert_projected_stars(&projected_stars);
                    if run > 0 { elapsed += start.elapsed().as_secs_f64(); }
                    std::hint::black_box(frame.stars());
                }
                eprintln!("Pixel warp fov={fov} {projection:?}, dense={dense}, regions={}: {:.3}ms; three frames after warmup", selected.regions.len(), elapsed * 1000.0 / 3.0);
                let mut detail = StepTimes::with_trace(true);
                let projected_stars = project_test_image(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut detail);
                frame.insert_projected_stars(&projected_stars);
                detail.measure("END");
                for step in detail.steps().iter().filter(|step| matches!(step.name, "Region mapping" | "Tile filter preparation" | "Tile rasterization")) {
                    eprintln!("  {}: {:.3}ms (separate traced frame)", step.name, step.average_seconds * 1000.0);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "content_bounds_tests.rs"]
pub(super) mod content_bounds_tests;

#[cfg(test)]
#[path = "row_stepping_tests.rs"]
pub(super) mod row_stepping_tests;
