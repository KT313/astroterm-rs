//! Pixel canvas limits and independent text sizing.
use std::io;
use ratatui::layout::Rect;
use ratatui_image::FontSize;

/// Give raster text its own grid, so glyph size, line spacing and panel extent scale together. Sky pixels and
/// terminal image protocol dimensions continue to use the physical cell size.
pub(super) fn compute_text_layout(screen: Rect, area: Rect, font: FontSize, scale: f64) -> (Rect, Rect, (u16, u16)) {
    let width = (f64::from(font.width) * scale).round().clamp(1.0, f64::from(u16::MAX)) as u16;
    let height = (f64::from(font.height) * scale).round().clamp(1.0, f64::from(u16::MAX)) as u16;
    let scale_rect = |rect: Rect| {
        let left = u32::from(rect.x) * u32::from(font.width) / u32::from(width);
        let top = u32::from(rect.y) * u32::from(font.height) / u32::from(height);
        let right = (u32::from(rect.right()) * u32::from(font.width)).div_ceil(u32::from(width));
        let bottom = (u32::from(rect.bottom()) * u32::from(font.height)).div_ceil(u32::from(height));
        Rect::new(
            left.min(65535) as u16,
            top.min(65535) as u16,
            (right - left).min(65535) as u16,
            (bottom - top).min(65535) as u16,
        )
    };
    let text_screen = scale_rect(screen);
    (text_screen, scale_rect(area).intersection(text_screen), (width, height))
}

/// Include margins and metadata in the allocation budget, not only the square sky viewport.
pub(super) fn validate_frame_size(screen: Rect, font: FontSize) -> io::Result<(u32, u32)> {
    let (width, height) = (
        u32::from(screen.width) * u32::from(font.width),
        u32::from(screen.height) * u32::from(font.height),
    );
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > crate::constants::MAX_IMAGE_PIXELS as u64 {
        return Err(io::Error::other(
            "terminal image exceeds 16 megapixels or has zero size",
        ));
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_changes_spacing_and_capacity_without_changing_sky_pixels() {
        let screen = Rect::new(0, 0, 100, 40);
        let sky = Rect::new(10, 0, 80, 40);
        let font = FontSize::new(10, 20);
        assert_eq!(compute_text_layout(screen, sky, font, 1.0), (screen, sky, (10, 20)));
        let (small, _, cell) = compute_text_layout(screen, sky, font, 0.85);
        assert_eq!(cell, (9, 17));
        assert!(small.width > screen.width && small.height > screen.height);
        let (large, _, cell) = compute_text_layout(screen, sky, font, 2.0);
        assert_eq!(cell, (20, 40));
        assert_eq!(large, Rect::new(0, 0, 50, 20));
    }

    #[test]
    fn tiny_cells_and_partial_text_rows_stay_nonzero_and_clipped() {
        let (screen, area, cell) =
            compute_text_layout(Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1), FontSize::new(1, 1), 0.25);
        assert_eq!(cell, (1, 1));
        assert_eq!(screen, area);
        assert_eq!(screen.width, 1);
    }
}
