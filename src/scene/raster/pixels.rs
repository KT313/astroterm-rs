//! Pure RGBA rasterization of the projected sky. Viewport units are pixels; no terminal I/O or astronomy lives here.
use crate::constants::PIXEL_BACKGROUND_RGBA;
mod stars;
#[cfg(test)]
mod validation;
use image::RgbaImage;
use crate::scene::pipeline::draw_pixel_sky_from_inputs;

pub(in crate::scene) use stars::{initialize_star_layer, prepare_star_opacities, draw_pixel_stars, composite_star_layer};
pub(crate) use stars::{pixel_star_fits, select_pixel_star_labels};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::model::{RenderOptions, ObservedStarView, PlanetKind, ProjectedSky, ScreenPoint};
use crate::timing::StepTimes;


pub fn draw_pixel_sky(sky: &ProjectedSky<'_>, options: &RenderOptions, times: &mut StepTimes) -> Option<RgbaImage> {
    let mut storage = crate::state::SceneCache::default(); // headless callers still use an explicit buffer owner
    let mut stars = Vec::with_capacity(sky.stars.len());
    super::super::caching::prepare_pixel_star_inputs(sky, options, &mut stars);
    draw_pixel_sky_from_inputs(&mut storage.star_layer, &mut storage.star_opacities, &mut storage.image_scratch, sky, options, times, stars)
}

/// Build the sky image in `scratch` (the allocation displaced by the last store) and paint the background once.
pub(in crate::scene) fn initialize_pixel_canvas(viewport: crate::model::ProjectionViewport, scratch: &mut Vec<u8>) -> Option<Pixmap> {
    let (width, height) = (
        u32::try_from(viewport.width).ok()?,
        u32::try_from(viewport.height).ok()?,
    );
    if u64::from(width) * u64::from(height) > crate::constants::MAX_IMAGE_PIXELS as u64 {
        return None;
    }
    let size = tiny_skia::IntSize::from_wh(width, height)?;
    let length = width as usize * height as usize * 4;
    let mut data = std::mem::take(scratch);
    if data.len() != length {
        data.clear();
        if data.try_reserve_exact(length).is_err() { *scratch = data; return None; }
        data.resize(length, 0);
    }
    fill_background(&mut data);
    Pixmap::from_vec(data, size)
}

/// Opaque background, so the premultiplied bytes equal the straight colour; one word store per pixel.
fn fill_background(data: &mut [u8]) {
    let pixel = [PIXEL_BACKGROUND_RGBA[0], PIXEL_BACKGROUND_RGBA[1], PIXEL_BACKGROUND_RGBA[2], 255];
    match bytemuck::try_cast_slice_mut::<u8, u32>(data) {
        Ok(words) => words.fill(u32::from_ne_bytes(pixel)),
        Err(_) => for target in data.chunks_exact_mut(4) { target.copy_from_slice(&pixel); },
    }
}

pub(in crate::scene) fn draw_pixel_horizon(canvas: &mut Pixmap, sky: &ProjectedSky<'_>) {
    for &[a, b] in sky.horizon {
        draw_segment(canvas, a, b, [80, 110, 150], 1.2);
    }
}

/// Every visible arc goes into one path and is stroked once; a stroke per segment cost a blitter setup each.
pub(in crate::scene) fn draw_pixel_constellations(canvas: &mut Pixmap, sky: &ProjectedSky<'_>, options: &RenderOptions) {
    if !options.constellations { return; }
    let mut path = PathBuilder::new();
    for figure in sky.constellations.iter().filter(|figure| figure.maximum_magnitude <= options.magnitude_threshold) {
        for arc in &figure.arcs {
            let mut points = arc.points.iter().map(|&(y, x)| (x as f32, y as f32));
            let Some((x, y)) = points.next() else { continue; };
            path.move_to(x, y);                                                       // each arc is its own subpath; arcs are not joined to each other
            for (x, y) in points { path.line_to(x, y); }
        }
    }
    let Some(path) = path.finish() else { return; };
    stroke_path(canvas, &path, [68, 86, 112], 0.8);
}

pub(in crate::scene) fn draw_pixel_planets(canvas: &mut Pixmap, sky: &ProjectedSky<'_>) {
    for planet in sky.planets.iter().rev() {
        if let Some((y, x)) = planet.cell {
            let radius = match planet.kind {
                PlanetKind::Sun => 7.0,
                PlanetKind::Jupiter => 4.0,
                _ => 3.0,
            };
            draw_disc(canvas, x as f32, y as f32, radius, planet_rgb(planet.kind));
        }
    }
}

pub(in crate::scene) fn draw_pixel_moon(canvas: &mut Pixmap, sky: &ProjectedSky<'_>) {
    if let Some((y, x)) = sky.moon.cell {
        draw_moon_disc(
            canvas,
            (x as f32, y as f32),
            9.0,
            sky.moon.illumination.illuminated_fraction,
            sky.moon.light_direction,
        );
    }
}

pub(in crate::scene) fn draw_pixel_grid(canvas: &mut Pixmap, sky: &ProjectedSky<'_>, options: &RenderOptions) {
    let (width, height) = (canvas.width(), canvas.height());
    if options.grid && !sky.facing {
        let (cx, cy) = ((width as f64 - 1.0) * 0.5, (height as f64 - 1.0) * 0.5);
        for angle in (0..360).step_by(30) {
            let (s, c) = (angle as f64).to_radians().sin_cos();
            draw_segment(
                canvas,
                (cy as i32, cx as i32),
                ((cy - cy * s).round() as i32, (cx + cx * c).round() as i32),
                [36, 55, 77],
                0.7,
            );
        }
    }
}

pub(crate) fn star_rgb(star: &ObservedStarView<'_>) -> [u8; 3] {
    star.display_color().rgb()
}

pub(crate) fn planet_rgb(kind: PlanetKind) -> [u8; 3] {
    match kind {
        PlanetKind::Sun => [255, 223, 115],
        PlanetKind::Mercury => [195, 190, 185],
        PlanetKind::Venus => [255, 234, 183],
        PlanetKind::Mars => [246, 123, 82],
        PlanetKind::Jupiter => [224, 183, 146],
        PlanetKind::Saturn => [230, 209, 151],
        PlanetKind::Uranus => [142, 225, 231],
        PlanetKind::Neptune => [96, 145, 255],
    }
}

fn draw_disc(canvas: &mut Pixmap, x: f32, y: f32, radius: f32, rgb: [u8; 3]) {
    let Some(path) = PathBuilder::from_circle(x, y, radius) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn draw_segment(canvas: &mut Pixmap, a: (i32, i32), b: (i32, i32), rgb: [u8; 3], width: f32) {
    let mut path = PathBuilder::new();
    path.move_to(a.1 as f32, a.0 as f32);
    path.line_to(b.1 as f32, b.0 as f32);
    let Some(path) = path.finish() else {
        return;
    };
    stroke_path(canvas, &path, rgb, width);
}

fn stroke_path(canvas: &mut Pixmap, path: &tiny_skia::Path, rgb: [u8; 3], width: f32) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    canvas.stroke_path(path, &paint, &Stroke { width, ..Stroke::default() }, Transform::identity(), None);
}

/// Illuminate a sphere: n·light > 0, with cos(phase angle) = 2*fraction-1. Four subpixel samples soften both
/// limb and terminator. A degenerate projected Sun direction matters only near a full/new Moon.
fn draw_moon_disc(canvas: &mut Pixmap, center: (f32, f32), radius: f32, fraction: f64, direction: Option<ScreenPoint>) {
    let direction = direction.unwrap_or(ScreenPoint { x: 1.0, y: 0.0 });
    let cos_phase = 2.0 * fraction.clamp(0.0, 1.0) - 1.0;
    let sin_phase = (1.0 - cos_phase * cos_phase).sqrt();
    let width = canvas.width() as usize;
    let height = canvas.height() as i32;
    for y in
        ((center.1 - radius - 1.0).floor() as i32).max(0)..=((center.1 + radius + 1.0).ceil() as i32).min(height - 1)
    {
        for x in ((center.0 - radius - 1.0).floor() as i32).max(0)
            ..=((center.0 + radius + 1.0).ceil() as i32).min(width as i32 - 1)
        {
            let offset = (y as usize * width + x as usize) * 4;
            let pixel = &mut canvas.data_mut()[offset..offset + 4];
            let original = [pixel[0], pixel[1], pixel[2]];
            let mut sum = [0_u16; 3];
            for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let dx = f64::from((x as f32 + sx - center.0) / radius);
                let dy = f64::from((y as f32 + sy - center.1) / radius);
                let r2 = dx * dx + dy * dy;
                let color = if r2 > 1.0 {
                    original
                } else {
                    let dot = (dx * direction.x - dy * direction.y) * sin_phase + (1.0 - r2).sqrt() * cos_phase;
                    if dot > 0.0 { [230, 234, 219] } else { [23, 29, 40] }
                };
                for i in 0..3 {
                    sum[i] += u16::from(color[i]);
                }
            }
            for i in 0..3 {
                pixel[i] = (sum[i] / 4) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Color;
    #[test]
    fn moon_phase_and_screen_direction_control_the_lit_half() {
        for (fraction, direction, left, right) in [
            (0.0, ScreenPoint { x: 1.0, y: 0.0 }, false, false),
            (1.0, ScreenPoint { x: 1.0, y: 0.0 }, true, true),
            (0.5, ScreenPoint { x: 1.0, y: 0.0 }, false, true),
            (0.5, ScreenPoint { x: -1.0, y: 0.0 }, true, false),
        ] {
            let mut canvas = Pixmap::new(32, 32).unwrap();
            canvas.fill(Color::BLACK);
            draw_moon_disc(&mut canvas, (16.0, 16.0), 10.0, fraction, Some(direction));
            assert_eq!(canvas.pixel(11, 16).unwrap().red() > 100, left);
            assert_eq!(canvas.pixel(21, 16).unwrap().red() > 100, right);
            assert_eq!(canvas.pixel(0, 0).unwrap().red(), 0);
        }
        let mut canvas = Pixmap::new(32, 32).unwrap();
        canvas.fill(Color::BLACK);
        draw_moon_disc(
            &mut canvas,
            (16.0, 16.0),
            10.0,
            0.5,
            Some(ScreenPoint { x: 0.0, y: 1.0 }),
        );
        assert!(canvas.pixel(16, 11).unwrap().red() > 100);
        assert!(canvas.pixel(16, 21).unwrap().red() < 100);
    }

    #[test]
    fn raster_disc_hits_its_center_and_preserves_distant_background() {
        let mut canvas = Pixmap::new(32, 32).unwrap();
        canvas.fill(Color::BLACK);
        draw_disc(&mut canvas, 16.0, 16.0, 3.0, [240, 100, 50]);
        assert_eq!(canvas.pixel(16, 16).unwrap().red(), 240);
        assert_eq!(canvas.pixel(2, 2).unwrap().red(), 0);
    }
}
