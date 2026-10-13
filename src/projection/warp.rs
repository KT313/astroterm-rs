//! Step 5 of the frame: the inverse ray-sampling implementation.
//!
//! Inputs: the frame (its star layer and its per-view ray table), the region caches, the selected regions, the
//! observation and the view. Output: the star layer of the frame, the light of the region tiles under every
//! screen pixel. Runs every frame; the cost is proportional to the screen pixels, never to the star count.
//!
//! Per pixel: the stored camera-frame ray is turned into the horizon frame with the view's camera basis, the
//! refraction is removed through a fixed table, the observation's rotation takes it into catalog axes and the
//! aberration shift is removed; the cube-map lookup names the region, whose tile is sampled bilinearly when a
//! texel covers at least a pixel and otherwise by adding up the texels under the pixel (tiles have a fixed size,
//! plan section 4). Within the drift and glyph margin of a region edge the neighbouring regions' tiles are sampled
//! too, so that a star that straddles a border or drifted across it is complete. The tiles already include the per-star zoom boost;
//! tone mapping is left to the frame output.
use crate::timing::StepTimes;
use crate::astro::{Horizontal, Vector3, apply_refraction};
use crate::constants::{CELL_COUNT, GRID_DEPTH, STELLAR_DRIFT_MARGIN, TILE_EDGE_TEXELS};
use crate::model::{ProjectionKind, View};
use crate::observation::Observation;
use crate::projection::prepare_camera;
use crate::regions::{RegionCaches, STAR_GLYPH_RADIUS, Texel, TileFrame, TileGeometry, tile_frames};
use crate::text::{TextRasterizer, create_text_rasterizer, set_text_cell_size};
use crate::selection::Selected;
use std::f64::consts::FRAC_PI_2;
use std::sync::OnceLock;

const EDGE: usize = 1 << GRID_DEPTH;                                       // regions per cube-face edge
const REFRACTION_TABLE_SIZE: usize = 8192;                                 // entries over sin(apparent altitude); 0.002″ from the direct solve
const REFRACTION_ITERATIONS: usize = 6;                                    // fixed-point steps of the inverse refraction

/// The square of the frame that shows the sky: the projection's radius-1 circle is inscribed in it. The rest of
/// the frame is margin for text (owner's layout: a centred square sky, the metadata panel at the top left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkyArea {
    pub x: usize,                           // left column of the square in frame pixels
    pub y: usize,                           // top row
    pub size: usize,                        // side in pixels
}

impl SkyArea {
    /// The largest centred square of a `width` × `height` frame.
    pub fn centred(width: usize, height: usize) -> Self {
        let size = width.min(height);
        Self { x: (width - size) / 2, y: (height - size) / 2, size }
    }
    /// The frame pixel (row, column) of a sky-square cell, as `project_to_cell` numbers them.
    pub fn frame_pixel(&self, cell: (i32, i32)) -> (i32, i32) { (cell.0 + self.y as i32, cell.1 + self.x as i32) }
    /// Whether a sky cell and the three cells above and to the left of it lie inside the square (a star glyph fits).
    pub fn glyph_fits(&self, (row, column): (i32, i32)) -> bool { row > 0 && column > 0 && (row as usize) < self.size && (column as usize) < self.size }
    pub fn viewport(&self) -> crate::model::ProjectionViewport { crate::model::ProjectionViewport { width: self.size, height: self.size } }
}

/// The screen-sized buffers of a frame: the star layer step 5 fills, the overlay step 6 draws on top of it, the
/// text rasterizer the overlay's labels use, and the ray table kept per view.
pub struct Frame {
    width: usize,
    height: usize,
    sky: SkyArea,
    stars: Vec<Texel>,                      // row-major, width × height: light per pixel before tone mapping
    overlay: Vec<u8>,                       // premultiplied RGBA, transparent where nothing was drawn: points, lines and text over the stars
    text: TextRasterizer,
    text_cell: (u16, u16),                  // pixel size of one text cell: glyph size and line spacing
    rays: ViewTable,
}

impl Frame {
    /// A frame with the sky in the given square and text cells of `text_cell` pixels.
    pub fn new(width: usize, height: usize, sky: SkyArea, text_cell: (u16, u16)) -> Self {
        let mut text = create_text_rasterizer().expect("the bundled font parses");
        set_text_cell_size(&mut text, text_cell.0, text_cell.1);
        Self { width, height, sky, stars: vec![Texel::default(); width * height], overlay: vec![0; width * height * 4], text, text_cell, rays: ViewTable::default() }
    }

    /// A square frame that is all sky, with the default text cell: the shape of the tests and the parity fixtures.
    pub fn square(size: usize) -> Self { Self::new(size, size, SkyArea { x: 0, y: 0, size }, (10, 20)) }

    /// Adopt a new size and sky square; the layers are cleared and the ray table rebuilt on the next projection.
    pub fn resize(&mut self, width: usize, height: usize, sky: SkyArea) {
        if (width, height, sky) == (self.width, self.height, self.sky) { return; }
        self.width = width;
        self.height = height;
        self.sky = sky;
        self.stars.clear();
        self.stars.resize(width * height, Texel::default());
        self.overlay.clear();
        self.overlay.resize(width * height * 4, 0);
    }

    pub fn set_text_cell(&mut self, text_cell: (u16, u16)) {
        self.text_cell = text_cell;
        set_text_cell_size(&mut self.text, text_cell.0, text_cell.1);
    }

    pub fn width(&self) -> usize { self.width }
    pub fn height(&self) -> usize { self.height }
    pub fn sky(&self) -> SkyArea { self.sky }
    pub fn text_cell(&self) -> (u16, u16) { self.text_cell }
    pub fn stars(&self) -> &[Texel] { &self.stars }
    pub fn stars_mut(&mut self) -> &mut [Texel] { &mut self.stars }
    pub fn star(&self, row: usize, column: usize) -> Texel { self.stars[row * self.width + column] }
    pub fn overlay(&self) -> &[u8] { &self.overlay }
    /// The overlay and the text rasterizer together, for drawing text onto the overlay.
    pub fn overlay_and_text(&mut self) -> (&mut [u8], &mut TextRasterizer) { (&mut self.overlay, &mut self.text) }
    pub fn clear_overlay(&mut self) { self.overlay.fill(0); }
    pub fn rays(&self) -> &ViewTable { &self.rays }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("width", &self.width).field("height", &self.height).field("sky", &self.sky).field("text_cell", &self.text_cell).finish()
    }
}

/// What a ray table was built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewKey {
    pub projection: ProjectionKind,
    pub fov_degrees: f64,
    pub width: usize,
    pub height: usize,
    pub sky: SkyArea,
}

/// The per-view ray table (plan section 7): for every screen pixel its apparent direction in the camera frame
/// (components along the camera's right, up and forward axes). The rays of neighbouring pixels give each pixel's
/// footprint on the sky, so nothing else is stored.
///
/// **Key:** projection kind, field of view, frame size and sky square. **Invalidated by:** any key change (zoom, a
/// projection switch, a terminal resize); the table is then rebuilt. Panning, Earth's rotation and the observer do
/// not touch it because the rays are stored relative to the camera, and refraction is removed per frame through
/// a fixed table instead of being baked in.
#[derive(Clone, Debug, Default)]
pub struct ViewTable {
    key: Option<ViewKey>,
    rays: Vec<Ray>,
}

#[derive(Clone, Copy, Debug)]
struct Ray {
    direction: [f32; 3],                    // unit, camera frame (right, up, forward); NaN for a pixel outside the view
}

impl ViewTable {
    pub fn key(&self) -> Option<ViewKey> { self.key }

    /// Hold the rays of `view` for a `width` × `height` frame with the sky in `sky`, rebuilding only when the key
    /// changed. Returns whether it did.
    fn prepare(&mut self, view: &View, width: usize, height: usize, sky: SkyArea) -> bool {
        let key = ViewKey { projection: view.projection, fov_degrees: view.fov_degrees, width, height, sky };
        if self.key == Some(key) { return false; }
        let scale = prepare_camera(view).scale;
        let radius = (sky.size as f64 - 1.0) / 2.0;
        let (cx, cy) = (sky.x as f64 + radius, sky.y as f64 + radius);
        self.rays.clear();
        self.rays.reserve(width * height);
        for row in 0..height {
            for column in 0..width {
                // pixel to projection-plane coordinates, the inverse of `project_to_cell` on the sky square
                let px = if radius > 0.0 { (column as f64 - cx) / radius } else { 0.0 };
                let py = if radius > 0.0 { -(row as f64 - cy) / radius } else { 0.0 };
                let radius = px.hypot(py);
                if radius > 1.0 + 1e-9 { self.rays.push(Ray { direction: [f32::NAN; 3] }); continue; } // outside the rendered circle

                // inverse projection to an apparent unit direction in the camera frame
                let direction = match view.projection {
                    ProjectionKind::Stereographic => {
                        let (qx, qy) = (px / scale, py / scale);
                        let c = (1.0 - qx * qx - qy * qy) / (1.0 + qx * qx + qy * qy);         // r = scale × tan(a / 2)
                        [qx * (1.0 + c), qy * (1.0 + c), c]
                    }
                    ProjectionKind::Equidistant => {
                        let theta = radius * FRAC_PI_2 / scale;                                 // r = scale × a / (π / 2)
                        if theta >= std::f64::consts::PI - 1e-9 { self.rays.push(Ray { direction: [f32::NAN; 3] }); continue; } // the point behind the observer has no direction
                        let (s, c) = theta.sin_cos();
                        let (ux, uy) = if radius > 0.0 { (px / radius, py / radius) } else { (0.0, 0.0) };
                        [ux * s, uy * s, c]
                    }
                };
                self.rays.push(Ray { direction: direction.map(|v| v as f32) });
            }
        }
        self.key = Some(key);
        true
    }
}

/// Refraction removal as a table over sin(apparent altitude): per entry the true sin(altitude) and the factor that
/// rescales the horizontal components. Fixed geometry, built once.
struct RefractionTable {
    entries: Vec<(f32, f32)>,
}

fn refraction_table() -> &'static RefractionTable {
    static TABLE: OnceLock<RefractionTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries = (0..=REFRACTION_TABLE_SIZE).map(|i| {
            let apparent = (-1.0 + 2.0 * i as f64 / REFRACTION_TABLE_SIZE as f64).clamp(-1.0, 1.0).asin();
            let truth = remove_refraction(apparent);
            let horizontal_scale = if apparent.cos() > 1e-9 { truth.cos() / apparent.cos() } else { 1.0 };
            (truth.sin() as f32, horizontal_scale as f32)
        }).collect();
        RefractionTable { entries }
    })
}

/// The true altitude whose refracted value is `apparent`, by fixed-point iteration of the refraction formula.
fn remove_refraction(apparent: f64) -> f64 {
    let mut altitude = apparent;
    for _ in 0..REFRACTION_ITERATIONS {
        let lift = apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - altitude;
        altitude = apparent - lift;
    }
    altitude
}

impl RefractionTable {
    /// The true direction for an apparent unit horizon direction, keeping its azimuth; linear interpolation in z.
    #[inline]
    fn remove(&self, apparent: [f32; 3]) -> [f32; 3] {
        let position = (apparent[2].clamp(-1.0, 1.0) + 1.0) * 0.5 * REFRACTION_TABLE_SIZE as f32;
        let index = (position as usize).min(REFRACTION_TABLE_SIZE - 1);
        let weight = position - index as f32;
        let (a, b) = (self.entries[index], self.entries[index + 1]);
        let (sin_true, horizontal_scale) = (a.0 + (b.0 - a.0) * weight, a.1 + (b.1 - a.1) * weight);
        [apparent[0] * horizontal_scale, apparent[1] * horizontal_scale, sin_true]
    }
}

/// Fill the frame's star layer from the region tiles for this observation and view. Returns whether the ray
/// table had to be rebuilt.
pub(super) fn project_tiles_inverse(frame: &mut Frame, caches: &RegionCaches, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes) -> bool {
    let rebuilt = times.measure_detail("View ray table", || frame.rays.prepare(view, frame.width, frame.height, frame.sky));
    let (in_view, camera) = times.measure_detail("Warp preparation", || {
        let camera = prepare_camera(view);
        let mut in_view = vec![false; CELL_COUNT];
        for &region in &selected.regions { in_view[region] = true; }
        (in_view, camera)
    });
    let pack = |v: Vector3| [v.x as f32, v.y as f32, v.z as f32];
    let warp = Warp {
        caches,
        in_view: &in_view,
        frames: tile_frames(),
        spread: spread_table(),
        basis: [pack(camera.right), pack(camera.up), pack(camera.forward)],
        to_catalog: observation.to_catalog.0.map(|row| row.map(|v| v as f32)),
        beta: pack(observation.beta),
        refraction: observation.atmosphere.then(refraction_table),
        finest_density: tile_frames().iter().map(|tile| tile.density).fold(0.0, f32::max),
    };
    let (rays, width) = (frame.rays.rays.as_slice(), frame.width);
    times.measure_detail("Inverse pixel sampling", || {
        for (index, pixel) in frame.stars.iter_mut().enumerate() { *pixel = warp.pixel(rays, index, width); }
    });
    rebuilt
}

/// The per-frame constants of the warp.
struct Warp<'a> {
    caches: &'a RegionCaches,
    in_view: &'a [bool],                    // per region: selected this frame, so its tiles are current
    frames: &'a [TileFrame],
    spread: &'static [u16; EDGE],
    basis: [[f32; 3]; 3],                   // the camera's right, up and forward axes in the horizon frame
    to_catalog: [[f32; 3]; 3],
    beta: [f32; 3],
    refraction: Option<&'static RefractionTable>,
    finest_density: f32,                    // the highest texels per radian of any tile: above it a pixel never covers a whole texel
}

/// A direction located on the cube map: face, cell coordinates, position inside the cell, and the face
/// coordinates and major component needed to probe across cell edges.
#[derive(Clone, Copy)]
struct Located {
    face: usize,
    ix: usize,
    iy: usize,
    fx: f32,
    fy: f32,
    stretch: f32,                           // how much faster the face coordinates grow than the angle, at this point: 1 + u² + v²
    major: f32,                             // the absolute value of the direction's largest component
}

impl Warp<'_> {
    /// The light at one pixel.
    #[inline]
    fn pixel(&self, rays: &[Ray], index: usize, width: usize) -> Texel {
        let d = rays[index].direction;
        if d[0].is_nan() { return Texel::default(); }

        // the pixel's footprint: the rays of the next column and the next row (or the previous ones at an edge)
        let column = index % width;
        let step_column = neighbour_delta(rays, d, index, 1, column + 1 < width, column > 0);
        let step_row = neighbour_delta(rays, d, index, width, index + width < rays.len(), index >= width);
        let (Some(dc), Some(dr)) = (step_column, step_row) else { return Texel::default(); }; // a view one pixel wide has no footprint
        let catalog = self.catalog_direction(d);
        let pixel_angle = length(dc).max(length(dr));                                        // the pixel's width on the sky
        let footprint = (pixel_angle * self.finest_density > 1.0).then(|| [self.catalog_direction(add(d, dc)), self.catalog_direction(add(d, dr))]); // only where a pixel can cover more than a texel

        // the pixel's own region
        let located = locate(catalog);
        let region = cell_index(located, self.spread);
        let mut out = Texel::default();
        self.sample(region, catalog, footprint, &mut out);

        // within the drift and glyph margin of a region edge, the neighbours' tiles also cover this pixel
        let margin_angle = STELLAR_DRIFT_MARGIN as f32 + STAR_GLYPH_RADIUS as f32 / self.frames[region].density + pixel_angle;
        let margin_cells = margin_angle * located.stretch * (EDGE as f32 / 2.0);
        let dx = if located.fx < margin_cells { -1 } else if located.fx > 1.0 - margin_cells { 1 } else { 0 };
        let dy = if located.fy < margin_cells { -1 } else if located.fy > 1.0 - margin_cells { 1 } else { 0 };
        let shift = margin_angle * located.stretch * located.major;
        if dx != 0 { self.sample(self.neighbour(located, catalog, dx, 0, shift), catalog, footprint, &mut out); }
        if dy != 0 { self.sample(self.neighbour(located, catalog, 0, dy, shift), catalog, footprint, &mut out); }
        if dx != 0 && dy != 0 { self.sample(self.neighbour(located, catalog, dx, dy, shift), catalog, footprint, &mut out); }
        out
    }

    /// A camera-frame ray as a catalog direction: camera basis, refraction removed, rotation, aberration removed.
    /// Nothing downstream needs a unit vector, so there is no renormalisation.
    #[inline]
    fn catalog_direction(&self, d: [f32; 3]) -> [f32; 3] {
        let [right, up, forward] = self.basis;
        let mut horizon = [right[0] * d[0] + up[0] * d[1] + forward[0] * d[2], right[1] * d[0] + up[1] * d[1] + forward[1] * d[2], right[2] * d[0] + up[2] * d[1] + forward[2] * d[2]];
        if let Some(table) = self.refraction { horizon = table.remove(horizon); }
        let rotated = [dot(self.to_catalog[0], horizon), dot(self.to_catalog[1], horizon), dot(self.to_catalog[2], horizon)];
        [rotated[0] - self.beta[0], rotated[1] - self.beta[1], rotated[2] - self.beta[2]]
    }

    /// Add the light of `region`'s tile under the pixel to `out`. `footprint` holds the catalog directions of the
    /// next column's and the next row's pixel; with the pixel's own they span its footprint on the tile.
    #[inline]
    fn sample(&self, region: usize, catalog: [f32; 3], footprint: Option<[[f32; 3]; 2]>, out: &mut Texel) {
        if !self.in_view[region] { return; }
        let image = self.caches.region_image(region);
        let texels = image.texels();
        if texels.is_empty() { return; }
        let tile = &self.frames[region];
        let Some(centre) = tile_coordinates(tile, catalog) else { return; };
        if let Some(footprint) = footprint {
            let (Some(next_column), Some(next_row)) = (tile_coordinates(tile, footprint[0]), tile_coordinates(tile, footprint[1])) else { return; };
            let e1 = (next_column.0 - centre.0, next_column.1 - centre.1);                   // one pixel to the right, in texels
            let e2 = (next_row.0 - centre.0, next_row.1 - centre.1);                         // one pixel down, in texels
            let area = e1.0 * e2.1 - e1.1 * e2.0;
            if area.abs() > 1.0 { return sample_footprint(texels, image.lit_rows(), centre, e1, e2, area, out); }
        }
        sample_bilinear(texels, centre.0, centre.1, out)
    }

    /// The region one cell from `located` along the face axes; across a face edge the direction is probed instead.
    #[inline]
    fn neighbour(&self, located: Located, catalog: [f32; 3], dx: i64, dy: i64, shift: f32) -> usize {
        let last = EDGE as i64 - 1;
        let (nx, ny) = (located.ix as i64 + dx, located.iy as i64 + dy);
        if (0..=last).contains(&nx) && (0..=last).contains(&ny) {
            return cell_index(Located { ix: nx as usize, iy: ny as usize, ..located }, self.spread);
        }
        let (ax, ay) = face_axes(located.face);
        let (sx, sy) = (dx as f32 * shift, dy as f32 * shift);
        let probe = [catalog[0] + ax[0] * sx + ay[0] * sy, catalog[1] + ax[1] * sx + ay[1] * sy, catalog[2] + ax[2] * sx + ay[2] * sy];
        cell_index(locate(probe), self.spread)
    }
}

/// The camera-frame step from this pixel to a neighbour: the next one along `step` when it exists and lies in the
/// view, otherwise the previous one, otherwise nothing.
#[inline]
fn neighbour_delta(rays: &[Ray], d: [f32; 3], index: usize, step: usize, has_next: bool, has_previous: bool) -> Option<[f32; 3]> {
    if has_next {
        let next = rays[index + step].direction;
        if !next[0].is_nan() { return Some([next[0] - d[0], next[1] - d[1], next[2] - d[2]]); }
    }
    if has_previous {
        let previous = rays[index - step].direction;
        if !previous[0].is_nan() { return Some([d[0] - previous[0], d[1] - previous[1], d[2] - previous[2]]); }
    }
    None
}

/// Tile pixel coordinates (column, row) of a catalog direction, or `None` behind the tangent plane.
#[inline]
fn tile_coordinates(tile: &TileFrame, catalog: [f32; 3]) -> Option<(f32, f32)> {
    let depth = dot(catalog, tile.center);
    if depth <= 1e-6 { return None; }
    let scale = tile.density / depth;
    let middle = TileGeometry::middle() as f32;
    Some((middle + dot(catalog, tile.east) * scale, middle - dot(catalog, tile.north) * scale))
}

/// Add up the texels whose centres lie inside the pixel's footprint, the parallelogram spanned by `e1` and `e2`
/// around `centre` (half-open, so the footprints of neighbouring pixels tile the plane and every texel's light
/// reaches the screen exactly once).
#[inline]
fn sample_footprint(texels: &[Texel], lit: &[u64], centre: (f32, f32), e1: (f32, f32), e2: (f32, f32), area: f32, out: &mut Texel) {
    let last = TILE_EDGE_TEXELS as i64 - 1;
    let (reach_x, reach_y) = ((e1.0.abs() + e2.0.abs()) * 0.5, (e1.1.abs() + e2.1.abs()) * 0.5);
    let (x0, x1) = (floor_fast(centre.0 - reach_x).max(0), floor_fast(centre.0 + reach_x + 1.0).min(last));
    let (y0, y1) = (floor_fast(centre.1 - reach_y).max(0), floor_fast(centre.1 + reach_y + 1.0).min(last));
    if x0 > x1 || y0 > y1 { return; }
    let window = u64::MAX >> (63 - (x1 - x0));                                                // the columns x0..=x1 as bits
    let inverse = 1.0 / area;
    let mut sum = *out;
    for y in y0..=y1 {
        let mut bits = (lit[y as usize] >> x0) & window;                                    // only the lit texels of this row are visited
        if bits == 0 { continue; }
        let row = &texels[y as usize * TILE_EDGE_TEXELS..];
        let ty = y as f32 - centre.1;
        while bits != 0 {
            let x = x0 + i64::from(bits.trailing_zeros());
            bits &= bits - 1;
            let tx = x as f32 - centre.0;
            let p = (tx * e2.1 - ty * e2.0) * inverse;                                       // position in pixel units along e1
            let q = (e1.0 * ty - e1.1 * tx) * inverse;                                       // and along e2
            if (-0.5..0.5).contains(&p) && (-0.5..0.5).contains(&q) { sum = sum.plus(row[x as usize]); }
        }
    }
    *out = sum;
}

#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }

#[inline]
fn length(a: [f32; 3]) -> f32 { dot(a, a).sqrt() }

/// Bilinear sample at tile pixel coordinates; texels outside the tile contribute nothing.
#[inline]
fn sample_bilinear(texels: &[Texel], u: f32, v: f32, out: &mut Texel) {
    let size = TILE_EDGE_TEXELS as i64;
    if u < -1.0 || v < -1.0 || u >= size as f32 || v >= size as f32 { return; }
    let (x0, y0) = (floor_fast(u), floor_fast(v));
    let (wx, wy) = (u - x0 as f32, v - y0 as f32);
    let fetch = |x: i64, y: i64| -> Texel {
        if x < 0 || y < 0 || x >= size || y >= size { return Texel::default(); }
        texels[y as usize * TILE_EDGE_TEXELS + x as usize]
    };
    let (p00, p10, p01, p11) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
    let row0 = p00.scaled(1.0 - wx).plus(p10.scaled(wx));
    let row1 = p01.scaled(1.0 - wx).plus(p11.scaled(wx));
    *out = out.plus(row0.scaled(1.0 - wy).plus(row1.scaled(wy)));
}

/// `v.floor() as i64` without the library call: truncate towards zero, then step down for negative fractions.
#[inline]
fn floor_fast(v: f32) -> i64 {
    let truncated = v as i64;
    if (truncated as f32) > v { truncated - 1 } else { truncated }
}

/// `hash_direction` in f32 at `GRID_DEPTH`, keeping the position inside the cell; one division per pixel.
#[inline]
fn locate(p: [f32; 3]) -> Located {
    let (x, y, z) = (p[0].abs(), p[1].abs(), p[2].abs());
    let (face, a, b, major) = if x >= y && x >= z { (usize::from(p[0] < 0.0), p[1], p[2], x) }
        else if y >= z { (2 + usize::from(p[1] < 0.0), p[0], p[2], y) }
        else { (4 + usize::from(p[2] < 0.0), p[0], p[1], z) };
    let inverse = 1.0 / major;
    let (u, v) = (a * inverse, b * inverse);                                               // face coordinates in [-1, 1]
    let n = EDGE as f32;
    let (qx, qy) = ((u + 1.0) * 0.5 * n, (v + 1.0) * 0.5 * n);                            // in [0, n]
    let (ix, iy) = ((qx as usize).min(EDGE - 1), (qy as usize).min(EDGE - 1));            // truncation is floor for non-negative values
    Located { face, ix, iy, fx: qx - ix as f32, fy: qy - iy as f32, stretch: 1.0 + u * u + v * v, major }
}

#[inline]
fn cell_index(located: Located, spread: &[u16; EDGE]) -> usize {
    located.face << (2 * GRID_DEPTH) | usize::from(spread[located.ix]) | usize::from(spread[located.iy]) << 1
}

/// Bit spreading for the cube-map Morton index, precomputed for every cell coordinate of one face edge.
fn spread_table() -> &'static [u16; EDGE] {
    static SPREAD: OnceLock<[u16; EDGE]> = OnceLock::new();
    SPREAD.get_or_init(|| std::array::from_fn(|value| (0..GRID_DEPTH).map(|bit| (((value >> bit) & 1) << (2 * bit)) as u16).sum()))
}

/// The 3D directions along which a face's cell coordinates grow, for probing across cell edges.
fn face_axes(face: usize) -> ([f32; 3], [f32; 3]) {
    match face {
        0 | 1 => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        2 | 3 => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        _ => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    }
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, J2000, Observer, refract_direction};
    use crate::catalog::{SpaceMotion, StarId, load_embedded_catalog};
    use crate::model::{RenderOptions, SkyTables, ViewCenter, hash_direction, region_caps};
    use crate::observation::observe;
    use crate::projection::{project_camera, project_to_cell};
    use crate::regions::{refresh_regions, zoom_opacity_boost};
    use crate::selection::select_regions;

    fn tables_of(catalog: crate::catalog::Catalog) -> SkyTables {
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn bsc5() -> SkyTables { tables_of(load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap()) }

    /// One star at the unit catalog `direction`, as a catalog of its own.
    fn single_star(direction: Vector3, magnitude: f64) -> SkyTables {
        let mut parsed = load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        let mut star = parsed.stars[0].clone();
        parsed.stars.clear();
        parsed.constellations.clear();
        parsed.hr_representatives.clear();
        star.id = StarId(1);
        star.has_data = true;
        star.magnitude = magnitude;
        star.right_ascension = direction.y.atan2(direction.x);
        star.declination = direction.z.asin();
        star.space_motion = Some(SpaceMotion { distance_pc: 10.0, position: direction, velocity: Vector3 { x: 0.0, y: 0.0, z: 0.0 } });
        parsed.stars.push(star);
        tables_of(parsed)
    }

    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    fn view(fov_degrees: f64, azimuth: f64, tilt: f64, projection: ProjectionKind) -> View { View { center: ViewCenter::Facing { azimuth, tilt }, projection, fov_degrees } }
    fn settings() -> RenderOptions { RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 9.0, dynamic_names: true } }

    /// Observe, select, refresh and project one frame.
    fn render(frame: &mut Frame, caches: &mut RegionCaches, tables: &SkyTables, view: &View, days: f64, atmosphere: bool) -> (Observation, bool) {
        let time = FrameTime::from_utc(J2000 + days);
        let observation = observe(&boston(), atmosphere, view, time, &mut crate::timing::StepTimes::default());
        let selected = select_regions(&tables.stars, &observation, &mut crate::timing::StepTimes::default());
        refresh_regions(caches, tables, &selected, time, view, &settings(), &mut crate::timing::StepTimes::default());
        let rebuilt = project_tiles_inverse(frame, caches, &selected, &observation, view, &mut crate::timing::StepTimes::default());
        (observation, rebuilt)
    }

    /// Opacity-weighted centre of the lit pixels, their count and their total opacity.
    fn blob(frame: &Frame) -> Option<((f64, f64), usize, f64)> {
        let (mut row_sum, mut column_sum, mut total, mut count) = (0.0, 0.0, 0.0, 0);
        for (index, texel) in frame.stars().iter().enumerate() {
            if texel.opacity <= 0.0 { continue; }
            let weight = f64::from(texel.opacity);
            row_sum += (index / frame.width()) as f64 * weight;
            column_sum += (index % frame.width()) as f64 * weight;
            total += weight;
            count += 1;
        }
        (count > 0).then(|| ((row_sum / total, column_sum / total), count, total))
    }

    /// Where the old per-star projection puts a catalog direction, as float pixel coordinates (row, column).
    fn old_position(observation: &Observation, view: &View, frame: &Frame, direction: Vector3) -> (f64, f64) {
        let apparent = observation.apparent_star_direction(direction);
        let point = project_camera(prepare_camera(view), apparent).unwrap();
        let sky = frame.sky();
        let radius = (sky.size as f64 - 1.0) / 2.0;
        (sky.y as f64 + radius - point.y * radius, sky.x as f64 + radius + point.x * radius)
    }

    #[test]
    fn rays_invert_the_camera_projection_inside_the_view_and_are_absent_outside() {
        for (projection, fov) in [(ProjectionKind::Stereographic, 180.0), (ProjectionKind::Stereographic, 25.0), (ProjectionKind::Equidistant, 360.0), (ProjectionKind::Equidistant, 90.0)] {
            let view = view(fov, 0.7, 0.4, projection);
            let (width, height) = (101, 81);
            let mut table = ViewTable::default();
            assert!(table.prepare(&view, width, height, SkyArea::centred(width, height)));
            let camera = prepare_camera(&view);
            let sky = SkyArea::centred(width, height);
            let viewport = sky.viewport();
            let (mut inside, mut outside) = (0, 0);
            for row in 0..height {
                for column in 0..width {
                    let ray = table.rays[row * width + column];
                    if ray.direction[0].is_nan() { outside += 1; continue; }
                    inside += 1;
                    let d = ray.direction.map(f64::from);
                    let horizon = camera.right * d[0] + camera.up * d[1] + camera.forward * d[2];
                    assert!((horizon.length() - 1.0).abs() < 1e-5);
                    let point = project_camera(camera, horizon).unwrap();
                    assert_eq!(sky.frame_pixel(project_to_cell(viewport, point)), (row as i32, column as i32), "{projection:?} {fov}° pixel ({row}, {column})");
                }
            }
            assert!(inside > sky.size * sky.size / 2 && outside > 0, "{projection:?}: {inside} inside, {outside} outside");
        }
    }

    #[test]
    fn the_ray_table_is_reused_across_frames_and_rebuilt_on_zoom_resize_or_projection_change() {
        let tables = bsc5();
        let mut caches = RegionCaches::default();
        let mut frame = Frame::square(120);
        let wide = view(90.0, 1.0, 0.5, ProjectionKind::Stereographic);
        assert!(render(&mut frame, &mut caches, &tables, &wide, 0.0, true).1);
        assert!(!render(&mut frame, &mut caches, &tables, &wide, 0.0, true).1);               // the same frame
        assert!(!render(&mut frame, &mut caches, &tables, &wide, 3.0 / 24.0, true).1);        // Earth's rotation
        assert!(!render(&mut frame, &mut caches, &tables, &view(90.0, 2.0, 0.1, ProjectionKind::Stereographic), 0.0, true).1); // panning
        assert!(!render(&mut frame, &mut caches, &tables, &wide, 0.0, false).1);              // the atmosphere switch
        assert!(render(&mut frame, &mut caches, &tables, &view(45.0, 1.0, 0.5, ProjectionKind::Stereographic), 0.0, true).1); // zoom
        assert!(render(&mut frame, &mut caches, &tables, &view(45.0, 1.0, 0.5, ProjectionKind::Equidistant), 0.0, true).1); // projection
        frame.resize(100, 100, SkyArea::centred(100, 100));
        assert!(render(&mut frame, &mut caches, &tables, &view(45.0, 1.0, 0.5, ProjectionKind::Equidistant), 0.0, true).1); // resize
        assert_eq!(frame.rays().key(), Some(ViewKey { projection: ProjectionKind::Equidistant, fov_degrees: 45.0, width: 100, height: 100, sky: SkyArea { x: 0, y: 0, size: 100 } }));
        assert_eq!(frame.stars().len(), 10_000);
    }

    #[test]
    fn refraction_removal_inverts_the_refraction_formula() {
        let table = refraction_table();
        for altitude_degrees in [-0.5, 0.0, 0.3, 1.0, 3.0, 10.0, 30.0, 60.0, 89.0, 90.0] {
            let altitude = f64::to_radians(altitude_degrees);
            let apparent = Vector3 { x: 0.6 * altitude.cos(), y: 0.8 * altitude.cos(), z: altitude.sin() };
            let truth = table.remove([apparent.x as f32, apparent.y as f32, apparent.z as f32]);
            let back = refract_direction(Vector3 { x: f64::from(truth[0]), y: f64::from(truth[1]), z: f64::from(truth[2]) });
            assert!((back - apparent).length() < 2e-6, "{altitude_degrees}°: {back:?} vs {apparent:?}"); // 0.4″, the f32 table's resolution
            assert!(f64::from(truth[2]) <= apparent.z + 1e-7);                                   // refraction lifts, so the truth is lower
        }
    }

    #[test]
    fn a_single_star_lands_where_the_old_projection_puts_it() {
        let width = 301;
        for (fov, azimuth, tilt, days, atmosphere) in [(10.0, 1.0, 0.5, 0.0, true), (60.0, 2.5, 0.2, 100.0, true), (180.0, 4.0, 0.9, 4000.0, false), (120.0, 0.3, 0.1, 0.5, true)] {
            let view = view(fov, azimuth, tilt, ProjectionKind::Stereographic);
            let time = FrameTime::from_utc(J2000 + days);
            let observation = observe(&boston(), atmosphere, &view, time, &mut crate::timing::StepTimes::default());
            let offset = observation.to_catalog.apply(Vector3 { x: 0.0, y: 0.0, z: 1.0 }).cross(observation.patch.center).normalized();
            let direction = (observation.patch.center + offset * (0.25 * fov.to_radians() / 2.0)).normalized(); // a quarter of the way to the edge
            let tables = single_star(direction, 2.0);
            let mut caches = RegionCaches::default();
            let mut frame = Frame::square(width);
            render(&mut frame, &mut caches, &tables, &view, days, atmosphere);
            let ((row, column), count, total) = blob(&frame).unwrap_or_else(|| panic!("{fov}°: no star drawn"));
            let expected = old_position(&observation, &view, &frame, direction);
            let region = hash_direction(GRID_DEPTH, direction);
            let index = (row as usize) * width + column as usize;
            let (here, right) = (frame.rays().rays[index].direction.map(f64::from), frame.rays().rays[index + 1].direction.map(f64::from));
            let pixel_angle = ((here[0] - right[0]).powi(2) + (here[1] - right[1]).powi(2) + (here[2] - right[2]).powi(2)).sqrt();
            let pixels_per_texel = (1.0 / (pixel_angle * f64::from(tile_frames()[region].density))).max(1.0);
            let tolerance = 1.0 + pixels_per_texel;
            let distance = ((row - expected.0).powi(2) + (column - expected.1).powi(2)).sqrt();
            assert!(distance <= tolerance, "{fov}°: blob at ({row:.1}, {column:.1}), old projection at ({:.1}, {:.1}), {distance:.2} px apart, tolerance {tolerance:.2}", expected.0, expected.1);
            assert!(count >= 1 && total > 0.0);
            let glyph_opacity = f64::from(crate::regions::star_opacity(2.0, zoom_opacity_boost(fov)));
            assert!(total >= 0.9 * 4.0 * glyph_opacity && total <= 1.5 * 4.0 * glyph_opacity * pixels_per_texel * pixels_per_texel, "{fov}°: total light {total}, glyph {glyph_opacity}"); // the light of the four texels, spread when a texel covers several pixels
        }
    }

    #[test]
    fn a_glyph_straddling_a_region_border_renders_complete() {
        let view = view(45.0, 1.0, 0.5, ProjectionKind::Stereographic);                 // about one texel per pixel
        let time = FrameTime::from_utc(J2000);
        let observation = observe(&boston(), false, &view, time, &mut crate::timing::StepTimes::default());
        let centre = observation.patch.center;
        let region = hash_direction(GRID_DEPTH, centre);
        let cap = region_caps()[region];
        let tile = TileGeometry::new(region);
        // walk from the region centre until the region changes, then step back a third of a texel
        let step = tile.direction_of(TileGeometry::middle() + 1.0, TileGeometry::middle()) - cap.center;
        let mut border = cap.center;
        while hash_direction(GRID_DEPTH, border + step) == region { border = border + step; }
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);                                          // border + lo × step is inside, + hi × step outside
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if hash_direction(GRID_DEPTH, border + step * mid) == region { lo = mid } else { hi = mid }
        }
        let inside = (border + step * (lo - 0.33)).normalized();                            // a third of a texel inside the border
        assert_eq!(hash_direction(GRID_DEPTH, inside), region);
        assert_ne!(hash_direction(GRID_DEPTH, (border + step * (hi + 0.5)).normalized()), region);
        let mut results = Vec::new();
        for direction in [inside, cap.center] {
            let tables = single_star(direction, 1.0);
            let mut caches = RegionCaches::default();
            let mut frame = Frame::square(401);
            render(&mut frame, &mut caches, &tables, &view, 0.0, false);
            results.push(blob(&frame).unwrap());
        }
        let (at_border, at_centre) = (results[0], results[1]);
        assert!((at_border.2 - at_centre.2).abs() <= 0.15 * at_centre.2, "light {} at the border vs {} at the centre", at_border.2, at_centre.2);
        assert!(at_border.1 >= at_centre.1 * 3 / 4 && at_border.1 <= at_centre.1 * 5 / 4 + 2, "{} lit pixels at the border vs {}", at_border.1, at_centre.1);
    }

    #[test]
    fn pixels_outside_the_view_stay_dark_and_zooming_in_boosts_the_light() {
        let direction = Vector3 { x: 0.2, y: -0.5, z: 0.84 }.normalized();
        let tables = single_star(direction, 3.0);
        let mut totals = Vec::new();
        for fov in [180.0, 90.0] {
            let time = FrameTime::from_utc(J2000);
            let centre_h = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            let observation = observe(&boston(), false, &View::default(), time, &mut crate::timing::StepTimes::default());
            let star_h = observation.to_horizon.apply(direction);
            let azimuth = star_h.x.atan2(star_h.y);
            let tilt = star_h.z.asin();
            let view = View { center: ViewCenter::Facing { azimuth, tilt }, projection: ProjectionKind::Stereographic, fov_degrees: fov }; // look straight at the star
            let _ = centre_h;
            let mut caches = RegionCaches::default();
            let mut frame = Frame::square(201);
            render(&mut frame, &mut caches, &tables, &view, 0.0, false);
            assert!(frame.star(0, 0).is_empty() && frame.star(200, 200).is_empty());        // the corners lie outside the rendered circle
            let ((row, column), _, total) = blob(&frame).unwrap();
            assert!((row - 100.0).abs() < 3.0 && (column - 100.0).abs() < 3.0, "{fov}°: ({row}, {column})");
            totals.push(total);
        }
        let boost = zoom_opacity_boost(90.0);
        assert!(totals[1] > totals[0] * boost * 0.5, "{totals:?}, boost {boost}");           // more light when zoomed in: the boost and the larger blob
    }

    #[test]
    fn the_brightest_region_star_is_the_brightest_pixel_at_the_centre_of_a_small_frame() {
        let tables = bsc5();
        let (region, group, row) = tables.stars.iter().flat_map(|(region, group, table)| (0..table.len()).map(move |row| (region, group, row)))
            .min_by_key(|&(region, group, row)| tables.stars.group(region, group).columns().magnitude[row]).unwrap(); // Polaris and the other endpoints are points, not tiles
        let direction = tables.stars.group(region, group).stored_direction(row);
        let time = FrameTime::from_utc(2_460_678.0);                                        // 2025-01-02T12:00Z
        let site = Observer { latitude: direction.z.asin(), longitude: 0.0 };
        let horizon = observe(&site, true, &View::default(), time, &mut crate::timing::StepTimes::default()).to_horizon.apply(direction);
        let view = View { center: ViewCenter::Facing { azimuth: horizon.x.atan2(horizon.y), tilt: horizon.z.asin() }, projection: ProjectionKind::Stereographic, fov_degrees: 180.0 }; // look straight at it
        let mut caches = RegionCaches::default();
        let mut frame = Frame::new(81, 41, SkyArea::centred(81, 41), (10, 20));
        let observation = observe(&site, true, &view, time, &mut crate::timing::StepTimes::default());
        let selected = select_regions(&tables.stars, &observation, &mut crate::timing::StepTimes::default());
        refresh_regions(&mut caches, &tables, &selected, time, &view, &RenderOptions { magnitude_threshold: 2.1, ..settings() }, &mut crate::timing::StepTimes::default());
        project_tiles_inverse(&mut frame, &caches, &selected, &observation, &view, &mut crate::timing::StepTimes::default());
        let (brightest, light) = frame.stars().iter().enumerate().fold((0, 0.0), |best, (index, texel)| if texel.opacity > best.1 { (index, texel.opacity) } else { best });
        let (row, column) = (brightest / 81, brightest % 81);
        assert!(light > 0.0 && (row as i64 - 20).abs() <= 2 && (column as i64 - 40).abs() <= 3, "brightest pixel at ({row}, {column}) with {light}"); // the sky square is the middle 41 columns
    }

    /// Release-mode cost of the warp; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_warp_at_full_hd() {
        let tables = bsc5();
        for (width, height) in [(1920, 1100), (1280, 720)] {
            for fov in [10.0, 60.0, 180.0] {
                let mut caches = RegionCaches::default();
                let mut frame = Frame::new(width, height, SkyArea::centred(width, height), (10, 20));
                let view = view(fov, 1.0, 0.5, ProjectionKind::Stereographic);
                let time = FrameTime::from_utc(J2000);
                let observation = observe(&boston(), true, &view, time, &mut crate::timing::StepTimes::default());
                let selected = select_regions(&tables.stars, &observation, &mut crate::timing::StepTimes::default());
                refresh_regions(&mut caches, &tables, &selected, time, &view, &settings(), &mut crate::timing::StepTimes::default());
                let started = std::time::Instant::now();
                project_tiles_inverse(&mut frame, &caches, &selected, &observation, &view, &mut crate::timing::StepTimes::default());
                let first = started.elapsed();
                let started = std::time::Instant::now();
                let frames = 5;
                for _ in 0..frames { project_tiles_inverse(&mut frame, &caches, &selected, &observation, &view, &mut crate::timing::StepTimes::default()); }
                let steady = started.elapsed() / frames;
                let lit = frame.stars().iter().filter(|texel| !texel.is_empty()).count();
                eprintln!("{width}×{height} {fov:>5}°  table+warp {first:>10.3?}  warp {steady:>10.3?}  regions={} lit pixels={lit}", selected.regions.len());
            }
        }
    }
}
