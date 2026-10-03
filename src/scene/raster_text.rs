//! Antialiased monochrome font masks blended into an opaque RGBA frame. The bundled font keeps rendering
//! independent of installed fonts; unsupported characters use its visible replacement glyph.
use fontdue::{Font, FontSettings, Metrics};
use image::RgbaImage;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

const FONT: &[u8] = include_bytes!("../../data/fonts/DejaVuSansMono.ttf");
const MAX_CACHED_GLYPHS: usize = 512;

struct Glyph {
    metrics: Metrics,
    coverage: Vec<u8>,
}

pub struct TextRasterizer {
    font: Font,
    glyphs: HashMap<char, Glyph>,
    cell: (u16, u16),
    size: f32,
    baseline: f32,
}

impl TextRasterizer {
    /// Discard computed glyph masks in bypass mode; within-frame reuse remains permitted.
    pub fn begin_frame(&mut self, reuse_assets: bool) {
        if !reuse_assets {
            self.glyphs.clear();
        }
    }

    pub fn new() -> Result<Self, &'static str> {
        let font = Font::from_bytes(FONT, FontSettings::default())?;
        let mut renderer = Self {
            font,
            glyphs: HashMap::new(),
            cell: (0, 0),
            size: 1.0,
            baseline: 1.0,
        };
        renderer.set_cell_size(10, 20);
        Ok(renderer)
    }

    /// Preserve the measured terminal layout while using the application's own font. Resize invalidates masks.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        let cell = (width.max(1), height.max(1));
        if cell == self.cell {
            return;
        }
        self.cell = cell;
        self.glyphs.clear();
        let line = self
            .font
            .horizontal_line_metrics(1.0)
            .expect("bundled font has horizontal metrics");
        let advance = self.font.metrics('M', 1.0).advance_width;
        self.size = (f32::from(cell.0) / advance)
            .min(f32::from(cell.1) / line.new_line_size)
            .min(256.0);
        let line = self.font.horizontal_line_metrics(self.size).unwrap();
        self.baseline = (f32::from(cell.1) - line.new_line_size) * 0.5 + line.ascent;
    }

    /// Paint a prepared cell layout into the image. Backgrounds are applied before glyphs so wide characters
    /// and combining marks are not erased by neighboring cells. Reset backgrounds leave sky pixels visible.
    pub fn paint_buffer(&mut self, image: &mut RgbaImage, text: &ratatui::buffer::Buffer, cell: (u16, u16)) {
        self.set_cell_size(cell.0, cell.1);
        let (w, h) = (u32::from(self.cell.0), u32::from(self.cell.1));
        for y in text.area.top()..text.area.bottom() {
            for x in text.area.left()..text.area.right() {
                if let Some(rgb) = color_rgb(text[(x, y)].bg) {
                    for py in (u32::from(y) * h).min(image.height())..((u32::from(y) + 1) * h).min(image.height()) {
                        for px in (u32::from(x) * w).min(image.width())..((u32::from(x) + 1) * w).min(image.width()) {
                            image.put_pixel(px, py, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
                        }
                    }
                }
            }
        }
        for y in text.area.top()..text.area.bottom() {
            for x in text.area.left()..text.area.right() {
                let value = &text[(x, y)];
                if value.symbol().trim().is_empty() {
                    continue;
                }
                let columns = unicode_width::UnicodeWidthStr::width(value.symbol()).max(1) as u32;
                self.draw_text(
                    image,
                    value.symbol(),
                    ((u32::from(x) * w) as i32, (u32::from(y) * h) as i32),
                    (w * columns, h),
                    color_rgb(value.fg).unwrap_or([255; 3]),
                );
            }
        }
    }

    /// Draw text inside the supplied rectangle (pixel origin and extent). Advance is in terminal-cell units;
    /// combining marks share their base's cell. Negative origins and clipping at every image edge are supported.
    pub fn draw_text(
        &mut self,
        image: &mut RgbaImage,
        text: &str,
        origin: (i32, i32),
        extent: (u32, u32),
        color: [u8; 3],
    ) {
        let (left, top) = (origin.0.max(0), origin.1.max(0));
        let right = (i64::from(origin.0) + i64::from(extent.0)).clamp(0, i64::from(image.width())) as i32;
        let bottom = (i64::from(origin.1) + i64::from(extent.1)).clamp(0, i64::from(image.height())) as i32;
        if left >= right || top >= bottom {
            return;
        }
        let mut pen = origin.0;
        for ch in text
            .chars()
            .filter(|ch| !ch.is_control() && !matches!(ch, '\u{fe0f}' | '\u{fe0e}' | '\u{200d}'))
        {
            let columns = ch.width().unwrap_or(0);
            let ch = if self.font.lookup_glyph_index(ch) == 0 {
                '\u{fffd}'
            } else {
                ch
            };
            if !self.glyphs.contains_key(&ch) {
                if self.glyphs.len() >= MAX_CACHED_GLYPHS {
                    self.glyphs.clear();
                }
                let (metrics, coverage) = self.font.rasterize(ch, self.size);
                self.glyphs.insert(ch, Glyph { metrics, coverage });
            }
            let glyph = &self.glyphs[&ch];
            let x0 = pen + glyph.metrics.xmin;
            let y0 = origin.1 + self.baseline.round() as i32 - glyph.metrics.ymin - glyph.metrics.height as i32;
            for y in top.max(y0)..bottom.min(y0 + glyph.metrics.height as i32) {
                for x in left.max(x0)..right.min(x0 + glyph.metrics.width as i32) {
                    let alpha = u32::from(glyph.coverage[(y - y0) as usize * glyph.metrics.width + (x - x0) as usize]);
                    let pixel = image.get_pixel_mut(x as u32, y as u32);
                    for (channel, foreground) in pixel.0[..3].iter_mut().zip(color) {
                        *channel =
                            ((u32::from(foreground) * alpha + u32::from(*channel) * (255 - alpha) + 127) / 255) as u8;
                    }
                }
            }
            pen += i32::from(self.cell.0) * columns as i32;
        }
    }
}

fn color_rgb(color: ratatui::style::Color) -> Option<[u8; 3]> {
    use ratatui::style::Color::*;
    Some(match color {
        Reset => return None,
        Rgb(r, g, b) => [r, g, b],
        Black => [0, 0, 0],
        White => [255; 3],
        Gray => [192; 3],
        DarkGray => [128; 3],
        Red => [128, 0, 0],
        Green => [0, 128, 0],
        Yellow => [128, 128, 0],
        Blue => [0, 0, 128],
        Magenta => [128, 0, 128],
        Cyan => [0, 128, 128],
        LightRed => [255, 0, 0],
        LightGreen => [0, 255, 0],
        LightYellow => [255, 255, 0],
        LightBlue => [92, 145, 255],
        LightMagenta => [255, 0, 255],
        LightCyan => [0, 255, 255],
        Indexed(index) => {
            const ANSI: [[u8; 3]; 16] = [
                [0, 0, 0],
                [128, 0, 0],
                [0, 128, 0],
                [128, 128, 0],
                [0, 0, 128],
                [128, 0, 128],
                [0, 128, 128],
                [192; 3],
                [128; 3],
                [255, 0, 0],
                [0, 255, 0],
                [255, 255, 0],
                [0, 0, 255],
                [255, 0, 255],
                [0, 255, 255],
                [255; 3],
            ];
            if index < 16 {
                ANSI[index as usize]
            } else if index >= 232 {
                [8 + 10 * (index - 232); 3]
            } else {
                let i = index - 16;
                let levels = [0, 95, 135, 175, 215, 255];
                [
                    levels[(i / 36) as usize],
                    levels[((i / 6) % 6) as usize],
                    levels[(i % 6) as usize],
                ]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn font_covers_catalog_and_metadata_symbols_and_draws_antialiased_text() {
        let mut renderer = TextRasterizer::new().unwrap();
        for ch in "αβγδεζθλμξπσφω°−×♈éü".chars() {
            assert_ne!(renderer.font.lookup_glyph_index(ch), 0, "missing {ch}");
        }
        let mut image = RgbaImage::from_pixel(100, 24, Rgba([0, 0, 0, 255]));
        renderer.draw_text(&mut image, "α Vir 20°", (0, 0), (100, 24), [255, 255, 255]);
        assert!(image.pixels().any(|p| p[0] == 255));
        assert!(image.pixels().any(|p| p[0] > 0 && p[0] < 255));
        assert!(image.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn clipping_and_resize_do_not_leave_stale_glyph_masks() {
        let mut renderer = TextRasterizer::new().unwrap();
        let mut image = RgbaImage::from_pixel(40, 40, Rgba([0, 0, 0, 255]));
        renderer.draw_text(&mut image, "Test", (-4, -2), (16, 12), [255, 0, 0]);
        for (x, y, p) in image.enumerate_pixels() {
            if x >= 12 || y >= 10 {
                assert_eq!(*p, Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!renderer.glyphs.is_empty());
        renderer.set_cell_size(20, 40);
        assert!(renderer.glyphs.is_empty());
        renderer.draw_text(&mut image, "A", (0, 0), (40, 40), [255, 255, 255]);
        assert!(image.pixels().any(|p| p[1] > 0));
    }

    #[test]
    fn missing_glyph_is_visible_and_deterministic() {
        let mut renderer = TextRasterizer::new().unwrap();
        let mut missing = RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 255]));
        let mut replacement = missing.clone();
        renderer.draw_text(&mut missing, "\u{10ffff}", (0, 0), (20, 20), [255; 3]);
        renderer.draw_text(&mut replacement, "�", (0, 0), (20, 20), [255; 3]);
        assert_eq!(missing, replacement);
        assert!(missing.pixels().any(|p| p[0] != 0));
    }
}
