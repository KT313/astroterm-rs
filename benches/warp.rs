//! Pipeline restructuring, chunk 2: the cost of warping sky-fixed region tiles onto the screen (plan §7).
//!
//! Standalone: no catalog, no caches. Per view it builds the table of true horizon directions per pixel (inverse
//! projection, refraction removed), fills synthetic tiles at the view's pixel density, and warps them with a real
//! rotation matrix and a constant aberration shift. The warp runs on one thread and reads every tile whose glyph
//! margin can reach a pixel, as the real pipeline will.
//!
//! Tiles are cube-map cells at a depth chosen per view so that a tile is at least `MIN_TILE_EDGE` pixels wide; at
//! wide fields that is coarser than the depth-6 regions of the star table, at narrow fields it is the region itself.
//! Tile axes follow the screen axes at render time so that neighbouring screen pixels read neighbouring tile pixels.
use astroterm::astro::models::orientation::{compute_body_fixed_rotation, compute_horizon_rotation, compute_slow_orientation};
use astroterm::astro::{Horizontal, J2000, Matrix3, Observer, Vector3, apply_refraction};
use astroterm::constants::GRID_DEPTH;
use astroterm::model::{ProjectionKind, View, ViewCenter};
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};
use std::f64::consts::{FRAC_PI_2, PI};
use std::time::Duration;

const GLYPH_MARGIN: usize = 6;                      // largest glyph radius in pixels, added around every tile
const MIN_TILE_EDGE: f64 = 64.0;                    // a tile is at least this many pixels wide, choosing the cube-map depth
const REFRACTION_ITERATIONS: usize = 6;             // fixed-point steps to remove refraction from an apparent altitude
const REFRACTION_TABLE_SIZE: usize = 8192;          // entries of the one-time apparent-to-true altitude table over sin(altitude)
const LIGHT_SPEED_AU_DAY: f64 = 173.144632674240;
const SITE: Observer = Observer { latitude: 0.6229, longitude: 2.4381 }; // Tokyo
const RESOLUTIONS: [(usize, usize); 2] = [(1920, 1100), (1280, 720)];
const MAX_EDGE: usize = 1 << GRID_DEPTH;

/// Tile texel: linear RGB. A padded 16-byte texel measured the same, so the smaller one stays.
type Texel = [f32; 3];

/// True horizon direction per screen pixel; NaN where the projection has no direction.
struct ViewTable {
    directions: Vec<[f32; 3]>,
}

/// One gnomonic tile around a cube-map cell centre: tangent basis, pixels per tangent unit and its pixel block.
struct Tile {
    center: [f32; 3],
    e1: [f32; 3],
    e2: [f32; 3],
    scale: f32,                                     // pixels per tangent-plane unit
    half: f32,                                      // half the tile edge in pixels
    size: usize,
    offset: usize,
}

struct Tiles {
    depth: u8,                                      // cube-map depth of the tiles
    margin_cells: f32,                              // the glyph margin as a fraction of one cell edge
    margin_angle: f32,                              // the glyph margin as an angle, for probing across face edges
    pixels: Vec<Texel>,
    tiles: Vec<Tile>,
    slot_of_cell: Vec<u32>,
}

/// A direction located on the cube map: face, cell coordinates and the position inside the cell.
#[derive(Clone, Copy)]
struct Located {
    face: usize,
    ix: usize,
    iy: usize,
    fx: f32,
    fy: f32,
}

/// The camera basis and scale as `projection::prepare_camera` builds them (its fields are private to the crate).
fn camera_basis(view: &View) -> (Vector3, Vector3, Vector3, f64) {
    let (right, up, forward) = match view.center {
        ViewCenter::Zenith => (Vector3 { x: -1.0, y: 0.0, z: 0.0 }, Vector3 { x: 0.0, y: 1.0, z: 0.0 }, Vector3 { x: 0.0, y: 0.0, z: 1.0 }),
        ViewCenter::Facing { azimuth, tilt } => {
            let (sa, ca) = azimuth.sin_cos();
            let (st, ct) = tilt.sin_cos();
            (Vector3 { x: ca, y: -sa, z: 0.0 }, Vector3 { x: -st * sa, y: -st * ca, z: ct }, Vector3 { x: ct * sa, y: ct * ca, z: st })
        }
    };
    let scale = match view.projection {
        ProjectionKind::Stereographic if view.fov_degrees == 180.0 => 1.0,
        ProjectionKind::Stereographic => 1.0 / (view.fov_degrees.to_radians() / 4.0).tan(),
        ProjectionKind::Equidistant => 180.0 / view.fov_degrees,
    };
    (right, up, forward, scale)
}

/// Screen pixels per radian at the view centre, which sets the tile pixel density.
fn pixels_per_radian(view: &View, height: usize) -> f64 {
    let (_, _, _, scale) = camera_basis(view);
    let radius = (height as f64 - 1.0) / 2.0;
    match view.projection {
        ProjectionKind::Stereographic => scale * radius / 2.0,
        ProjectionKind::Equidistant => scale * radius / FRAC_PI_2,
    }
}

/// The deepest cube-map level whose cells are still at least `MIN_TILE_EDGE` pixels wide, at most the region depth.
fn tile_depth(view: &View, height: usize) -> u8 {
    let face_pixels = FRAC_PI_2 * pixels_per_radian(view, height);
    ((face_pixels / MIN_TILE_EDGE).log2().floor().max(0.0) as u8).min(GRID_DEPTH)
}

/// The altitude whose refracted value is `apparent`.
fn remove_refraction(apparent: f64) -> f64 {
    let mut altitude = apparent;
    for _ in 0..REFRACTION_ITERATIONS {
        let lift = apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - altitude;
        altitude = apparent - lift;
    }
    altitude
}

/// Refraction removal as a table over z = sin(apparent altitude): per entry the true sin(altitude) and the factor
/// that rescales the horizontal components. Built once per process; refraction does not depend on the view.
struct RefractionTable {
    entries: Vec<(f64, f64)>,
}

fn build_refraction_table() -> RefractionTable {
    let entries = (0..=REFRACTION_TABLE_SIZE).map(|i| {
        let z = -1.0 + 2.0 * i as f64 / REFRACTION_TABLE_SIZE as f64;
        let apparent = z.clamp(-1.0, 1.0).asin();
        let truth = remove_refraction(apparent);
        let horizontal_scale = if apparent.cos() > 1e-9 { truth.cos() / apparent.cos() } else { 1.0 };
        (truth.sin(), horizontal_scale)
    }).collect();
    RefractionTable { entries }
}

impl RefractionTable {
    /// The true direction for an apparent unit direction, keeping its azimuth; linear interpolation in z.
    #[inline]
    fn remove(&self, apparent: Vector3) -> Vector3 {
        let position = (apparent.z.clamp(-1.0, 1.0) + 1.0) * 0.5 * REFRACTION_TABLE_SIZE as f64;
        let index = (position as usize).min(REFRACTION_TABLE_SIZE - 1);
        let weight = position - index as f64;
        let (a, b) = (self.entries[index], self.entries[index + 1]);
        let (sin_true, horizontal_scale) = (a.0 + (b.0 - a.0) * weight, a.1 + (b.1 - a.1) * weight);
        Vector3 { x: apparent.x * horizontal_scale, y: apparent.y * horizontal_scale, z: sin_true }
    }
}

/// Build the per-view table: inverse projection per pixel, then refraction removed from the apparent altitude.
fn build_view_table(view: &View, width: usize, height: usize, refraction: &RefractionTable) -> ViewTable {
    let (right, up, forward, scale) = camera_basis(view);
    let (ry, rx) = ((height as f64 - 1.0) / 2.0, (width as f64 - 1.0) / 2.0);
    let mut directions = Vec::with_capacity(width * height);
    for row in 0..height {
        for column in 0..width {
            // pixel to projection-plane coordinates, the inverse of `project_to_cell`
            let px = (column as f64 - rx) / rx;
            let py = -(row as f64 - ry) / ry;

            // inverse projection to an apparent unit direction in the camera basis
            let apparent = match view.projection {
                ProjectionKind::Stereographic => {
                    let (qx, qy) = (px / scale, py / scale);
                    let c = (1.0 - qx * qx - qy * qy) / (1.0 + qx * qx + qy * qy);
                    right * (qx * (1.0 + c)) + up * (qy * (1.0 + c)) + forward * c
                }
                ProjectionKind::Equidistant => {
                    let radius = px.hypot(py);
                    let theta = radius * FRAC_PI_2 / scale;
                    if theta > PI { directions.push([f32::NAN; 3]); continue; }
                    let (s, c) = theta.sin_cos();
                    let (ux, uy) = if radius > 0.0 { (px / radius, py / radius) } else { (0.0, 0.0) };
                    right * (ux * s) + up * (uy * s) + forward * c
                }
            };

            // refraction removed, keeping the azimuth
            let truth = refraction.remove(apparent);
            directions.push([truth.x as f32, truth.y as f32, truth.z as f32]);
        }
    }
    ViewTable { directions }
}

/// Direction of a point on a cube face, as `model::catalog::grid::direction` defines it.
fn face_direction(face: usize, u: f64, v: f64) -> Vector3 {
    match face {
        0 => Vector3 { x: 1.0, y: u, z: v },
        1 => Vector3 { x: -1.0, y: u, z: v },
        2 => Vector3 { x: u, y: 1.0, z: v },
        3 => Vector3 { x: u, y: -1.0, z: v },
        4 => Vector3 { x: u, y: v, z: 1.0 },
        _ => Vector3 { x: u, y: v, z: -1.0 },
    }.normalized()
}

/// The 3D directions along which a face's cell coordinates grow, for probing across cell edges.
fn face_axes(face: usize) -> ([f32; 3], [f32; 3]) {
    match face {
        0 | 1 => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        2 | 3 => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        _ => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    }
}

/// Centre direction and angular radius of a cell at the given depth, decoded as `build_caps` encodes it.
fn cell_geometry(cell: usize, depth: u8) -> (Vector3, f64) {
    let n = 1_usize << depth;
    let (face, morton) = (cell / (n * n), cell % (n * n));
    let (mut x, mut y) = (0, 0);
    for bit in 0..depth {
        x |= ((morton >> (2 * bit)) & 1) << bit;
        y |= ((morton >> (2 * bit + 1)) & 1) << bit;
    }
    let step = 2.0 / n as f64;
    let (u, v) = (-1.0 + step * x as f64, -1.0 + step * y as f64);
    let center = face_direction(face, u + step / 2.0, v + step / 2.0);
    let radius = [(u, v), (u + step, v), (u, v + step), (u + step, v + step)]
        .into_iter()
        .map(|(u, v)| { let corner = face_direction(face, u, v); center.cross(corner).length().atan2(center.dot(corner)) })
        .fold(0.0, f64::max);
    (center, radius)
}

/// Cells at the tile depth whose cap overlaps the view cone, with the selection margin of the current pipeline.
fn select_cells(view: &View, rotation: Matrix3, depth: u8) -> Vec<usize> {
    let (_, _, forward, _) = camera_basis(view);
    let forward_catalog = rotation.transpose().apply(forward);
    let cone = (view.fov_degrees.to_radians() / 2.0 + astroterm::constants::STELLAR_DRIFT_MARGIN).min(PI);
    (0..6 << (2 * depth))
        .filter(|&cell| {
            let (center, radius) = cell_geometry(cell, depth);
            let sum = cone + radius;
            sum >= PI || center.dot(forward_catalog) >= sum.cos()
        })
        .collect()
}

fn to_f32(v: Vector3) -> [f32; 3] { [v.x as f32, v.y as f32, v.z as f32] }

/// Synthetic tiles for the selected cells at the view's pixel density, axes aligned with the screen at render time;
/// every tile carries one bright pixel at its centre.
fn build_tiles(view: &View, height: usize, rotation: Matrix3, depth: u8, cells: &[usize]) -> Tiles {
    let density = pixels_per_radian(view, height);
    let (right, up, _, _) = camera_basis(view);
    let (right, up) = (rotation.transpose().apply(right), rotation.transpose().apply(up)); // screen axes in the catalog frame
    let cell_angle = 2.0 * (1.0 / (1_usize << depth) as f64).atan();                       // a cell's angular width at the face centre
    let mut tiles = Vec::with_capacity(cells.len());
    let mut pixels = Vec::new();
    let mut slot_of_cell = vec![u32::MAX; 6 << (2 * depth)];
    for (slot, &cell) in cells.iter().enumerate() {
        let (center, radius) = cell_geometry(cell, depth);
        let hint = if up.dot(center).abs() < 0.9 { up } else { right };
        let e1 = hint.cross(center).normalized();                                           // screen right, projected onto the tile plane
        let e2 = e1.cross(center);                                                          // screen up
        let half = (radius.tan() * density).ceil() + GLYPH_MARGIN as f64;
        let size = 2 * half as usize;
        let offset = pixels.len();
        pixels.resize(offset + size * size, [0.0; 3]);
        pixels[offset + (size / 2) * size + size / 2] = [1.0, 0.8, 0.6];                 // one star at the cell centre
        tiles.push(Tile { center: to_f32(center), e1: to_f32(e1), e2: to_f32(e2), scale: density as f32, half: half as f32, size, offset });
        slot_of_cell[cell] = slot as u32;
    }
    Tiles {
        depth,
        margin_cells: (GLYPH_MARGIN as f64 / (cell_angle * density)) as f32,
        margin_angle: (GLYPH_MARGIN as f64 / density) as f32,
        pixels, tiles, slot_of_cell,
    }
}

/// Bit spreading for the cube-map Morton index, precomputed for every cell coordinate of one face edge.
fn spread_table() -> [u16; MAX_EDGE] {
    std::array::from_fn(|value| (0..GRID_DEPTH).map(|bit| (((value >> bit) & 1) << (2 * bit)) as u16).sum())
}

/// `hash_direction` in f32 for any depth, keeping the position inside the cell; one division per pixel.
#[inline(always)]
fn locate(p: [f32; 3], depth: u8) -> Located {
    let (x, y, z) = (p[0].abs(), p[1].abs(), p[2].abs());
    let (face, a, b, major) = if x >= y && x >= z { (usize::from(p[0] < 0.0), p[1], p[2], x) }
        else if y >= z { (2 + usize::from(p[1] < 0.0), p[0], p[2], y) }
        else { (4 + usize::from(p[2] < 0.0), p[0], p[1], z) };
    let n = (1_usize << depth) as f32;
    let scale = 0.5 * n / major;
    let (qx, qy) = ((a + major) * scale, (b + major) * scale);               // in [0, n]
    let (ix, iy) = (qx as usize, qy as usize);                                // truncation is floor for non-negative values; no libm call
    let last = (1_usize << depth) - 1;
    Located { face, ix: ix.min(last), iy: iy.min(last), fx: qx - ix as f32, fy: qy - iy as f32 }
}

#[inline(always)]
fn cell_index(located: Located, depth: u8, spread: &[u16; MAX_EDGE]) -> usize {
    located.face << (2 * depth) | usize::from(spread[located.ix]) | usize::from(spread[located.iy]) << 1
}

#[inline(always)]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }

/// Gnomonic tile coordinates of a catalog direction (any length), relative to the tile centre, in pixels.
#[inline(always)]
fn tile_coordinates(tile: &Tile, direction: [f32; 3]) -> (f32, f32) {
    let depth = tile.scale / dot(direction, tile.center);
    (dot(direction, tile.e1) * depth, dot(direction, tile.e2) * depth)
}

/// Bilinear sample of a tile at pixel coordinates relative to its centre; outside the tile contributes nothing.
#[inline(always)]
fn sample(tiles: &Tiles, tile: &Tile, u: f32, v: f32, out: &mut [f32; 3]) {
    let (fx, fy) = (u + tile.half - 0.5, v + tile.half - 0.5);
    let (x0, y0) = ((fx + 2.0) as i64 - 2, (fy + 2.0) as i64 - 2);           // floor without a libm call; exact for fx >= -2, and anything below -1 is rejected next
    let size = tile.size as i64;
    if x0 < -1 || y0 < -1 || x0 >= size || y0 >= size { return; }
    let (wx, wy) = (fx - x0 as f32, fy - y0 as f32);
    let (p00, p10, p01, p11) = if x0 >= 0 && y0 >= 0 && x0 + 1 < size && y0 + 1 < size {
        let row0 = tile.offset + y0 as usize * tile.size + x0 as usize;              // the common case: all four texels inside
        let row1 = row0 + tile.size;
        (tiles.pixels[row0], tiles.pixels[row0 + 1], tiles.pixels[row1], tiles.pixels[row1 + 1])
    } else {
        let fetch = |x: i64, y: i64| -> Texel {
            if x < 0 || y < 0 || x >= size || y >= size { return [0.0; 3]; }
            tiles.pixels[tile.offset + y as usize * tile.size + x as usize]
        };
        (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1))
    };
    for channel in 0..3 {
        out[channel] += (p00[channel] * (1.0 - wx) + p10[channel] * wx) * (1.0 - wy)
            + (p01[channel] * (1.0 - wx) + p11[channel] * wx) * wy;
    }
}

/// Sample the tile of a cell, if one is loaded.
#[inline(always)]
fn sample_cell(tiles: &Tiles, cell: usize, catalog: [f32; 3], out: &mut [f32; 3]) {
    let slot = tiles.slot_of_cell[cell];
    if slot == u32::MAX { return; }
    let tile = &tiles.tiles[slot as usize];
    let (u, v) = tile_coordinates(tile, catalog);
    sample(tiles, tile, u, v, out);
}

/// The cell one step from `located` along the face axes; across a face edge the direction is probed instead.
#[inline(always)]
fn neighbour_cell(tiles: &Tiles, located: Located, catalog: [f32; 3], dx: i64, dy: i64, spread: &[u16; MAX_EDGE]) -> usize {
    let last = (1_i64 << tiles.depth) - 1;
    let (nx, ny) = (located.ix as i64 + dx, located.iy as i64 + dy);
    if (0..=last).contains(&nx) && (0..=last).contains(&ny) {
        return cell_index(Located { ix: nx as usize, iy: ny as usize, ..located }, tiles.depth, spread);
    }
    let (ax, ay) = face_axes(located.face);
    let (sx, sy) = (dx as f32 * tiles.margin_angle, dy as f32 * tiles.margin_angle);
    let probe = [catalog[0] + ax[0] * sx + ay[0] * sy, catalog[1] + ax[1] * sx + ay[1] * sy, catalog[2] + ax[2] * sx + ay[2] * sy];
    cell_index(locate(probe, tiles.depth), tiles.depth, spread)
}

/// What the warp does per pixel; the reduced modes exist only to attribute the cost.
#[derive(Clone, Copy, PartialEq)]
enum Mode { Full, PrimaryOnly, MathOnly }

/// Per frame: every screen pixel is mapped back to its catalog direction and samples the tiles covering it.
fn warp(table: &ViewTable, tiles: &Tiles, rotation_inverse: [[f32; 3]; 3], beta: [f32; 3], mode: Mode, frame: &mut [[f32; 3]]) {
    let spread = spread_table();
    let (depth, margin) = (tiles.depth, tiles.margin_cells);
    for (direction, out) in table.directions.iter().zip(frame.iter_mut()) {
        *out = [0.0; 3];
        if direction[0].is_nan() { continue; }

        // true horizon direction → catalog direction: inverse rotation, then the aberration shift removed.
        // Nothing below needs a unit vector, so there is no renormalisation.
        let rotated = [dot(rotation_inverse[0], *direction), dot(rotation_inverse[1], *direction), dot(rotation_inverse[2], *direction)];
        let catalog = [rotated[0] - beta[0], rotated[1] - beta[1], rotated[2] - beta[2]];

        // the pixel's own cell
        let located = locate(catalog, depth);
        let cell = cell_index(located, depth, &spread);
        if mode == Mode::MathOnly { out[0] = located.fx; out[1] = located.fy; out[2] = cell as f32; continue; }
        sample_cell(tiles, cell, catalog, out);
        if mode == Mode::PrimaryOnly { continue; }

        // within a glyph margin of a cell edge, the neighbours' margins also cover this pixel
        let dx = if located.fx < margin { -1 } else if located.fx > 1.0 - margin { 1 } else { 0 };
        let dy = if located.fy < margin { -1 } else if located.fy > 1.0 - margin { 1 } else { 0 };
        if dx != 0 { sample_cell(tiles, neighbour_cell(tiles, located, catalog, dx, 0, &spread), catalog, out); }
        if dy != 0 { sample_cell(tiles, neighbour_cell(tiles, located, catalog, 0, dy, &spread), catalog, out); }
        if dx != 0 && dy != 0 { sample_cell(tiles, neighbour_cell(tiles, located, catalog, dx, dy, &spread), catalog, out); }
    }
}

/// The same warp over horizontal bands of rows on `threads` threads; the per-pixel work is unchanged.
fn warp_parallel(pool: &rayon::ThreadPool, table: &ViewTable, tiles: &Tiles, rotation_inverse: [[f32; 3]; 3], beta: [f32; 3], width: usize, frame: &mut [[f32; 3]]) {
    use rayon::prelude::*;
    let band = 16 * width;
    pool.install(|| {
        frame.par_chunks_mut(band).zip(table.directions.par_chunks(band)).for_each(|(rows, directions)| {
            let table = ViewTable { directions: directions.to_vec() };                  // a borrowed band would avoid this copy in the real pipeline
            warp(&table, tiles, rotation_inverse, beta, Mode::Full, rows);
        });
    });
}

/// Catalog-to-horizon rotation for the site at one instant, from the real orientation model.
fn horizon_rotation() -> Matrix3 {
    let tt = J2000 + 9000.0;
    compute_horizon_rotation(&SITE).compose(compute_body_fixed_rotation(compute_slow_orientation(tt), tt))
}

fn rotation_inverse_f32(rotation: Matrix3) -> [[f32; 3]; 3] {
    rotation.transpose().0.map(|row| row.map(|value| value as f32))
}

/// Earth's orbital velocity over c, as an aberration shift in the horizon frame.
fn aberration_beta() -> [f32; 3] {
    to_f32(Vector3 { x: 0.0172, y: 0.0021, z: -0.0003 } * (1.0 / LIGHT_SPEED_AU_DAY))
}

fn view_for(fov_degrees: f64, projection: ProjectionKind) -> View {
    View { center: ViewCenter::Facing { azimuth: 225_f64.to_radians(), tilt: 30_f64.to_radians() }, projection, fov_degrees }
}

/// A star painted where the warp chain maps one screen pixel must come back on that pixel; checked once per view
/// for a pixel deep inside a cell and for one just inside a cell edge, whose light arrives through a neighbour.
fn check_stars_land_on_their_pixels(view: &View, width: usize, height: usize, rotation: Matrix3, refraction: &RefractionTable) {
    let table = build_view_table(view, width, height, refraction);
    let depth = tile_depth(view, height);
    let mut tiles = build_tiles(view, height, rotation, depth, &select_cells(view, rotation, depth));
    let (inverse, beta) = (rotation_inverse_f32(rotation), aberration_beta());
    let spread = spread_table();
    for pixel in &mut tiles.pixels { *pixel = [0.0; 3]; }
    let locate_pixel = |index: usize| {
        let direction = table.directions[index];
        let rotated = [dot(inverse[0], direction), dot(inverse[1], direction), dot(inverse[2], direction)];
        let catalog = [rotated[0] - beta[0], rotated[1] - beta[1], rotated[2] - beta[2]];
        (catalog, locate(catalog, depth))
    };
    let interior = (height / 3) * width + width / 3;                                    // an arbitrary screen pixel
    let edge = (interior..interior + width / 2).find(|&index| { let (_, l) = locate_pixel(index); l.fx > 1.0 - tiles.margin_cells * 0.5 }).expect("a pixel near a cell edge");
    for (target, paint_in_neighbour) in [(interior, false), (edge, true)] {
        let (catalog, located) = locate_pixel(target);
        let cell = if paint_in_neighbour { neighbour_cell(&tiles, located, catalog, 1, 0, &spread) } else { cell_index(located, depth, &spread) };
        let tile = &tiles.tiles[tiles.slot_of_cell[cell] as usize];
        let (u, v) = tile_coordinates(tile, catalog);
        let (x, y) = ((u + tile.half - 0.5).round() as usize, (v + tile.half - 0.5).round() as usize);
        tiles.pixels[tile.offset + y * tile.size + x] = [1.0; 3];
        let mut frame = vec![[0.0_f32; 3]; width * height];
        warp(&table, &tiles, inverse, beta, Mode::Full, &mut frame);
        let brightest = (0..frame.len()).max_by(|&a, &b| frame[a][0].total_cmp(&frame[b][0])).unwrap();
        let (dy, dx) = ((brightest / width) as i64 - (target / width) as i64, (brightest % width) as i64 - (target % width) as i64);
        assert!(dx.abs() <= 1 && dy.abs() <= 1, "fov {}: star drawn {dy} rows, {dx} columns from its pixel (neighbour {paint_in_neighbour})", view.fov_degrees);
        assert!(frame[brightest][0] > 0.2, "fov {}: the star's light did not reach the frame (neighbour {paint_in_neighbour})", view.fov_degrees);
        tiles.pixels[tile.offset + y * tile.size + x] = [0.0; 3];
    }
}

fn benchmark_warp(criterion: &mut Criterion) {
    let rotation = horizon_rotation();
    let (inverse, beta) = (rotation_inverse_f32(rotation), aberration_beta());
    let start = std::time::Instant::now();
    let refraction = build_refraction_table();
    eprintln!("refraction table: {} entries in {:.2} ms (once per process)", refraction.entries.len(), start.elapsed().as_secs_f64() * 1000.0);
    let direct = Horizontal { azimuth: 0.0, altitude: remove_refraction(0.05) }.to_unit_vector();
    let tabulated = refraction.remove(Horizontal { azimuth: 0.0, altitude: 0.05 }.to_unit_vector());
    eprintln!("refraction table vs direct solve at altitude 0.05 rad: {:.3} arcsec", direct.cross(tabulated).length().atan2(direct.dot(tabulated)).to_degrees() * 3600.0);
    let views = [
        ("stereo10", view_for(10.0, ProjectionKind::Stereographic)),
        ("stereo60", view_for(60.0, ProjectionKind::Stereographic)),
        ("stereo180", view_for(180.0, ProjectionKind::Stereographic)),
        ("equi360", view_for(360.0, ProjectionKind::Equidistant)),
    ];
    let mut group = criterion.benchmark_group("warp");
    group.sample_size(10).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3)).sampling_mode(SamplingMode::Flat);
    for (width, height) in RESOLUTIONS {
        for (name, view) in &views {
            check_stars_land_on_their_pixels(view, width, height, rotation, &refraction);
            let depth = tile_depth(view, height);
            let cells = select_cells(view, rotation, depth);
            let tiles = build_tiles(view, height, rotation, depth, &cells);
            let table = build_view_table(view, width, height, &refraction);
            let mut frame = vec![[0.0_f32; 3]; width * height];
            eprintln!("{width}x{height} {name}: depth {depth}, {} tiles, {} px each, {:.1} MiB of tiles, {:.0} px/rad, margin {:.2} cells",
                tiles.tiles.len(), tiles.tiles.first().map_or(0, |tile| tile.size), tiles.pixels.len() as f64 * 12.0 / 1048576.0, pixels_per_radian(view, height), tiles.margin_cells);
            group.bench_function(format!("table/{width}x{height}/{name}"), |b| b.iter(|| build_view_table(view, width, height, &refraction)));
            group.bench_function(format!("warp/{width}x{height}/{name}"), |b| b.iter(|| warp(&table, &tiles, inverse, beta, Mode::Full, &mut frame)));
            if width == 1920 {
                group.bench_function(format!("primary-only/{width}x{height}/{name}"), |b| b.iter(|| warp(&table, &tiles, inverse, beta, Mode::PrimaryOnly, &mut frame)));
                group.bench_function(format!("math-only/{width}x{height}/{name}"), |b| b.iter(|| warp(&table, &tiles, inverse, beta, Mode::MathOnly, &mut frame)));
                for threads in [2, 4] {
                    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                    group.bench_function(format!("threads{threads}/{width}x{height}/{name}"), |b| b.iter(|| warp_parallel(&pool, &table, &tiles, inverse, beta, width, &mut frame)));
                }
            }
        }
    }
    group.finish();
}

criterion_group!(benches, benchmark_warp);
criterion_main!(benches);
