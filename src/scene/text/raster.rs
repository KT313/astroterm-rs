//! Antialiased monochrome font masks blended into an opaque RGBA frame. The bundled font keeps rendering
//! independent of installed fonts; unsupported characters use its visible replacement glyph.
use crate::constants::MAX_CACHED_GLYPHS;
use fontdue::{Font, FontSettings};
use image::RgbaImage;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

# [cfg (feature = "memory-diagnostics")] use crate::timing::Access;
# [cfg (feature = "memory-diagnostics")] use crate::timing::BufferId;
# [cfg (feature = "memory-diagnostics")] use crate::timing::BufferShape;
# [cfg (feature = "memory-diagnostics")] use crate::timing::IndexDomain;
# [cfg (feature = "memory-diagnostics")] use crate::timing::MemoryEvent;
# [cfg (feature = "memory-diagnostics")] use crate::timing::Operation;

const FONT: &[u8] = include_bytes!("../../../data/fonts/DejaVuSansMono.ttf");

use crate::model::Glyph;

use crate::state::TextRasterizer;

/// Discard computed glyph masks in bypass mode; within-frame reuse remains permitted.
pub fn begin_text_frame(state: &mut TextRasterizer, reuse_assets: bool) {
    if !reuse_assets {
        state.glyphs.clear();
    }
}

pub fn create_text_rasterizer() -> Result<TextRasterizer, &'static str> {
    let font = Font::from_bytes(FONT, FontSettings::default())?;
    let mut renderer = TextRasterizer {
        font,
        glyphs: HashMap::new(),
        cell: (0, 0),
        size: 1.0,
        baseline: 1.0,
        #[cfg(feature = "memory-diagnostics")]
        glyph_operations: None,
    };
    set_text_cell_size(&mut renderer, 10, 20);
    Ok(renderer)
}

/// Preserve the measured terminal layout while using the application's own font. Resize invalidates masks.
pub fn set_text_cell_size(state: &mut TextRasterizer, width: u16, height: u16) {
    let cell = (width.max(1), height.max(1));
    if cell == state.cell {
        return;
    }
    state.cell = cell;
    #[cfg(feature = "memory-diagnostics")]
    if let Some(counts) = &mut state.glyph_operations { counts.cleared += 1; counts.cleared_masks += state.glyphs.len(); }
    state.glyphs.clear();
    let line = state
        .font
        .horizontal_line_metrics(1.0)
        .expect("bundled font has horizontal metrics");
    let advance = state.font.metrics('M', 1.0).advance_width;
    state.size = (f32::from(cell.0) / advance)
        .min(f32::from(cell.1) / line.new_line_size)
        .min(256.0);
    let line = state.font.horizontal_line_metrics(state.size).unwrap();
    state.baseline = (f32::from(cell.1) - line.new_line_size) * 0.5 + line.ascent;
}

/// Measure one text pass and aggregate mask operations already visited by its drawing loops.
/// Counter increments are included in the pass time; descriptor construction is charged as diagnostics.
pub(crate) fn paint_text_buffer_with_times(state: &mut TextRasterizer, image: &mut RgbaImage, text: &ratatui::buffer::Buffer, cell: (u16, u16), times: &mut crate::timing::StepTimes) {
    #[cfg(feature = "memory-diagnostics")]
    { state.glyph_operations = times.inspect_memory(Default::default); }
    times.measure("Text rasterization", || paint_text_buffer(state, image, text, cell));
    #[cfg(feature = "memory-diagnostics")]
    {
        times.record_borrow(BufferId::TextCells, Access::ReadOnly, || BufferShape::vector(&text.content, IndexDomain::Cells));
        times.record_borrow(BufferId::FrameImage, Access::Writable, || BufferShape::vector(image.as_raw(), IndexDomain::Bytes));
        times.record_borrow(BufferId::GlyphMasks, Access::Writable, || BufferShape::unknown(IndexDomain::Glyphs));
        if let Some(counts) = times.inspect_memory(|| state.glyph_operations.take()).flatten() {
            times.describe("Text rasterization", || format!("glyph memory operations aggregate interleaved loop branches: {} clear calls, {} removed masks, {} new masks, {} reuse lookups; event categories are not per-glyph chronology", counts.cleared, counts.cleared_masks, counts.built, counts.reused));
            if counts.cleared > 0 { times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::GlyphMasks, Operation::Clear, None, None, Some(counts.cleared_masks), None)); }
            if counts.built > 0 { times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::GlyphMasks, Operation::Build, None, None, Some(counts.built), Some(counts.coverage_bytes))); }
            if counts.reused > 0 { times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::GlyphMasks, Operation::Reuse, None, None, Some(counts.reused), None)); }
        }
    }
}

/// Paint a prepared cell layout into the image. Backgrounds are applied before glyphs so wide characters
/// and combining marks are not erased by neighboring cells. Reset backgrounds leave sky pixels visible.
pub fn paint_text_buffer(state: &mut TextRasterizer, image: &mut RgbaImage, text: &ratatui::buffer::Buffer, cell: (u16, u16)) {
    set_text_cell_size(state, cell.0, cell.1);
    let (w, h) = (u32::from(state.cell.0), u32::from(state.cell.1));
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
            draw_text(state,
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
    state: &mut TextRasterizer,
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
        let ch = if state.font.lookup_glyph_index(ch) == 0 {
            '\u{fffd}'
        } else {
            ch
        };
        let glyph = if let Some(glyph) = state.glyphs.get(&ch) {
            #[cfg(feature = "memory-diagnostics")]
            if let Some(counts) = &mut state.glyph_operations { counts.reused += 1; }
            glyph
        } else {
            if state.glyphs.len() >= MAX_CACHED_GLYPHS {
                #[cfg(feature = "memory-diagnostics")]
                if let Some(counts) = &mut state.glyph_operations { counts.cleared += 1; counts.cleared_masks += state.glyphs.len(); }
                state.glyphs.clear();
            }
            let (metrics, coverage) = state.font.rasterize(ch, state.size);
            #[cfg(feature = "memory-diagnostics")]
            if let Some(counts) = &mut state.glyph_operations {
                counts.built += 1;
                counts.coverage_bytes += coverage.len();
            }
            state.glyphs.entry(ch).or_insert(Glyph { metrics, coverage })
        };
        let x0 = pen + glyph.metrics.xmin;
        let y0 = origin.1 + state.baseline.round() as i32 - glyph.metrics.ymin - glyph.metrics.height as i32;
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
        pen += i32::from(state.cell.0) * columns as i32;
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
        let mut renderer = create_text_rasterizer().unwrap();
        for ch in "αβγδεζθλμξπσφω°−×♈éü".chars() {
            assert_ne!(renderer.font.lookup_glyph_index(ch), 0, "missing {ch}");
        }
        let mut image = RgbaImage::from_pixel(100, 24, Rgba([0, 0, 0, 255]));
        draw_text(&mut renderer, &mut image, "α Vir 20°", (0, 0), (100, 24), [255, 255, 255]);
        assert!(image.pixels().any(|p| p[0] == 255));
        assert!(image.pixels().any(|p| p[0] > 0 && p[0] < 255));
        assert!(image.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn clipping_and_resize_do_not_leave_stale_glyph_masks() {
        let mut renderer = create_text_rasterizer().unwrap();
        let mut image = RgbaImage::from_pixel(40, 40, Rgba([0, 0, 0, 255]));
        draw_text(&mut renderer, &mut image, "Test", (-4, -2), (16, 12), [255, 0, 0]);
        for (x, y, p) in image.enumerate_pixels() {
            if x >= 12 || y >= 10 {
                assert_eq!(*p, Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!renderer.glyphs.is_empty());
        set_text_cell_size(&mut renderer, 20, 40);
        assert!(renderer.glyphs.is_empty());
        draw_text(&mut renderer, &mut image, "A", (0, 0), (40, 40), [255, 255, 255]);
        assert!(image.pixels().any(|p| p[1] > 0));
    }

    #[test]
    fn full_glyph_cache_keeps_hits_and_clears_only_for_a_missing_glyph() {
        let mut renderer = create_text_rasterizer().unwrap();
        let (metrics, coverage) = renderer.font.rasterize('A', renderer.size);
        renderer.glyphs.insert('A', Glyph { metrics, coverage: coverage.clone() });
        for index in 1..MAX_CACHED_GLYPHS {
            let ch = char::from_u32(0xe000 + index as u32).unwrap();
            renderer.glyphs.insert(ch, Glyph { metrics, coverage: coverage.clone() });
        }
        let mut image = RgbaImage::new(20, 20);
        draw_text(&mut renderer, &mut image, "A", (0, 0), (20, 20), [255; 3]);
        assert_eq!(renderer.glyphs.len(), MAX_CACHED_GLYPHS);
        image.fill(0);
        draw_text(&mut renderer, &mut image, "B", (0, 0), (20, 20), [255; 3]);
        assert_eq!(renderer.glyphs.len(), 1);
        assert!(renderer.glyphs.contains_key(&'B'));
        let mut expected = RgbaImage::new(20, 20);
        draw_text(&mut create_text_rasterizer().unwrap(), &mut expected, "B", (0, 0), (20, 20), [255; 3]);
        assert_eq!(image, expected);
    }

    #[test]
    fn missing_glyph_is_visible_and_deterministic() {
        let mut renderer = create_text_rasterizer().unwrap();
        let mut missing = RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 255]));
        let mut replacement = missing.clone();
        draw_text(&mut renderer, &mut missing, "\u{10ffff}", (0, 0), (20, 20), [255; 3]);
        draw_text(&mut renderer, &mut replacement, "�", (0, 0), (20, 20), [255; 3]);
        assert_eq!(missing, replacement);
        assert!(missing.pixels().any(|p| p[0] != 0));
    }
}

#[cfg(all(test, feature = "memory-diagnostics"))]
mod memory_tests {
    use super::*;
    use crate::timing::StepTimes;

    fn trace(active: bool) -> StepTimes {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_events(active);
        times
    }
    fn count(times: &StepTimes, wanted: Operation) -> usize {
        times.trace().unwrap().steps.iter().flat_map(|step| &step.memory_events).filter_map(|record| {
            match record.event {
                MemoryEvent::Operation { buffer: BufferId::GlyphMasks, operation, elements, .. } if operation == wanted => elements,
                _ => None,
            }
        }).sum()
    }

    #[test]
    fn mask_events_count_existing_branches_without_changing_text_pixels() {
        let mut state = create_text_rasterizer().unwrap();
        let mut text = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 3, 1));
        for x in 0..3 { text[(x, 0)].set_symbol("A"); }
        let mut image = RgbaImage::new(30, 20);
        let mut first = trace(true);
        paint_text_buffer_with_times(&mut state, &mut image, &text, (10, 20), &mut first);
        assert_eq!(count(&first, Operation::Build), 1);
        assert_eq!(count(&first, Operation::Reuse), 2);
        assert!(state.glyph_operations.is_none());
        let expected = image.clone();
        image.fill(0);
        let mut hit = trace(true);
        paint_text_buffer_with_times(&mut state, &mut image, &text, (10, 20), &mut hit);
        assert_eq!(image, expected);
        assert_eq!(count(&hit, Operation::Build), 0);
        assert_eq!(count(&hit, Operation::Reuse), 3);
        image.fill(0);
        let mut inactive = trace(false);
        paint_text_buffer_with_times(&mut state, &mut image, &text, (10, 20), &mut inactive);
        assert_eq!(image, expected);
        assert!(inactive.trace().unwrap().steps.iter().all(|step| step.memory_events.is_empty()));
        assert!(state.glyph_operations.is_none());
    }

    #[test]
    fn mask_resizing_reports_clear_then_actual_new_masks() {
        let mut state = create_text_rasterizer().unwrap();
        let mut text = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 1, 1));
        text[(0, 0)].set_symbol("A");
        let mut image = RgbaImage::new(40, 40);
        paint_text_buffer_with_times(&mut state, &mut image, &text, (10, 20), &mut trace(false));
        let mut resized = trace(true);
        paint_text_buffer_with_times(&mut state, &mut image, &text, (20, 40), &mut resized);
        assert_eq!(count(&resized, Operation::Build), 1);
        assert_eq!(count(&resized, Operation::Reuse), 0);
        assert!(resized.trace().unwrap().steps[0].memory_events.iter().any(|record| matches!(record.event, MemoryEvent::Operation { buffer: BufferId::GlyphMasks, operation: Operation::Clear, .. })));
    }
}
