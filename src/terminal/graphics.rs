//! Image encoding and complete-frame serialization. Graphics protocols receive the fully composed raster;
//! half-block output merges native text into its cell buffer before serialization.
use std::io::{self, Write};

use image::DynamicImage;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::Rect,
    widgets::Widget,
};
use ratatui_image::{
    Image,
    picker::ProtocolType,
    protocol::{Protocol, halfblocks::Halfblocks, iterm2::Iterm2, kitty::Kitty, sixel::Sixel},
};

/// One image ID reused by this alternate-screen application; no unbounded per-frame image allocation.
pub const IMAGE_ID: u32 = 1_953_849_929;

pub fn encode_image(image: DynamicImage, area: Rect, protocol: ProtocolType, tmux: bool) -> io::Result<Protocol> {
    let size = area.as_size();
    let result = match protocol {
        ProtocolType::Halfblocks => Halfblocks::new(image, size).map(Protocol::Halfblocks),
        ProtocolType::Sixel => Sixel::new(image, size, tmux).map(Protocol::Sixel),
        ProtocolType::Kitty => Kitty::new(image, size, IMAGE_ID, tmux, false).map(Protocol::Kitty),
        ProtocolType::Iterm2 => Iterm2::new(image, size, tmux).map(Protocol::ITerm2),
    };
    result.map_err(io::Error::other)
}

pub fn compose_image(protocol: &Protocol, screen: Rect, area: Rect) -> Buffer {
    let mut image = Buffer::empty(screen);
    for y in screen.top()..screen.bottom() {
        for x in screen.left()..screen.right() {
            if !area.contains((x, y).into()) {
                image[(x, y)].set_bg(ratatui::style::Color::Rgb(3, 6, 14));
            }
        }
    }
    Image::new(protocol).render(area, &mut image);
    image
}

/// Half-blocks are ordinary cells: merge text before generating any terminal commands.
pub fn compose_halfblocks(protocol: &Protocol, screen: Rect, area: Rect, text: &Buffer) -> Buffer {
    assert!(matches!(protocol, Protocol::Halfblocks(_)));
    let mut frame = compose_image(protocol, screen, area);
    for y in screen.top()..screen.bottom() {
        for x in screen.left()..screen.right() {
            let Some(cell) = text.cell((x, y)) else {
                continue;
            };
            if cell.bg == ratatui::style::Color::Reset && cell.symbol() == " " {
                continue;
            }
            let background = frame[(x, y)].bg;
            frame[(x, y)] = cell.clone();
            if cell.bg == ratatui::style::Color::Reset {
                frame[(x, y)].bg = background; // transparent text keeps the underlying half-block background
            }
            let width = unicode_width::UnicodeWidthStr::width(cell.symbol()) as u16;
            for next in x + 1..x.saturating_add(width).min(screen.right()) {
                let background = frame[(next, y)].bg;
                frame[(next, y)] = cell.clone();
                frame[(next, y)].set_symbol(" ");
                if cell.bg == ratatui::style::Color::Reset {
                    frame[(next, y)].bg = background;
                }
            }
        }
    }
    frame
}

/// Serialize a single completed cell buffer, containing either the image protocol or merged half-block/text cells.
pub fn serialize_frame(image: &Buffer) -> io::Result<Vec<u8>> {
    crossterm::style::force_color_output(true); // RGB is required even for Kitty image IDs encoded in cell colors
    let mut frame = Vec::new();
    crossterm::queue!(frame, crossterm::terminal::BeginSynchronizedUpdate)?;
    let blank = Buffer::empty(image.area);
    let mut backend = CrosstermBackend::new(&mut frame);
    backend.draw(blank.diff(image).into_iter())?;
    crossterm::queue!(frame, crossterm::terminal::EndSynchronizedUpdate)?;
    Ok(frame)
}

/// Publish one already assembled frame. A short write is retried by write_all; a failed write/flush attempts to
/// release synchronization as well. Session restoration also releases it on quit or panic.
pub fn present_frame(out: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    if let Err(error) = out.write_all(frame).and_then(|_| out.flush()) {
        let _ = crossterm::execute!(out, crossterm::terminal::EndSynchronizedUpdate);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn clear_image(out: &mut impl Write, kitty: bool, tmux: bool) -> io::Result<()> {
    if kitty {
        let command = format!("\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\");
        if tmux {
            write!(out, "\x1bPtmux;{}\x1b\\", command.replace('\x1b', "\x1b\x1b"))?;
        } else {
            out.write_all(command.as_bytes())?;
        }
    }
    crossterm::execute!(
        out,
        crossterm::style::ResetColor,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        style::{Color, Style},
        widgets::Paragraph,
    };

    #[test]
    fn graphics_transmits_raster_text_and_halfblocks_merge_native_text_before_serialization() {
        let area = Rect::new(0, 0, 24, 2);
        let label = "RASTER_TEXT_SENTINEL";
        let mut text = Buffer::empty(area);
        Paragraph::new(label)
            .style(Style::default().fg(Color::White).bg(Color::Black))
            .render(area, &mut text);
        let mut raster = crate::scene::raster_text::TextRasterizer::new().unwrap();
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            let mut image = image::RgbaImage::from_pixel(240, 40, image::Rgba([20, 40, 80, 255]));
            let before = image.clone();
            if protocol != ProtocolType::Halfblocks {
                raster.paint_buffer(&mut image, &text, (10, 20));
                assert_ne!(image, before);
            }
            let encoded = encode_image(DynamicImage::ImageRgba8(image), area, protocol, false).unwrap();
            let buffer = if protocol == ProtocolType::Halfblocks {
                compose_halfblocks(&encoded, area, area, &text)
            } else {
                compose_image(&encoded, area, area)
            };
            let output = String::from_utf8(serialize_frame(&buffer).unwrap()).unwrap();
            assert!(output.starts_with("\x1b[?2026h") && output.ends_with("\x1b[?2026l"));
            assert_eq!(output.matches("\x1b[?2026h").count(), 1);
            assert_eq!(output.matches("\x1b[?2026l").count(), 1);
            assert_eq!(output.contains(label), protocol == ProtocolType::Halfblocks);
            let marker = match protocol {
                ProtocolType::Halfblocks => label,
                ProtocolType::Sixel => "\x1bP",
                ProtocolType::Kitty => "\x1b_G",
                ProtocolType::Iterm2 => "\x1b]1337;",
            };
            assert!(output.contains(marker), "{protocol:?}");
        }
    }

    #[test]
    fn halfblock_composition_replaces_wide_text_continuations() {
        let area = Rect::new(0, 0, 4, 1);
        let encoded = encode_image(
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 20, image::Rgba([20, 40, 80, 255]))),
            area,
            ProtocolType::Halfblocks,
            false,
        )
        .unwrap();
        let mut text = Buffer::empty(area);
        Paragraph::new("界x").render(area, &mut text);
        let frame = compose_halfblocks(&encoded, area, area, &text);
        assert_eq!(frame[(0, 0)].symbol(), "界");
        assert_eq!(frame[(1, 0)].symbol(), " ");
        assert_eq!(frame[(2, 0)].symbol(), "x");
        assert_eq!(frame[(0, 0)].bg, Color::Rgb(20, 40, 80));
        assert_eq!(frame[(1, 0)].bg, Color::Rgb(20, 40, 80));
    }

    #[test]
    fn failed_frame_write_attempts_to_release_synchronization() {
        struct InterruptedFrame {
            bytes: Vec<u8>,
            calls: usize,
        }
        impl Write for InterruptedFrame {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                if self.calls == 2 {
                    return Err(io::Error::other("transport failure"));
                }
                let count = if self.calls == 1 {
                    8.min(bytes.len())
                } else {
                    bytes.len()
                };
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = InterruptedFrame {
            bytes: Vec::new(),
            calls: 0,
        };
        let error = present_frame(&mut out, b"\x1b[?2026hunfinished frame").unwrap_err();
        assert_eq!(error.to_string(), "transport failure");
        assert!(out.bytes.ends_with(b"\x1b[?2026l"));
    }
}
