//! Exact fast path for the minimum-radius, integer-centered star used by the pixel renderer.
use tiny_skia::Pixmap;

pub(in crate::scene) const MINIMUM_STAR_RADIUS: f32 = 0.55;

/// Draw the 0.55-pixel circle with the same coverage and integer blending as tiny-skia 0.12's default pipeline.
/// At integer centers its 4×4 antialiasing grid covers four samples in each of the four adjoining pixels, giving
/// coverage 64. The default low-precision blend rounds (source*64 + destination*191) upward after division by 256.
/// All four premultiplied RGBA channels are blended, preserving both opaque scenes and transparent test canvases.
///
/// Restrict the shortcut to canvases at most 4096 pixels on either axis, where this footprint is qualified. Larger
/// canvases use the existing path renderer, including its tiling/large-coordinate behavior. There is no runtime
/// mask or lookup table, so `--disable-cache` executes this arithmetic directly too.
pub(super) fn draw_minimum_star(canvas: &mut Pixmap, x: i32, y: i32, rgb: [u8; 3]) -> bool {
    let (width, height) = (canvas.width(), canvas.height());
    if width > 4096 || height > 4096 {
        return false;
    }
    if x < 0 || y < 0 || x > width as i32 || y > height as i32 {
        return true; // an integer center beyond these bounds has no covered pixels in the canvas
    }

    // visit only the four adjoining pixels, clipped independently at the canvas edges
    let color = [rgb[0], rgb[1], rgb[2], 255];
    for row in [y - 1, y] {
        if row < 0 || row >= height as i32 {
            continue;
        }
        for column in [x - 1, x] {
            if column < 0 || column >= width as i32 {
                continue;
            }
            let offset = (row as usize * width as usize + column as usize) * 4;
            for (destination, source) in canvas.data_mut()[offset..offset + 4].iter_mut().zip(color) {
                *destination = ((u32::from(source) * 64 + u32::from(*destination) * 191 + 255) >> 8) as u8;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::raster::pixels::draw_disc;
    use tiny_skia::Color;

    #[test]
    fn every_source_and_background_channel_matches_the_path_renderer() {
        let mut actual = Pixmap::new(4, 4).unwrap();
        let mut expected = actual.clone();
        for background in 0..=255_u8 {
            for source in 0..=255_u8 {
                let color = Color::from_rgba8(background, 255 - background, background / 2, 255);
                actual.fill(color);
                expected.fill(color);
                let rgb = [source, source / 2, 255 - source];
                assert!(draw_minimum_star(&mut actual, 2, 2, rgb));
                draw_disc(&mut expected, 2.0, 2.0, MINIMUM_STAR_RADIUS, rgb);
                assert_eq!(
                    actual.data(),
                    expected.data(),
                    "background={background} source={source}"
                );
            }
        }
    }

    #[test]
    fn integer_coordinates_clipping_and_overlaps_match() {
        for (width, height) in [(1, 1), (2, 3), (33, 19), (4096, 4), (4, 4096)] {
            let mut actual = Pixmap::new(width, height).unwrap();
            let mut expected = actual.clone();
            for &(x, y) in &[
                (0, 0),
                (1, 1),
                (width as i32 - 1, height as i32 - 1),
                (width as i32, height as i32),
                (-1, 0),
                (0, -1),
                (i32::MIN, i32::MAX),
            ] {
                for rgb in [[210, 55, 17], [0, 0, 0], [255, 255, 255], [31, 93, 121]] {
                    assert!(draw_minimum_star(&mut actual, x, y, rgb));
                    draw_disc(&mut expected, x as f32, y as f32, MINIMUM_STAR_RADIUS, rgb);
                    assert_eq!(actual.data(), expected.data(), "{width}x{height}: {x},{y}");
                }
            }
            // sweep each coordinate on long strips, preserving overlaps on a transparent background
            if width == 4096 || height == 4096 {
                for n in 0..4096 {
                    let (x, y) = if width == 4096 { (n, 2) } else { (2, n) };
                    let rgb = [(n % 256) as u8, 83, 247];
                    assert!(draw_minimum_star(&mut actual, x, y, rgb));
                    draw_disc(&mut expected, x as f32, y as f32, MINIMUM_STAR_RADIUS, rgb);
                }
                assert_eq!(actual.data(), expected.data());
            }
        }
    }

    #[test]
    fn mixed_large_coordinates_and_fallback_preserve_the_canvas() {
        let mut actual = Pixmap::new(4096, 4096).unwrap();
        let mut expected = actual.clone();
        for x in [0, 1, 127, 255, 511, 1023, 2047, 4094, 4095, 4096] {
            for y in [0, 1, 127, 255, 511, 1023, 2047, 4094, 4095, 4096] {
                let rgb = [19, 177, 254];
                assert!(draw_minimum_star(&mut actual, x, y, rgb));
                draw_disc(&mut expected, x as f32, y as f32, MINIMUM_STAR_RADIUS, rgb);
            }
        }
        assert_eq!(actual.data(), expected.data());
        let mut large = Pixmap::new(4097, 1).unwrap();
        let original = large.clone();
        assert!(!draw_minimum_star(&mut large, 1, 0, [255; 3]));
        assert_eq!(large.data(), original.data());
    }
}
