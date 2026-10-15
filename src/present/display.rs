//! The terminal a run draws on: the raw-mode session, the detected graphics protocol, the cell size and the
//! sky square. Opened once at startup and refitted on resize; step 7 writes through its `Terminal`.
//!
//! The frame covers the whole terminal in pixels (one image per frame, as the old pixel renderer sent it), and
//! the sky is the largest centred square that the terminal's cell aspect ratio allows. Text cells follow the
//! terminal's cells scaled by the text scale; in half-block mode the frame is downsampled whole, text included.
use crate::constants::MAX_IMAGE_PIXELS;
use crate::model::TerminalSettings;
use crate::projection::Frame;
use crate::terminal::transport::graphics::detection::{CompressionSupport, detect_protocol};
use crate::terminal::transport::graphics::clear_image;
use crate::terminal::{TerminalSession, fit_square_viewport, open_terminal_session};
use ratatui::layout::Rect;
use ratatui_image::{FontSize, picker::{Picker, ProtocolType}};
use std::io;

use super::Terminal;

/// The facts about the display that the metadata panel and the benchmark report show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub protocol: ProtocolType,
    pub tmux: bool,
    pub compression: CompressionSupport,
    pub shared_memory: bool,                // Kitty uploads go through shared memory (live: false after a fallback to streaming)
    pub screen: Rect,                       // the terminal in cells
    pub font: (u16, u16),                   // pixels per cell
    pub image: (usize, usize),              // the frame in pixels
    pub text_cell: (u16, u16),              // pixels per text cell on the frame
}

/// An open terminal with a graphics protocol. Dropping it restores the terminal.
pub struct Display {
    terminal: Terminal<TerminalSession>,
    info: DisplayInfo,
    aspect_ratio: Option<f64>,              // the configured cell aspect ratio, if any
    text_scale: f64,
}

impl Display {
    /// Switch the terminal to the alternate screen, detect the graphics protocol (or use the forced one) and
    /// size the frame to the terminal.
    pub fn open(forced: Option<ProtocolType>, settings: TerminalSettings, text_scale: f64) -> io::Result<Self> {
        let mut session = open_terminal_session()?;
        session.configure_graphics(false, false);                                          // cleanup also covers a failed capability query
        let picker = Picker::halfblocks();
        let tmux = picker.tmux_detected();
        let (protocol, font, compression, shared_memory) = detect_protocol(forced, tmux, session.output())?; // shared memory only after the terminal answered the probe
        let font = font.unwrap_or(picker.font_size());
        session.configure_graphics(protocol == ProtocolType::Kitty, tmux);
        let terminal = Terminal::new(session, protocol, tmux, Rect::default(), compression == CompressionSupport::Supported, shared_memory);
        let info = DisplayInfo { protocol, tmux, compression, shared_memory, screen: Rect::default(), font: (font.width, font.height), image: (0, 0), text_cell: (1, 1) };
        let mut display = Self { terminal, info, aspect_ratio: settings.aspect_ratio, text_scale };
        display.measure()?;
        Ok(display)
    }

    pub fn info(&self) -> DisplayInfo { DisplayInfo { shared_memory: self.terminal.shared_memory(), ..self.info } } // the transport can fall back mid-run
    pub fn terminal(&mut self) -> &mut Terminal<TerminalSession> { &mut self.terminal }

    /// A frame sized to the terminal as last measured.
    pub fn new_frame(&self) -> Frame {
        Frame::new(self.info.image.0, self.info.image.1, self.drawing_area_size(), self.info.text_cell)
    }

    /// Adopt the terminal's new size after a resize: the frame and the terminal are resized and the screen
    /// cleared, so the next frame is sent whole.
    pub fn fit(&mut self, frame: &mut Frame) -> io::Result<()> {
        self.measure()?;
        frame.resize(self.info.image.0, self.info.image.1, self.drawing_area_size());
        frame.set_text_cell(self.info.text_cell);
        Ok(())
    }

    /// Read the terminal size and cell size and lay the frame out; clear the screen.
    fn measure(&mut self) -> io::Result<()> {
        let (columns, rows) = crossterm::terminal::size()?;
        if columns == 0 || rows == 0 { return Err(io::Error::other("terminal has no drawable area")); }
        let mut font = FontSize::new(self.info.font.0, self.info.font.1);
        if let Ok(size) = crossterm::terminal::window_size() && size.width >= columns && size.height >= rows {
            font = FontSize::new(size.width / columns, size.height / rows);                   // the terminal reports its pixel size
        }
        let font = (font.width.max(1), font.height.max(1));
        let image = (usize::from(columns) * usize::from(font.0), usize::from(rows) * usize::from(font.1));
        if image.0.checked_mul(image.1).is_none_or(|pixels| pixels > MAX_IMAGE_PIXELS) {
            return Err(io::Error::other("terminal image exceeds 16 megapixels or has zero size"));
        }
        let scale = if self.info.protocol == ProtocolType::Halfblocks { 1.0 } else { self.text_scale };
        let cell = |pixels: u16| (f64::from(pixels) * scale).round().clamp(1.0, f64::from(u16::MAX)) as u16;
        self.info = DisplayInfo { screen: Rect::new(0, 0, columns, rows), font, image, text_cell: (cell(font.0), cell(font.1)), ..self.info };
        self.terminal.resize(self.info.screen);
        clear_image(self.terminal.output(), self.info.protocol == ProtocolType::Kitty, self.info.tmux)
    }

    /// Return the square's side length allowed by the cell aspect ratio, in pixels.
    fn drawing_area_size(&self) -> usize {
        let info = self.info;
        let ratio = self.aspect_ratio.unwrap_or(f64::from(info.font.1) / f64::from(info.font.0));
        let layout = fit_square_viewport(info.screen.height, info.screen.width, ratio);
        let (width, height) = (layout.width.max(1) * usize::from(info.font.0), layout.height.max(1) * usize::from(info.font.1));
        width.min(height).min(info.image.0).min(info.image.1)
    }
}
