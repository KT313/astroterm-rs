//! Four-corner forward mapping of each merged region tile. Only the outer corners go through the astronomy
//! and camera formulas. Two textured triangles interpolate the interior, so curved edges are approximated.
//! The quad is painted row by row straight onto the star layer (`raster`), each pixel sampled once from the
//! tile (`tile_sampling`): the texels under the pixel's footprint averaged by area, nothing where they are all
//! empty. Region order and premultiplied source-over composition are unchanged.
use crate::astro::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Data, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, prepare_camera, project_camera};
use crate::regions::{TileGeometry, blend_star};
use crate::selection::Selected;
use crate::time::StepTimes;
use super::raster::{Point, scan_quad};
use super::tile_sampling::TileSampler;

pub(super) fn project_whole_tiles(size: usize, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes) -> Vec<Texel> {
    // Allocate a square image and prepare the current camera.
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size.checked_mul(size).expect("drawing area pixel count overflow")];
    if size < 2 { return stars; }
    times.measure("Projection.Camera preparation");
    let camera = prepare_camera(view);

    // One sampler for every region; no projected result is cached between frames.
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);

    // Project each tile's corners once, then paint its quad onto the frame.
    for &region in &selected.regions {
        let image = &data.star_regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(region);
        times.measure("Projection.Tile corner projection");
        let corners = project_tile_corners(&geometry, observation, camera, size);
        let Some(corners) = corners else { continue; };
        let Some(mappings) = texture_mappings(corners) else { continue; };
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows());
        times.measure("Projection.Tile rasterization");
        draw_tile(&mut stars, size, corners, &mappings, &sampler, image.texels());
    }
    times.measure("Projection");
    stars
}

/// Project the four outer texture boundaries. Skip tiles crossing the back singularity or folding over
/// themselves: a two-triangle surface cannot represent those mappings reliably.
fn project_tile_corners(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, size: usize) -> Option<[Point; 4]> {
    // Texture boundaries extend half a texel beyond the first and last texel centers.
    let minimum = -0.5;
    let maximum = TILE_EDGE_TEXELS as f64 - 0.5;
    let tile_corners = [(minimum, minimum), (maximum, minimum), (maximum, maximum), (minimum, maximum)];
    let directions = tile_corners.map(|(x, y)| {
        observation.apparent_star_direction(tile.direction_of(x, y))
    });
    if contains_antipode(directions, camera.forward) { return None; }

    // Run the real camera projection only at these four corners.
    let radius = (size as f64 - 1.0) * 0.5;
    let mut corners = [[0.0; 2]; 4];
    for (corner, direction) in corners.iter_mut().zip(directions) {
        let projected = project_camera(camera, direction)?;
        if !projected.x.is_finite() || !projected.y.is_finite() { return None; }
        *corner = [radius + projected.x * radius, radius - projected.y * radius];
    }

    // Accept either winding, but reject collapsed or nonconvex quadrilaterals in this prototype.
    if !is_convex(corners) { return None; }
    Some(corners)
}

/// Test the small spherical quad against the direction directly behind the camera. Boundary points count as
/// inside: the projection is singular there too. Texels away from the back hemisphere take the quick exit.
pub(super) fn contains_antipode(corners: [Vector3; 4], forward: Vector3) -> bool {
    if corners.iter().any(|corner| corner.dot(forward) >= 0.0) { return false; }
    let centre = corners.into_iter().fold(Vector3::default(), |sum, corner| sum + corner);
    (0..4).all(|i| {
        let normal = corners[i].cross(corners[(i + 1) % 4]);
        normal.dot(forward * -1.0) * normal.dot(centre) >= 0.0
    })
}

pub(super) fn is_convex(corners: [Point; 4]) -> bool {
    // Each consecutive pair of edges must turn in the same direction.
    let turns: [f64; 4] = std::array::from_fn(|index| {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let c = corners[(index + 2) % 4];
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    });
    turns.iter().all(|turn| turn.is_finite())
        && (turns.iter().all(|&turn| turn > 0.0) || turns.iter().all(|&turn| turn < 0.0))
}

/// The affine mappings of the quad's two triangles (corners 0, 1, 2 and 0, 2, 3) onto the full texture.
fn texture_mappings(corners: [Point; 4]) -> Option<[TextureMapping; 2]> {
    let edge = TILE_EDGE_TEXELS as f64;
    let texture = [[0.0, 0.0], [edge, 0.0], [edge, edge], [0.0, edge]];
    let mapping = |indices: [usize; 3]| TextureMapping::new(indices.map(|index| corners[index]), indices.map(|index| texture[index]));
    Some([mapping([0, 1, 2])?, mapping([0, 2, 3])?])
}

/// Paint the quad row by row: every pixel samples the tile once, through the mapping of the triangle it lies
/// in, and blends source-over onto the star layer with its coverage. Whole pixels step through the tile at a
/// constant pace along the row; outline pixels are resolved only when a lit texel lies under them at all.
#[allow(clippy::too_many_arguments)]
fn draw_tile(stars: &mut [Texel], size: usize, corners: [Point; 4], mappings: &[TextureMapping; 2], sampler: &TileSampler, texels: &[Texel]) {
    scan_quad(corners, size, |span| {
        let row = &mut stars[span.row * size..][..size];
        for (columns, triangle) in span.runs() {
            if columns.is_empty() { continue; }
            let mapping = &mappings[triangle];
            let [mut u, mut v] = mapping.texture_position([columns.start as f64, span.row as f64]);
            for column in columns {
                if let Some(rgba) = sampler.sample(texels, [u, v], mapping.footprint) {          // the whole pixel; outline pixels use it as a quick rejection
                    let (rgba, coverage) = if span.is_whole(column) { (rgba, 1.0) } else {
                        match span.edge_pixel(column) {
                            Some(edge) => match sampler.sample(texels, mapping.texture_position(edge.centre), mapping.footprint_of(edge.size)) {
                                Some(rgba) => (rgba, edge.coverage),
                                None => (rgba, 0.0),
                            },
                            None => (rgba, 0.0),
                        }
                    };
                    if coverage > 0.0 {
                        let texel = &mut row[column];
                        blend_star(texel, [rgba[0] * coverage, rgba[1] * coverage, rgba[2] * coverage], rgba[3] * coverage);
                        texel.rgb = texel.rgb.map(|value| value.min(1.0));                        // absorb last-bit rounding of the blend at opaque pixels
                        texel.opacity = texel.opacity.min(1.0);
                    }
                }
                u += mapping.per_screen_x[0];
                v += mapping.per_screen_x[1];
            }
        }
    });
}

/// Affine screen-to-texture mapping for one triangle. Derivatives are calculated once per triangle, not pixel.
pub(super) struct TextureMapping {
    screen_origin: Point,
    texture_origin: Point,
    per_screen_x: Point,
    per_screen_y: Point,
    footprint: Point,
}

impl TextureMapping {
    pub(super) fn new(screen: [Point; 3], texture: [Point; 3]) -> Option<Self> {
        // Invert the triangle's two screen-space edges; a collapsed triangle has no usable mapping.
        let screen_first = [screen[1][0] - screen[0][0], screen[1][1] - screen[0][1]];
        let screen_second = [screen[2][0] - screen[0][0], screen[2][1] - screen[0][1]];
        let determinant = screen_first[0] * screen_second[1] - screen_second[0] * screen_first[1];
        if determinant == 0.0 || !determinant.is_finite() { return None; }

        // Calculate how one screen-pixel step changes the two texture coordinates.
        let mut per_screen_x = [0.0; 2];
        let mut per_screen_y = [0.0; 2];
        for axis in 0..2 {
            let texture_first = texture[1][axis] - texture[0][axis];
            let texture_second = texture[2][axis] - texture[0][axis];
            per_screen_x[axis] = (texture_first * screen_second[1] - texture_second * screen_first[1]) / determinant;
            per_screen_y[axis] = (texture_second * screen_first[0] - texture_first * screen_second[0]) / determinant;
        }
        let footprint = std::array::from_fn(|axis| per_screen_x[axis].abs() + per_screen_y[axis].abs());
        if footprint.iter().any(|value: &f64| !value.is_finite()) { return None; }

        Some(Self { screen_origin: screen[0], texture_origin: texture[0], per_screen_x, per_screen_y, footprint })
    }

    /// The texture box of a screen rectangle of `size` pixels (the footprint is that of a whole pixel).
    pub(super) fn footprint_of(&self, size: Point) -> Point {
        std::array::from_fn(|axis| self.per_screen_x[axis].abs() * size[0] + self.per_screen_y[axis].abs() * size[1])
    }

    pub(super) fn texture_position(&self, pixel: Point) -> Point {
        // Interpolate inside this triangle instead of repeating sky and camera calculations per texel.
        let delta_x = pixel[0] - self.screen_origin[0];
        let delta_y = pixel[1] - self.screen_origin[1];
        std::array::from_fn(|axis| {
            self.texture_origin[axis] + delta_x * self.per_screen_x[axis] + delta_y * self.per_screen_y[axis]
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::projection::Frame;
    use super::*;
    use crate::astro::{J2000, Observer};
    use crate::model::{ProjectionKind, RenderOptions, ViewCenter, fixtures, hash_direction};
    use crate::observation::observe;
    use crate::regions::refresh_regions;

    #[test]
    fn antipodal_tiles_are_rejected_without_rejecting_nearby_back_facing_tiles() {
        let forward = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let directions = |offset| [(-0.01 + offset, -0.01), (0.01 + offset, -0.01), (0.01 + offset, 0.01), (-0.01 + offset, 0.01)]
            .map(|(x, y)| Vector3 { x, y, z: -1.0 }.normalized());
        assert!(contains_antipode(directions(0.0), forward));
        assert!(!contains_antipode(directions(0.1), forward));
    }

    #[test]
    fn mapping_handles_skew_and_reversed_winding() {
        let screen = [[10.0, 20.0], [18.0, 22.0], [12.0, 26.0]];
        let texture = [[0.0, 0.0], [64.0, 0.0], [0.0, 64.0]];
        for indices in [[0, 1, 2], [0, 2, 1]] {
            let mapping = TextureMapping::new(indices.map(|i| screen[i]), indices.map(|i| texture[i])).unwrap();
            for (point, expected) in screen.into_iter().zip(texture) {
                for (actual, expected) in mapping.texture_position(point).into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
        assert!(TextureMapping::new([[0.0; 2]; 3], texture).is_none());
        assert!(!is_convex([[0.0; 2]; 4]));
        assert!(!is_convex([[0.0, 0.0], [1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]));
    }

    /// Paint one tile onto a 9 × 9 all-sky frame.
    fn paint(frame: &mut [Texel], corners: [Point; 4], texels: &[Texel]) {
        let mappings = texture_mappings(corners).unwrap();
        let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
        sampler.prepare(&super::super::tile_sampling::lit_rows_of(texels, TILE_EDGE_TEXELS));
        draw_tile(frame, 9, corners, &mappings, &sampler, texels);
    }
    fn close(actual: Texel, expected: Texel) {
        for (a, b) in actual.rgb.into_iter().chain([actual.opacity]).zip(expected.rgb.into_iter().chain([expected.opacity])) {
            assert!((a - b).abs() < 1e-6, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn a_whole_tile_reduced_to_one_pixel_keeps_off_center_star_light() {
        let edge = TILE_EDGE_TEXELS;
        let mut texels = vec![Texel::default(); edge * edge];
        texels[0] = Texel { rgb: [0.8, 0.4, 0.2], opacity: 0.8 };
        let mut frame = vec![Texel::default(); 81];
        paint(&mut frame, [[3.5, 3.5], [4.5, 3.5], [4.5, 4.5], [3.5, 4.5]], &texels);
        close(frame[40], texels[0].scaled(1.0 / (edge * edge) as f32));
        assert_eq!(frame.iter().filter(|texel| !texel.is_empty()).count(), 1);
    }

    #[test]
    fn a_quad_covers_each_pixel_once_and_region_layers_keep_source_over_order() {
        let edge = TILE_EDGE_TEXELS;
        let mut frame = vec![Texel::default(); 81];
        let corners = [[2.5, 2.5], [5.5, 2.5], [5.5, 5.5], [2.5, 5.5]];
        for color in [[0.5, 0.0, 0.0], [0.0, 0.0, 0.5]] {
            paint(&mut frame, corners, &vec![Texel { rgb: color, opacity: 0.5 }; edge * edge]);
        }
        close(frame[40], Texel { rgb: [0.25, 0.0, 0.5], opacity: 0.75 });                  // the second tile over the first, not four half-blends
        assert_eq!(frame.iter().filter(|texel| !texel.is_empty()).count(), 9);
        assert!(frame[0].is_empty());

        let mut magnified = vec![Texel::default(); 81];
        let mut texels = vec![Texel::default(); edge * edge];
        texels[32 * edge + 31] = Texel { rgb: [1.0, 1.0, 1.0], opacity: 1.0 };              // one lit texel just left of and below the middle
        paint(&mut magnified, [[-100.0, -100.0], [108.0, -100.0], [108.0, 108.0], [-100.0, 108.0]], &texels); // 3.25 pixels per texel
        let lit: Vec<_> = magnified.iter().enumerate().filter(|(_, texel)| !texel.is_empty()).map(|(index, _)| (index / 9, index % 9)).collect();
        assert!(!lit.is_empty() && lit.iter().all(|&(row, column)| (2..=8).contains(&row) && (0..=5).contains(&column)), "{lit:?}"); // the texel, spread one texel wide by the filter
        let brightest = magnified.iter().enumerate().max_by(|a, b| a.1.opacity.total_cmp(&b.1.opacity)).map(|(index, _)| (index / 9, index % 9)).unwrap();
        assert!((5..=6).contains(&brightest.0) && (2..=3).contains(&brightest.1), "{brightest:?}");   // the texel's centre lands at (5.6, 2.4)
        assert!(magnified.iter().all(|texel| texel.opacity <= 1.0 && texel.rgb.iter().all(|&c| c <= 1.0)));
    }

    #[test]
    fn whole_tiles_place_stars_near_their_observed_positions_and_clear_empty_views() {
        let time = crate::astro::utc_jd_to_time_ms(J2000).unwrap();
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        for (fov, atmosphere, projection) in [(180.0, false, ProjectionKind::Stereographic), (60.0, true, ProjectionKind::Stereographic),
            (10.0, true, ProjectionKind::Equidistant)] {
            let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.02 }, projection, fov_degrees: fov };
            let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(crate::constants::GRID_DEPTH, direction);
            let star = fixtures::row(direction, 0.0, 3.0, 1);
            let mut data = fixtures::layout(&[(region, 0, star)]);
            let selected = Selected { regions: vec![region], whole_sky: false };
            let settings = RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
            refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::square(401);
            let projected_stars = project_whole_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
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
            assert!((center[0] / total - expected[0]).abs() < 3.0 && (center[1] / total - expected[1]).abs() < 3.0,
                "{fov}: center {center:?} / {total}, expected {expected:?}");
            let worst = frame.stars().iter().enumerate().filter(|(_, texel)| !(texel.opacity.is_finite() && texel.opacity <= 1.0)).map(|(index, texel)| (index / 401, index % 401, *texel)).next();
            assert!(worst.is_none(), "{fov}: {worst:?}");

            let projected_stars = project_whole_tiles(frame.drawing_area_width_and_height, &data, &Selected { regions: Vec::new(), whole_sky: false }, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }

    #[test]
    #[ignore = "timing only; measures whole-tile mapping without loading external datasets"]
    fn time_whole_tile_mapping() {
        // Prepare a real small catalog once; the dense variant fills the same selected tiles synthetically.
        let mut report = crate::progress::Report::quiet();
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut report).unwrap(), &mut report).unwrap();
        let mut data = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        let time = crate::astro::utc_jd_to_time_ms(J2000).unwrap();
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: 180.0, ..View::default() };
        let observation = observe(&Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() }, true, &view, time, &mut StepTimes::default());
        let selected = crate::selection::select_regions(&data.star_regions, &observation, &mut StepTimes::default());
        let settings = RenderOptions { magnitude_threshold: 9.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
        refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
        let mut frame = Frame::new(1920, 1100, 1100, (10, 20));

        for dense in [false, true] {
            if dense {
                for &region in &selected.regions {
                    let image = &mut data.star_regions[region].merged_tile;
                    image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS * TILE_EDGE_TEXELS];
                    image.lit_rows = vec![u64::MAX >> (64 - TILE_EDGE_TEXELS); TILE_EDGE_TEXELS];
                }
            }

            // Exclude the first frame, then average five whole-tile renders without detailed timers.
            let mut elapsed = 0.0;
            for run in 0..6 {
                let start = std::time::Instant::now();
                let projected_stars = project_whole_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
                frame.insert_projected_stars(&projected_stars);
                let seconds = start.elapsed().as_secs_f64();
                if run > 0 { elapsed += seconds; }
                std::hint::black_box(frame.stars());
            }
            eprintln!("dense={dense}; selected={}; whole_tile_ms={:.3}; five frames, timing detail disabled",
                selected.regions.len(), elapsed * 200.0);
        }
    }
}

