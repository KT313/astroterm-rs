//! Step 6 of the frame: `draw_points`.
//!
//! Inputs: the frame (star layer and overlay), the simulated solar system, the sky tables, the region caches
//! (constellation simulation and the label candidates), the selected regions, the observation, the view and the
//! render settings. Output: the complete frame buffer: the few point objects drawn over the star layer, the
//! constellation endpoint stars added to the star layer, and the text (labels, compass and horizon labels, the
//! metadata panel, the notices) on the overlay. Runs every frame; few objects, so it is cheap. It draws into the
//! frame only; nothing here touches the terminal.
//!
//! Positions come from the observation (`apparent_direction`) and the keep-list projection geometry, exactly as
//! the old per-star path computed them, so bodies and lines land where the old renderer put them.
use crate::astro::Vector3;
use crate::astro::accuracy::{ACCURACY_WARNING, needs_accuracy_warning};
use crate::catalog::{MAGNITUDE_CLIPPING_WARNING, NameId, magnitude_code};
use crate::constants::{DYNAMIC_NAME_COUNT, PIXEL_BACKGROUND_RGBA};
use crate::metadata::MetadataField;
use crate::model::{LabelColumn, RenderOptions, SkyTables, StarColor, View};
use crate::observation::Observation;
use crate::projection::{Frame, ProjectedArc, ScreenPoint, SkyArea, prepare_camera, project_camera, project_constellation_segment, project_horizon_labels, project_horizon_line, project_light_direction, project_to_cell};
use crate::regions::{RegionCaches, UNPLACED, blend_star, order_star_rows, premultiplied_color, rows_wanted, zoom_opacity_boost};
use crate::text::draw_text_over;
use crate::selection::Selected;
use crate::solar_system::SolarSystem;
use std::cmp::Ordering;
use tiny_skia::{FillRule, Paint, PathBuilder, PixmapMut, Stroke, Transform};

/// What step 6 reads besides the observation, the view and the settings.
pub struct PointInputs<'a> {
    pub solar_system: &'a SolarSystem,
    pub tables: &'a SkyTables,
    pub caches: &'a RegionCaches,            // constellation simulation and the simulated prefixes the labels are chosen from
    pub selected: &'a Selected,
    pub panel: &'a [MetadataField],          // metadata lines for the top left corner; empty when the panel is off
    pub notice: Option<&'static str>,        // the display's transport notice, shown above the warnings; none for a headless frame
}

/// A text label the step placed, for tests and the half-block fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    pub pixel: (i32, i32),                  // frame pixel (row, column) of the text origin
    pub rgb: [u8; 3],
}

/// What one call drew, for tests and timing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub star_labels: Vec<Label>,            // the dynamic names, dimmest first as painted
    pub body_labels: Vec<Label>,
    pub direction_labels: Vec<Label>,       // compass points, zenith, nadir, grid angles
    pub notices: Vec<&'static str>,
    pub endpoint_stars: usize,              // constellation stars added to the star layer
    pub arcs: usize,                        // constellation arcs stroked
    pub horizon_segments: usize,
    pub bodies: usize,                      // Sun, planets and moons drawn
}

const PLANET_RADIUS: f32 = 3.0;
const JUPITER_RADIUS: f32 = 4.0;
const SUN_RADIUS: f32 = 7.0;
const MOON_RADIUS: f32 = 9.0;
const HORIZON_RGB: [u8; 3] = [80, 110, 150];
const CONSTELLATION_RGB: [u8; 3] = [68, 86, 112];
const GRID_RGB: [u8; 3] = [36, 55, 77];
const DIRECTION_RGB: [u8; 3] = [92, 145, 255];                                      // the old text layer's light blue
const NOTICE_RGB: [u8; 3] = [255, 255, 0];
const PANEL_RGB: [u8; 3] = [255, 255, 255];

/// Draw everything but the warped stars into the frame.
pub fn draw_points(frame: &mut Frame, inputs: &PointInputs<'_>, observation: &Observation, view: &View, settings: &RenderOptions) -> Drawn {
    let mut drawn = Drawn::default();
    frame.clear_overlay();
    if frame.width() == 0 || frame.height() == 0 { return drawn; }                       // nothing to draw on
    let sky = frame.sky();
    let viewport = sky.viewport();
    let camera = prepare_camera(view);
    let pixel_of = |horizon: Vector3| project_camera(camera, horizon).filter(|point| point.is_visible()).map(|point| sky.frame_pixel(project_to_cell(viewport, point)));
    let threshold = magnitude_code(settings.magnitude_threshold);
    let tt = observation.time.tt;

    // constellation endpoint stars join the star layer as glyphs, like the tiles' stars
    let constellations = inputs.tables.constellations.table();
    let simulation = inputs.caches.simulations().constellations();
    let boost = zoom_opacity_boost(view.fov_degrees) as f32;
    let mut endpoint_pixels = Vec::with_capacity(constellations.len());
    let rows = simulation.len().min(constellations.len());
    for row in 0..rows {
        endpoint_pixels.push(pixel_of(observation.apparent_star_direction(simulation.directions()[row])));
    }
    for row in order_star_rows(&simulation.magnitudes()[..rows], &constellations.columns().id[..rows], threshold) {
        let code = simulation.magnitudes()[row];
        let Some(pixel) = endpoint_pixels[row] else { continue };
        if !sky.glyph_fits((pixel.0 - sky.y as i32, pixel.1 - sky.x as i32)) { continue; }
        let opacity = inputs.caches.opacities().of(code) * boost;
        let premultiplied = premultiplied_color(constellations.columns().display_color[row], opacity);
        let (width, stars) = (frame.width(), frame.stars_mut());
        let bottom_right = pixel.0 as usize * width + pixel.1 as usize;
        for offset in [bottom_right - width - 1, bottom_right - width, bottom_right - 1, bottom_right] { blend_star(&mut stars[offset], premultiplied, opacity); }
        drawn.endpoint_stars += 1;
    }

    // the overlay: horizon, constellation lines, grid, bodies
    let (width, height) = (frame.width(), frame.height());
    {
        let (overlay, _) = frame.overlay_and_text();
        let mut canvas = PixmapMut::from_bytes(overlay, width as u32, height as u32).expect("the overlay is width × height RGBA");
        if view.is_facing() {
            for [a, b] in project_horizon_line(view, viewport) {
                stroke_segment(&mut canvas, sky.frame_pixel(a), sky.frame_pixel(b), HORIZON_RGB, 1.2);
                drawn.horizon_segments += 1;
            }
        }
        if settings.constellations {
            drawn.arcs = stroke_constellations(&mut canvas, sky, observation, view, inputs, simulation, threshold);
        }
        if settings.grid && !view.is_facing() {
            let (cx, cy) = (sky.x as f64 + (sky.size as f64 - 1.0) * 0.5, sky.y as f64 + (sky.size as f64 - 1.0) * 0.5);
            let reach = (sky.size as f64 - 1.0) * 0.5;
            for angle in (0..360).step_by(30) {
                let (s, c) = (angle as f64).to_radians().sin_cos();
                stroke_segment(&mut canvas, (cy as i32, cx as i32), ((cy - reach * s).round() as i32, (cx + reach * c).round() as i32), GRID_RGB, 0.7);
            }
        }
        drawn.bodies = draw_bodies(&mut canvas, &pixel_of, inputs, observation, view, &mut drawn.body_labels);
    }

    // text: labels for the brightest stars, bodies and directions, the panel, the notices
    drawn.star_labels = select_star_labels(inputs, observation, settings, &pixel_of, sky, &endpoint_pixels);
    drawn.direction_labels = direction_labels(sky, view, settings, viewport);
    drawn.notices = [needs_accuracy_warning(tt).then_some(ACCURACY_WARNING), inputs.tables.clipping.any().then_some(MAGNITUDE_CLIPPING_WARNING), inputs.notice].into_iter().flatten().collect();
    let cell = frame.text_cell();
    let (cell_width, cell_height) = (i32::from(cell.0), i32::from(cell.1));
    let (overlay, text) = frame.overlay_and_text();
    for label in drawn.star_labels.iter().chain(&drawn.body_labels) {
        draw_text_over(text, overlay, width, height, &label.text, (label.pixel.1 + cell_width, label.pixel.0 - cell_height), label.rgb);
    }
    for label in &drawn.direction_labels {
        draw_text_over(text, overlay, width, height, &label.text, (label.pixel.1, label.pixel.0), label.rgb);
    }
    for (line, field) in inputs.panel.iter().enumerate() {
        draw_text_over(text, overlay, width, height, &format!("{}: {}", field.label, field.value), (0, line as i32 * cell_height), PANEL_RGB);
    }
    for (offset, notice) in drawn.notices.iter().enumerate() {
        let top = height as i32 - (offset as i32 + 1) * cell_height;
        if top < 0 { break; }
        fill_band(overlay, width, height, top, cell_height, [0, 0, 0]);                   // the old notices sat on a black line
        draw_text_over(text, overlay, width, height, notice, (0, top), NOTICE_RGB);
    }
    drawn
}

/// Sun, planets (outermost first, the Sun on top) and moons, with their labels.
fn draw_bodies(canvas: &mut PixmapMut<'_>, pixel_of: &dyn Fn(Vector3) -> Option<(i32, i32)>, inputs: &PointInputs<'_>, observation: &Observation, view: &View, labels: &mut Vec<Label>) -> usize {
    let bodies = &inputs.tables.bodies;
    let system = inputs.solar_system;
    let mut count = 0;
    for (body, &position) in bodies.planets.iter().zip(&system.planets).rev() {
        if body.id == crate::astro::models::BodyId::Earth { continue; }
        let Some(pixel) = pixel_of(observation.apparent_direction(position)) else { continue };
        let radius = if body.id == crate::astro::models::BodyId::Jupiter { JUPITER_RADIUS } else { PLANET_RADIUS };
        let rgb = body_rgb(body.id);
        fill_disc(canvas, pixel, radius, rgb);
        labels.push(Label { text: body.name.to_string(), pixel, rgb });
        count += 1;
    }
    if let Some(pixel) = pixel_of(observation.apparent_direction(system.sun)) {
        let rgb = body_rgb(bodies.sun.id);
        fill_disc(canvas, pixel, SUN_RADIUS, rgb);
        labels.push(Label { text: bodies.sun.name.to_string(), pixel, rgb });
        count += 1;
    }
    for (index, moon) in bodies.moons.iter().enumerate() {
        let moon_horizon = observation.apparent_direction(system.moon_position(index, observation.observer.position));
        let Some(pixel) = pixel_of(moon_horizon) else { continue };
        let illumination = system.moon_illumination(index);
        let light = project_light_direction(view, moon_horizon, observation.apparent_direction(system.sun));
        fill_moon_disc(canvas, pixel, MOON_RADIUS, illumination.illuminated_fraction, light);
        labels.push(Label { text: moon.name.to_string(), pixel, rgb: [255, 255, 255] });
        count += 1;
    }
    count
}

/// The colours of the old pixel renderer's bodies.
pub fn body_rgb(id: crate::astro::models::BodyId) -> [u8; 3] {
    use crate::astro::models::BodyId::*;
    match id {
        Sun => [255, 223, 115],
        Mercury => [195, 190, 185],
        Venus => [255, 234, 183],
        Earth => [96, 145, 255],
        Mars => [246, 123, 82],
        Jupiter => [224, 183, 146],
        Saturn => [230, 209, 151],
        Uranus => [142, 225, 231],
        Neptune => [96, 145, 255],
        Moon => [230, 234, 219],
    }
}

/// Stroke every figure whose endpoints are all bright enough, each arc its own subpath, one stroke call.
fn stroke_constellations(canvas: &mut PixmapMut<'_>, sky: SkyArea, observation: &Observation, view: &View, inputs: &PointInputs<'_>, simulation: &crate::regions::GroupSimulation, threshold: u16) -> usize {
    let viewport = sky.viewport();
    let mut path = PathBuilder::new();
    let mut arcs = 0;
    for figure in inputs.tables.constellations.figures() {
        let rows = figure.segments.iter().flatten().copied();
        let bright_enough = rows.clone().all(|row| simulation.magnitudes().get(row).is_some_and(|&code| code <= threshold && code != UNPLACED));
        if !bright_enough { continue; }
        for &[from, to] in &figure.segments {
            let (a, b) = (observation.apparent_star_direction(simulation.directions()[from]), observation.apparent_star_direction(simulation.directions()[to])); // as the endpoint stars themselves are placed
            for arc in project_constellation_segment(view, viewport, a, b) {
                add_arc(&mut path, sky, &arc);
                arcs += 1;
            }
        }
    }
    if let Some(path) = path.finish() { stroke_path(canvas, &path, CONSTELLATION_RGB, 0.8); }
    arcs
}

fn add_arc(path: &mut PathBuilder, sky: SkyArea, arc: &ProjectedArc) {
    let mut points = arc.points.iter().map(|&cell| { let (row, column) = sky.frame_pixel(cell); (column as f32, row as f32) });
    let Some((x, y)) = points.next() else { return };
    path.move_to(x, y);
    for (x, y) in points { path.line_to(x, y); }
}

/// The dynamic names: the brightest drawable stars in view (at most `DYNAMIC_NAME_COUNT`), chosen over the
/// simulated prefixes of the selected groups and the constellation stars, brighter current magnitude first and,
/// at equal codes, the higher catalog id first, as the old renderer did; dimmest first in the result.
fn select_star_labels(inputs: &PointInputs<'_>, observation: &Observation, settings: &RenderOptions, pixel_of: &dyn Fn(Vector3) -> Option<(i32, i32)>, sky: SkyArea, endpoint_pixels: &[Option<(i32, i32)>]) -> Vec<Label> {
    if !settings.dynamic_names || DYNAMIC_NAME_COUNT == 0 { return Vec::new(); }
    let threshold = magnitude_code(settings.magnitude_threshold);
    let tt = observation.time.tt;
    let mut best = Candidates::default();
    let names = &inputs.tables.names;
    let unicode = settings.unicode;
    for &region in &inputs.selected.regions {
        for (group, table) in inputs.tables.stars.region(region).iter().enumerate() {
            let rows = rows_wanted(table, tt, settings.magnitude_threshold);
            if rows == 0 { continue; }
            let simulation = inputs.caches.simulations().group(region, group);
            let columns = table.columns();
            for row in 0..rows.min(simulation.len()) {
                if best.full_above(columns.brightness_key[row]) { break; }                 // the key bounds the current magnitude, and keys ascend
                let code = simulation.magnitudes()[row];
                if code > threshold || code == UNPLACED { continue; }
                let Some(pixel) = pixel_of(observation.apparent_star_direction(simulation.directions()[row])) else { continue };
                if !sky.glyph_fits((pixel.0 - sky.y as i32, pixel.1 - sky.x as i32)) { continue; }
                let id = columns.id[row];
                best.offer((code, id), pixel, StarColor::rgb_of(columns.display_color[row]), || star_label(table.labels(), row, names, unicode));
            }
        }
    }
    let constellations = inputs.tables.constellations.table();
    let simulation = inputs.caches.simulations().constellations();
    for (row, &pixel) in endpoint_pixels.iter().enumerate().take(simulation.len().min(constellations.len())) {
        let code = simulation.magnitudes()[row];
        if code > threshold || code == UNPLACED { continue; }
        let Some(pixel) = pixel else { continue };
        if !sky.glyph_fits((pixel.0 - sky.y as i32, pixel.1 - sky.x as i32)) { continue; }
        let id = constellations.columns().id[row];
        best.offer((code, id), pixel, StarColor::rgb_of(constellations.columns().display_color[row]), || star_label(constellations.labels(), row, names, unicode));
    }
    best.entries.into_iter().rev().map(|(_, pixel, rgb, text)| Label { text, pixel, rgb }).collect()
}

/// A label candidate: rank (magnitude code, id), frame pixel, colour and text.
type Candidate = ((u16, u32), (i32, i32), [u8; 3], String);

/// The brightest label candidates so far, brightest first, at most `DYNAMIC_NAME_COUNT`.
#[derive(Default)]
struct Candidates {
    entries: Vec<Candidate>,
}

impl Candidates {
    /// Whether the list is full and every entry is brighter than a star whose brightness key is `key`.
    fn full_above(&self, key: u16) -> bool {
        self.entries.len() == DYNAMIC_NAME_COUNT && key > self.entries[DYNAMIC_NAME_COUNT - 1].0.0
    }
    /// Insert a candidate if it ranks among the brightest; the label text is built only then.
    fn offer(&mut self, rank: (u16, u32), pixel: (i32, i32), rgb: [u8; 3], label: impl FnOnce() -> String) {
        if self.entries.len() == DYNAMIC_NAME_COUNT && compare_rank(rank, self.entries[DYNAMIC_NAME_COUNT - 1].0) != Ordering::Less { return; }
        let position = self.entries.iter().position(|entry| compare_rank(rank, entry.0) == Ordering::Less).unwrap_or(self.entries.len());
        self.entries.insert(position, (rank, pixel, rgb, label()));
        self.entries.truncate(DYNAMIC_NAME_COUNT);
    }
}

/// Brighter code first; at equal codes the higher id first, as the old renderer named ties.
fn compare_rank(a: (u16, u32), b: (u16, u32)) -> Ordering {
    if a.0 == b.0 { b.1.cmp(&a.1) } else { a.0.cmp(&b.0) }
}

/// A star's label: its name entry in the requested mode (a proper name, or the designation the loader stored in
/// both scripts), a Gaia designation from the source id, or nothing, as the old renderer labelled stars.
pub fn star_label(labels: &LabelColumn, row: usize, names: &crate::catalog::StarNames, unicode: bool) -> String {
    match labels {
        LabelColumn::Names(entries) => names.get_for_mode(NameId::from_entry(entries[row]), unicode).map_or_else(String::new, str::to_string),
        LabelColumn::SourceIds(ids) => format!("Gaia DR3 {}", ids[row]),
    }
}

/// Compass points on the horizon (facing views) or on the frame edges and the grid angles (overhead views).
fn direction_labels(sky: SkyArea, view: &View, settings: &RenderOptions, viewport: crate::model::ProjectionViewport) -> Vec<Label> {
    let mut labels = Vec::new();
    if view.is_facing() {
        for (cell, text) in project_horizon_labels(view, viewport) {
            labels.push(Label { text: text.to_string(), pixel: sky.frame_pixel(cell), rgb: DIRECTION_RGB });
        }
        return labels;
    }
    let (top, left, size) = (sky.y as i32, sky.x as i32, sky.size as i32);
    if !settings.grid {                                                                   // the grid's angles replace the compass letters, which would overlap them
        for (pixel, text) in [((top, left + size / 2), "N"), ((top + size - 1, left + size / 2), "S"), ((top + size / 2, left), "E"), ((top + size / 2, left + size - 1), "W")] {
            labels.push(Label { text: text.to_string(), pixel, rgb: DIRECTION_RGB });
        }
    } else {
        for angle in (0..360).step_by(30) {
            let (s, c) = (angle as f64).to_radians().sin_cos();
            let row = f64::from(top) + (1.0 - s) * f64::from(size - 1) / 2.0;
            let column = f64::from(left) + (1.0 + c) * f64::from(size - 1) / 2.0;
            labels.push(Label { text: angle.to_string(), pixel: (row.round() as i32, column.round() as i32), rgb: DIRECTION_RGB });
        }
    }
    labels
}

fn fill_disc(canvas: &mut PixmapMut<'_>, (row, column): (i32, i32), radius: f32, rgb: [u8; 3]) {
    let Some(path) = PathBuilder::from_circle(column as f32, row as f32, radius) else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke_segment(canvas: &mut PixmapMut<'_>, a: (i32, i32), b: (i32, i32), rgb: [u8; 3], width: f32) {
    let mut path = PathBuilder::new();
    path.move_to(a.1 as f32, a.0 as f32);
    path.line_to(b.1 as f32, b.0 as f32);
    let Some(path) = path.finish() else { return };
    stroke_path(canvas, &path, rgb, width);
}

fn stroke_path(canvas: &mut PixmapMut<'_>, path: &tiny_skia::Path, rgb: [u8; 3], width: f32) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    canvas.stroke_path(path, &paint, &Stroke { width, ..Stroke::default() }, Transform::identity(), None);
}

/// The Moon as the old renderer drew it: a sphere lit where n·light > 0 with cos(phase angle) = 2 × fraction − 1,
/// four subpixel samples softening limb and terminator; the alpha is the covered share of the pixel.
fn fill_moon_disc(canvas: &mut PixmapMut<'_>, (row, column): (i32, i32), radius: f32, fraction: f64, direction: Option<ScreenPoint>) {
    let direction = direction.unwrap_or(ScreenPoint { x: 1.0, y: 0.0 });
    let cos_phase = 2.0 * fraction.clamp(0.0, 1.0) - 1.0;
    let sin_phase = (1.0 - cos_phase * cos_phase).sqrt();
    let (width, height) = (canvas.width() as i32, canvas.height() as i32);
    let (cx, cy) = (column as f32, row as f32);
    let data = canvas.data_mut();
    for y in ((cy - radius - 1.0).floor() as i32).max(0)..=((cy + radius + 1.0).ceil() as i32).min(height - 1) {
        for x in ((cx - radius - 1.0).floor() as i32).max(0)..=((cx + radius + 1.0).ceil() as i32).min(width - 1) {
            let mut sum = [0_u32; 4];
            for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let dx = f64::from((x as f32 + sx - cx) / radius);
                let dy = f64::from((y as f32 + sy - cy) / radius);
                let r2 = dx * dx + dy * dy;
                if r2 > 1.0 { continue; }
                let dot = (dx * direction.x - dy * direction.y) * sin_phase + (1.0 - r2).sqrt() * cos_phase;
                let color = if dot > 0.0 { [230, 234, 219] } else { [23, 29, 40] };
                for i in 0..3 { sum[i] += color[i]; }
                sum[3] += 255;
            }
            if sum[3] == 0 { continue; }
            let pixel = &mut data[(y as usize * width as usize + x as usize) * 4..][..4];
            for i in 0..4 { pixel[i] = (sum[i] / 4) as u8; }                                  // premultiplied: colour × covered share
        }
    }
}

/// An opaque band across the frame for the notices.
fn fill_band(overlay: &mut [u8], width: usize, height: usize, top: i32, rows: i32, rgb: [u8; 3]) {
    for y in top.max(0)..(top + rows).min(height as i32) {
        for pixel in overlay[y as usize * width * 4..(y as usize + 1) * width * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

/// The background colour of the sky, for the frame output.
pub fn background_rgb() -> [u8; 3] { [PIXEL_BACKGROUND_RGBA[0], PIXEL_BACKGROUND_RGBA[1], PIXEL_BACKGROUND_RGBA[2]] }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, J2000, Observer};
    use crate::catalog::{SpaceMotion, StarId, load_embedded_catalog};
    use crate::model::{ProjectionKind, ViewCenter};
    use crate::observation::observe;
    use crate::regions::refresh_regions;
    use crate::selection::select_regions;
    use crate::solar_system::simulate_solar_system;

    fn tables_of(catalog: crate::catalog::Catalog) -> SkyTables {
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn bsc5() -> SkyTables { tables_of(load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap()) }
    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    fn settings() -> RenderOptions { RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 6.5, dynamic_names: true } }

    /// A catalog of the given stars (unit direction, magnitude, id, optional name), nothing else.
    fn stars(entries: &[(Vector3, f64, u32, Option<&str>)]) -> SkyTables {
        let mut parsed = load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        let template = parsed.stars[0].clone();
        parsed.stars.clear();
        parsed.constellations.clear();
        parsed.hr_representatives.clear();
        parsed.names = crate::catalog::StarNames::default();
        for &(direction, magnitude, id, name) in entries {
            let mut star = template.clone();
            star.id = StarId(id);
            star.has_data = true;
            star.magnitude = magnitude;
            star.right_ascension = direction.y.atan2(direction.x);
            star.declination = direction.z.asin();
            star.space_motion = Some(SpaceMotion { distance_pc: 10.0, position: direction, velocity: Vector3 { x: 0.0, y: 0.0, z: 0.0 } });
            star.name = name.map(|name| parsed.names.insert(name).unwrap());
            star.designation = Some(crate::catalog::Designation::Hr(id));
            parsed.stars.push(star);
        }
        tables_of(parsed)
    }

    /// The whole frame for `tables` at `view`: steps 1 to 4 and 6 (the star layer stays empty unless asked).
    fn draw(tables: &SkyTables, view: &View, days: f64, settings: &RenderOptions, frame: &mut Frame) -> (Drawn, Observation) {
        let time = FrameTime::from_utc(J2000 + days);
        let observation = observe(&boston(), false, view, time);
        let solar_system = simulate_solar_system(&tables.bodies, time);
        let selected = select_regions(&tables.stars, &observation);
        let mut caches = RegionCaches::default();
        refresh_regions(&mut caches, tables, &selected, time, settings);
        let inputs = PointInputs { solar_system: &solar_system, tables, caches: &caches, selected: &selected, panel: &[], notice: None };
        (draw_points(frame, &inputs, &observation, view, settings), observation)
    }

    /// Catalog directions at the given horizon altitude and azimuth (degrees) for the zenith view at J2000.
    fn catalog_direction(azimuth_degrees: f64, altitude_degrees: f64) -> Vector3 {
        let observation = observe(&boston(), false, &View::default(), FrameTime::from_utc(J2000));
        observation.to_catalog.apply(crate::astro::Horizontal { azimuth: azimuth_degrees.to_radians(), altitude: altitude_degrees.to_radians() }.to_unit_vector())
    }

    fn overlay_pixel(frame: &Frame, row: i32, column: i32) -> [u8; 4] {
        let index = (row as usize * frame.width() + column as usize) * 4;
        frame.overlay()[index..index + 4].try_into().unwrap()
    }
    fn lit_overlay(frame: &Frame) -> usize { frame.overlay().chunks_exact(4).filter(|pixel| pixel[3] != 0).count() }

    #[test]
    fn overlapping_endpoint_stars_paint_dimmest_first_and_break_ties_by_ascending_id() {
        let direction = catalog_direction(90.0, 80.0);
        let colors = [StarColor::RedOrange, StarColor::BlueWhite, StarColor::Orange];
        for (magnitudes, expected_order) in [([0.0, 3.0, 6.0], [2, 1, 0]), ([3.0; 3], [1, 2, 0])] {
            let mut columns = crate::model::StarRowVec::new();
            for (i, id) in [30, 10, 20].into_iter().enumerate() {
                let code = magnitude_code(magnitudes[i]);
                columns.push(crate::model::StarRow { u0: [direction.x as f32, direction.y as f32, direction.z as f32],
                    w: [0.0; 3], distance: 0.0, magnitude: code, brightness_key: code, id, name: 0,
                    display_color: colors[i].index() }.into());
            }
            let tables = SkyTables { constellations: crate::model::Constellations::new(columns, vec![0; 3], Vec::new()), ..SkyTables::default() };
            let mut frame = Frame::square(201);
            let (drawn, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
            let mut expected = crate::regions::Texel::default();
            for row in expected_order {
                let opacity = crate::regions::star_opacity(magnitudes[row], 1.0);
                blend_star(&mut expected, premultiplied_color(colors[row].index(), opacity), opacity);
            }
            assert_eq!(drawn.endpoint_stars, 3);
            let lit: Vec<_> = frame.stars().iter().filter(|texel| !texel.is_empty()).collect();
            assert_eq!(lit.len(), 4);
            assert!(lit.iter().all(|&&texel| texel == expected));
        }
    }

    #[test]
    fn dynamic_names_go_to_the_five_brightest_stars_in_view_dimmest_first() {
        let entries: Vec<_> = (0..7).map(|step| (catalog_direction(step as f64 * 40.0, 60.0), 1.0 + step as f64 * 0.5, 100 + step, None)).collect();
        let tables = stars(&entries);
        let mut frame = Frame::square(401);
        let (drawn, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
        let labels: Vec<&str> = drawn.star_labels.iter().map(|label| label.text.as_str()).collect();
        assert_eq!(labels, ["HR 104", "HR 103", "HR 102", "HR 101", "HR 100"]);        // the two dimmest are left out; unnamed stars show their catalog number
        assert!(lit_overlay(&frame) > 0);
        let off = RenderOptions { dynamic_names: false, ..settings() };
        let (drawn, _) = draw(&tables, &View::default(), 0.0, &off, &mut frame);
        assert!(drawn.star_labels.is_empty());
    }

    #[test]
    fn labels_follow_current_magnitude_then_descending_id_and_skip_stars_too_dim_or_at_the_edge() {
        let mut entries: Vec<_> = (0..6).map(|id| (catalog_direction(id as f64 * 50.0, 70.0), 5.0, id, None)).collect(); // equal magnitudes: ties
        entries.push((catalog_direction(0.0, 0.0), 0.5, 99, Some("Edge")));                  // brightest, but on the horizon at the top edge: no full glyph
        let tables = stars(&entries);
        let mut frame = Frame::square(401);
        let (drawn, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
        let labels: Vec<&str> = drawn.star_labels.iter().map(|label| label.text.as_str()).collect();
        assert_eq!(labels, ["HR 1", "HR 2", "HR 3", "HR 4", "HR 5"]);                    // ties by descending id: 5 first, id 0 and the edge star out
        let dim = RenderOptions { magnitude_threshold: 3.0, ..settings() };
        let (drawn, _) = draw(&tables, &View::default(), 0.0, &dim, &mut frame);
        assert!(drawn.star_labels.is_empty());                                              // too dim to be drawn, so never named
    }

    #[test]
    fn proper_names_follow_the_unicode_switch_and_gaia_labels_come_from_the_source_id() {
        let mut parsed = load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        parsed.names = crate::catalog::StarNames::default();
        let template = parsed.stars[0].clone();
        parsed.stars.clear(); parsed.constellations.clear(); parsed.hr_representatives.clear();
        let mut star = template;
        star.id = StarId(42); star.has_data = true; star.magnitude = 1.0; star.name = None;
        star.designation = Some(crate::catalog::Designation::Bayer { letter: 0, component: 2, constellation: *b"Cen" });
        let direction = catalog_direction(90.0, 80.0);
        star.right_ascension = direction.y.atan2(direction.x); star.declination = direction.z.asin();
        star.space_motion = Some(SpaceMotion { distance_pc: 10.0, position: direction, velocity: Vector3 { x: 0.0, y: 0.0, z: 0.0 } });
        parsed.stars.push(star);
        let tables = tables_of(parsed);
        let mut frame = Frame::square(201);
        let (unicode, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
        assert_eq!(unicode.star_labels[0].text, "α² Cen");
        let (ascii, _) = draw(&tables, &View::default(), 0.0, &RenderOptions { unicode: false, ..settings() }, &mut frame);
        assert_eq!(ascii.star_labels[0].text, "Alp2 Cen");
        assert_eq!(star_label(&LabelColumn::SourceIds(vec![4295806720]), 0, &tables.names, true), "Gaia DR3 4295806720");
        assert_eq!(star_label(&LabelColumn::Names(vec![0]), 0, &tables.names, true), "");       // no name, no designation: an empty label, as before
    }

    #[test]
    fn planets_and_the_moon_do_not_take_star_label_slots_and_land_at_the_old_projection() {
        let tables = bsc5();
        let view = View { center: ViewCenter::Facing { azimuth: 3.5, tilt: 0.3 }, projection: ProjectionKind::Stereographic, fov_degrees: 180.0 };
        let mut frame = Frame::square(401);
        let (drawn, observation) = draw(&tables, &view, 3000.0, &settings(), &mut frame);
        assert_eq!(drawn.star_labels.len(), DYNAMIC_NAME_COUNT);
        assert!(drawn.bodies >= 1 && drawn.body_labels.len() == drawn.bodies);
        let solar_system = simulate_solar_system(&tables.bodies, observation.time);
        let camera = prepare_camera(&view);
        for label in &drawn.body_labels {
            let position = match label.text.as_str() {
                "Sun" => solar_system.sun,
                "Moon" => solar_system.moon_position(0, observation.observer.position),
                name => solar_system.planets[tables.bodies.planets.iter().position(|body| body.name == name).unwrap()],
            };
            let point = project_camera(camera, observation.apparent_direction(position)).unwrap();
            let expected = project_to_cell(frame.sky().viewport(), point);
            assert_eq!(label.pixel, frame.sky().frame_pixel(expected), "{}", label.text);
            let pixel = overlay_pixel(&frame, label.pixel.0, label.pixel.1);
            assert_eq!(pixel[3], 255, "{} is drawn opaque at its centre", label.text);
        }
    }

    #[test]
    fn the_lit_side_of_the_moon_follows_the_sun_on_screen() {
        let mut canvas_bytes = vec![0_u8; 64 * 64 * 4];
        for (fraction, direction, left, right) in [(0.0, ScreenPoint { x: 1.0, y: 0.0 }, false, false), (1.0, ScreenPoint { x: 1.0, y: 0.0 }, true, true), (0.5, ScreenPoint { x: 1.0, y: 0.0 }, false, true), (0.5, ScreenPoint { x: -1.0, y: 0.0 }, true, false)] {
            canvas_bytes.fill(0);
            let mut canvas = PixmapMut::from_bytes(&mut canvas_bytes, 64, 64).unwrap();
            fill_moon_disc(&mut canvas, (32, 32), 10.0, fraction, Some(direction));
            let red = |x: usize, y: usize| canvas_bytes[(y * 64 + x) * 4];
            assert_eq!(red(27, 32) > 100, left, "{fraction} {direction:?}");
            assert_eq!(red(37, 32) > 100, right);
            assert_eq!(canvas_bytes[3], 0);                                                  // transparent away from the disc
        }
    }

    #[test]
    fn constellation_lines_are_stroked_only_when_enabled_and_all_endpoints_are_bright_enough() {
        let tables = bsc5();
        let view = View { center: ViewCenter::Facing { azimuth: 3.5, tilt: 0.6 }, projection: ProjectionKind::Stereographic, fov_degrees: 120.0 };
        let mut frame = Frame::square(401);
        let (bright, _) = draw(&tables, &view, 0.0, &RenderOptions { magnitude_threshold: 9.0, dynamic_names: false, ..settings() }, &mut frame);
        assert!(bright.arcs > 20 && bright.endpoint_stars > 20, "{bright:?}");
        let lit_with_lines = lit_overlay(&frame);
        let (strict, _) = draw(&tables, &view, 0.0, &RenderOptions { magnitude_threshold: 2.0, dynamic_names: false, ..settings() }, &mut frame);
        assert!(strict.arcs < bright.arcs / 2 && strict.endpoint_stars < bright.endpoint_stars, "{strict:?}"); // figures with a faint endpoint vanish whole
        let (disabled, _) = draw(&tables, &view, 0.0, &RenderOptions { magnitude_threshold: 9.0, constellations: false, dynamic_names: false, ..settings() }, &mut frame);
        assert_eq!(disabled.arcs, 0);
        assert!(lit_overlay(&frame) < lit_with_lines);
        assert_eq!(disabled.endpoint_stars, bright.endpoint_stars);                           // the endpoint stars themselves stay
    }

    #[test]
    fn constellation_lines_pass_through_their_endpoint_stars() {
        let tables = bsc5();
        let view = View { center: ViewCenter::Facing { azimuth: 5.9, tilt: 0.35 }, projection: ProjectionKind::Stereographic, fov_degrees: 120.0 };
        let mut frame = Frame::square(801);
        let (drawn, _) = draw(&tables, &view, 0.0, &RenderOptions { magnitude_threshold: 9.0, ..settings() }, &mut frame);
        assert!(drawn.arcs > 20 && drawn.star_labels.len() == 5, "{drawn:?}");
        let (width, overlay) = (frame.width(), frame.overlay());
        let line_near = |(row, column): (i32, i32)| (-2..=2).flat_map(|dr| (-2..=2).map(move |dc| (row + dr, column + dc)))
            .any(|(r, c)| r >= 0 && c >= 0 && (r as usize) < width && (c as usize) < width && overlay[((r as usize) * width + c as usize) * 4 + 3] > 0);
        let through = drawn.star_labels.iter().filter(|label| line_near(label.pixel)).count();             // the brightest stars of the view are figure endpoints
        assert!(through >= 3, "lines pass through {through} of 5 labelled stars: {:?}", drawn.star_labels);
    }

    #[test]
    fn the_horizon_is_hidden_when_looking_far_above_it_and_crosses_the_middle_of_a_level_view() {
        let tables = stars(&[]);
        let mut frame = Frame::square(201);
        let high = View { center: ViewCenter::Facing { azimuth: 0.0, tilt: 80_f64.to_radians() }, projection: ProjectionKind::Stereographic, fov_degrees: 60.0 };
        let (drawn, _) = draw(&tables, &high, 0.0, &settings(), &mut frame);
        assert_eq!(drawn.horizon_segments, 0);
        assert!(drawn.direction_labels.iter().all(|label| label.text != "N"));
        let level = View { center: ViewCenter::Facing { azimuth: 0.0, tilt: 0.0 }, projection: ProjectionKind::Stereographic, fov_degrees: 180.0 };
        let (drawn, _) = draw(&tables, &level, 0.0, &settings(), &mut frame);
        assert!(drawn.horizon_segments > 0);
        let middle: Vec<u8> = (0..201).map(|column| overlay_pixel(&frame, 100, column)[3]).collect();
        assert!(middle.iter().filter(|&&alpha| alpha > 0).count() > 150, "the horizon runs across the middle row");
        let texts: Vec<&str> = drawn.direction_labels.iter().map(|label| label.text.as_str()).collect();
        assert!(texts.contains(&"N") && texts.contains(&"NE") && texts.contains(&"NW") && !texts.contains(&"S"), "{texts:?}"); // South is behind the observer
        assert!(texts.contains(&"Zenith") && texts.contains(&"Nadir"));
    }

    #[test]
    fn the_overhead_view_shows_compass_letters_on_the_edges_and_the_grid_on_request() {
        let tables = stars(&[]);
        let mut frame = Frame::new(301, 201, SkyArea::centred(301, 201), (10, 20));
        let (plain, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
        let sky = frame.sky();
        let find = |drawn: &Drawn, text: &str| drawn.direction_labels.iter().find(|label| label.text == text).map(|label| label.pixel);
        assert_eq!(find(&plain, "N"), Some((sky.y as i32, sky.x as i32 + sky.size as i32 / 2)));
        assert_eq!(find(&plain, "S"), Some((sky.y as i32 + sky.size as i32 - 1, sky.x as i32 + sky.size as i32 / 2)));
        assert_eq!(find(&plain, "E"), Some((sky.y as i32 + sky.size as i32 / 2, sky.x as i32)));
        assert_eq!(find(&plain, "W"), Some((sky.y as i32 + sky.size as i32 / 2, sky.x as i32 + sky.size as i32 - 1)));
        assert_eq!(plain.direction_labels.len(), 4);
        let before = lit_overlay(&frame);
        let (grid, _) = draw(&tables, &View::default(), 0.0, &RenderOptions { grid: true, ..settings() }, &mut frame);
        assert_eq!(grid.direction_labels.len(), 12);                                      // the angles replace the compass letters
        assert!(find(&grid, "90").is_some() && find(&grid, "330").is_some() && find(&grid, "N").is_none());
        assert!(lit_overlay(&frame) > before + 300, "twelve spokes");
        let centre = overlay_pixel(&frame, sky.y as i32 + sky.size as i32 / 2, sky.x as i32 + sky.size as i32 / 2);
        assert!(centre[3] > 0);
    }

    #[test]
    fn the_overlay_does_not_depend_on_the_star_layer_and_the_notices_sit_at_the_bottom() {
        let tables = stars(&[(catalog_direction(10.0, 60.0), 2.0, 1, None)]);
        let mut frame = Frame::square(201);
        let (_, _) = draw(&tables, &View::default(), 0.0, &RenderOptions { dynamic_names: false, ..settings() }, &mut frame);
        let overlay_without = frame.overlay().to_vec();
        frame.stars_mut().iter_mut().for_each(|texel| *texel = crate::regions::Texel { rgb: [0.5; 3], opacity: 0.5 });
        let (_, _) = draw(&tables, &View::default(), 0.0, &RenderOptions { dynamic_names: false, ..settings() }, &mut frame);
        assert_eq!(frame.overlay(), overlay_without.as_slice());                            // the overlay is drawn over whatever the stars are
        assert!(frame.stars().iter().all(|texel| texel.opacity >= 0.5));                       // and never erases them

        let (far, _) = draw(&tables, &View::default(), 30000.0 * 365.25, &settings(), &mut frame);  // far outside the validated intervals
        assert_eq!(far.notices, [ACCURACY_WARNING]);
        let band = 201 - 20..201;
        assert!(band.clone().any(|row| (0..201).any(|column| overlay_pixel(&frame, row, column)[3] == 255 && overlay_pixel(&frame, row, column)[0] > 200))); // yellow text on the black band
        assert!(band.clone().all(|row| overlay_pixel(&frame, row, 150)[3] == 255 && overlay_pixel(&frame, row, 150)[0] < 60), "the band is opaque black where no text is");
        let mut clipped = stars(&[(catalog_direction(10.0, 60.0), 2.0, 1, None)]);
        clipped.clipping = crate::catalog::MagnitudeClipping { lower: 1, upper: 0 };
        let (both, _) = draw(&clipped, &View::default(), 30000.0 * 365.25, &settings(), &mut frame);
        assert_eq!(both.notices, [ACCURACY_WARNING, MAGNITUDE_CLIPPING_WARNING]);           // the clipping notice above the accuracy warning
        let (none, _) = draw(&tables, &View::default(), 0.0, &settings(), &mut frame);
        assert!(none.notices.is_empty());
        let time = FrameTime::from_utc(J2000 + 30000.0 * 365.25);
        let observation = observe(&boston(), false, &View::default(), time);
        let solar_system = simulate_solar_system(&clipped.bodies, time);
        let selected = select_regions(&clipped.stars, &observation);
        let mut caches = RegionCaches::default();
        refresh_regions(&mut caches, &clipped, &selected, time, &settings());
        let inputs = PointInputs { solar_system: &solar_system, tables: &clipped, caches: &caches, selected: &selected, panel: &[], notice: Some("Shared memory unavailable; using streaming.") };
        let streaming = draw_points(&mut frame, &inputs, &observation, &View::default(), &settings());
        assert_eq!(streaming.notices, [ACCURACY_WARNING, MAGNITUDE_CLIPPING_WARNING, "Shared memory unavailable; using streaming."]); // the display's notice on top
    }

    /// Release-mode cost of step 6 on BSC5 at full HD; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_draw_points_at_full_hd() {
        let tables = bsc5();
        let view = View { center: ViewCenter::Facing { azimuth: 3.5, tilt: 0.6 }, projection: ProjectionKind::Stereographic, fov_degrees: 120.0 };
        let time = FrameTime::from_utc(J2000);
        let observation = observe(&boston(), true, &view, time);
        let solar_system = simulate_solar_system(&tables.bodies, time);
        let selected = select_regions(&tables.stars, &observation);
        let mut caches = RegionCaches::default();
        let settings = RenderOptions { magnitude_threshold: 9.0, ..settings() };
        refresh_regions(&mut caches, &tables, &selected, time, &settings);
        let fields = vec![MetadataField { label: "Date".into(), value: "2000-01-01".into() }; 12];
        let inputs = PointInputs { solar_system: &solar_system, tables: &tables, caches: &caches, selected: &selected, panel: &fields, notice: None };
        let mut frame = Frame::new(1920, 1100, SkyArea::centred(1920, 1100), (9, 17));
        draw_points(&mut frame, &inputs, &observation, &view, &settings);
        let started = std::time::Instant::now();
        let frames = 10;
        let mut drawn = Drawn::default();
        for _ in 0..frames { drawn = draw_points(&mut frame, &inputs, &observation, &view, &settings); }
        eprintln!("draw_points {:?} per frame: {} endpoint stars, {} arcs, {} bodies, {} star labels, {} direction labels", started.elapsed() / frames, drawn.endpoint_stars, drawn.arcs, drawn.bodies, drawn.star_labels.len(), drawn.direction_labels.len());
    }

    #[test]
    fn the_metadata_panel_is_painted_at_the_top_left() {
        let tables = stars(&[]);
        let time = FrameTime::from_utc(J2000);
        let observation = observe(&boston(), true, &View::default(), time);
        let solar_system = simulate_solar_system(&tables.bodies, time);
        let selected = select_regions(&tables.stars, &observation);
        let caches = RegionCaches::default();
        let fields = [MetadataField { label: "Date".into(), value: "2000-01-01".into() }, MetadataField { label: "Site".into(), value: "Boston".into() }];
        let mut frame = Frame::new(301, 201, SkyArea::centred(301, 201), (10, 20));
        let inputs = PointInputs { solar_system: &solar_system, tables: &tables, caches: &caches, selected: &selected, panel: &fields, notice: None };
        draw_points(&mut frame, &inputs, &observation, &View::default(), &RenderOptions { dynamic_names: false, ..settings() });
        let line = |row: i32| (0..301).filter(|&column| overlay_pixel(&frame, row, column)[3] > 0).count();
        assert!(line(10) > 20 && line(30) > 20, "two text lines at rows 0..40");
        assert_eq!((60..100).map(line).sum::<usize>(), 0);
    }
}
