//! Paint the screen-space metadata panel and notices after the sky objects and their labels.
use crate::metadata::MetadataField;
use crate::projection::Frame;
use crate::text::draw_text_over;

const NOTICE_RGB: [u8; 3] = [255, 255, 0];
const PANEL_RGB: [u8; 3] = [255, 255, 255];

/// Add the panel and notices to the existing overlay; never clear the sky graphics underneath.
pub fn draw_panel_and_notices(frame: &mut Frame, panel_lines: &[MetadataField], notices: &[&str]) {
    // Read the frame dimensions and the spacing between text lines.
    let (width, height) = (frame.width(), frame.height());
    if width == 0 || height == 0 { return; }
    let cell_height = i32::from(frame.text_cell().1);
    let (overlay, text) = frame.overlay_and_text();

    // Paint metadata and frame timings from the top left downwards.
    for (line, field) in panel_lines.iter().enumerate() {
        let content = format!("{}: {}", field.label, field.value);
        let position = (0, line as i32 * cell_height);
        draw_text_over(text, overlay, width, height, &content, position, PANEL_RGB);
    }

    // Keep notices readable over both the panel and sky by painting their black bands last.
    for (offset, notice) in notices.iter().enumerate() {
        let top = height as i32 - (offset as i32 + 1) * cell_height;
        if top < 0 { break; }
        fill_band(overlay, width, height, top, cell_height, [0, 0, 0]);
        draw_text_over(text, overlay, width, height, notice, (0, top), NOTICE_RGB);
    }

    // Check the completed overlay when pixel diagnostics are enabled.
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_overlay("draw_panel_and_notices output", frame.overlay());
}

/// An opaque band across the frame for the notices.
fn fill_band(overlay: &mut [u8], width: usize, height: usize, top: i32, rows: i32, rgb: [u8; 3]) {
    for y in top.max(0)..(top + rows).min(height as i32) {
        for pixel in overlay[y as usize * width * 4..(y as usize + 1) * width * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_preserves_existing_graphics_and_notices_cover_panel_text() {
        let fields = vec![MetadataField { label: "Time".into(), value: "12 ms".into() }; 4];
        let mut with_panel = Frame::square(80);
        let mut without_panel = Frame::square(80);
        for frame in [&mut with_panel, &mut without_panel] {
            for pixel in frame.overlay_and_text().0.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[10, 20, 30, 255]);
            }
        }

        // An empty panel and no notices must leave every existing pixel untouched.
        let original = with_panel.overlay().to_vec();
        draw_panel_and_notices(&mut with_panel, &[], &[]);
        assert_eq!(with_panel.overlay(), original);

        // Notices cover the panel's last line exactly as they cover a frame without a panel.
        draw_panel_and_notices(&mut with_panel, &fields, &["Warning"]);
        draw_panel_and_notices(&mut without_panel, &[], &["Warning"]);
        let notice_start = 60 * 80 * 4;
        assert_eq!(&with_panel.overlay()[notice_start..], &without_panel.overlay()[notice_start..]);
        assert_ne!(&with_panel.overlay()[..notice_start], &original[..notice_start]);
    }

    #[test]
    fn empty_frame_accepts_panel_and_notices() {
        let mut frame = Frame::square(0);
        let fields = [MetadataField { label: "Time".into(), value: "12 ms".into() }];
        draw_panel_and_notices(&mut frame, &fields, &["Warning"]);
        assert!(frame.overlay().is_empty());
    }
}
