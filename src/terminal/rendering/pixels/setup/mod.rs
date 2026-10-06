//! Pixel renderer creation, capability negotiation, and terminal resizing.
use std::io;
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::{FontSize, picker::{Picker, ProtocolType}};
use crate::model::{TerminalSettings, ProjectionViewport as Viewport, RenderOptions};
use crate::state::PixelState;
use crate::terminal::{TerminalSession, open_terminal_session, fit_square_viewport};
use crate::terminal::transport::graphics::{clear_image, encode_image, kitty};
use super::{detection, layout::validate_frame_size};

pub fn open_pixel_renderer(
    options: RenderOptions,
    settings: TerminalSettings,
    forced: Option<ProtocolType>,
    text_scale: f64,
) -> io::Result<(TerminalSession, PixelState)> {
    if !text_scale.is_finite() || !(0.25..=4.0).contains(&text_scale) {
        return Err(io::Error::other("text scale must be finite and between 0.25 and 4"));
    }
    let mut session = open_terminal_session()?;
    session.configure_graphics(false, false); // cleanup also covers a failed startup or capability query
    let picker = Picker::halfblocks();
    let tmux = picker.tmux_detected();
    let (protocol, font, compression) = detection::detect_protocol(forced, tmux, session.output())?;
    let font = font.unwrap_or(picker.font_size());
    session.configure_graphics(protocol == ProtocolType::Kitty, tmux);
    let mut renderer = PixelState {
        scene_cache: Default::default(),
        cache_diagnostics: Default::default(),
        reuse_assets: true,
        protocol,
        compression,
        kitty_image_id: kitty::IMAGE_IDS[0],
        font,
        tmux,
        screen: Rect::default(),
        area: Rect::default(),
        viewport: Viewport { width: 1, height: 1 },
        options,
        settings,
        time_zone: None,
        text_scale,
        frame_image: None,
        sky_image: image::RgbaImage::new(0, 0),
        rgb: image::RgbImage::new(0, 0),
        fields: Vec::new(),
        text: ratatui::buffer::Buffer::empty(Rect::default()),
        composed: ratatui::buffer::Buffer::empty(Rect::default()),
        upload: String::new(),
        compressed: Vec::new(),
        encoded: None,
        serialization_blank: ratatui::buffer::Buffer::empty(Rect::default()),
        serialized: Vec::new(),
        raster_text: if protocol == ProtocolType::Halfblocks {
            None
        } else {
            Some(crate::scene::create_text_rasterizer().map_err(io::Error::other)?)
        },
    };
    fit_pixel_terminal(&mut renderer, &mut session)?;
    if protocol != ProtocolType::Kitty {
        let test = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        encode_image(DynamicImage::ImageRgba8(test), Rect::new(0, 0, 1, 1), protocol, tmux)?;
    }
    Ok((session, renderer))
}

pub fn pixel_viewport(state: &PixelState) -> Viewport {
    state.viewport
}

pub fn fit_pixel_terminal(state: &mut PixelState, session: &mut TerminalSession) -> io::Result<()> {
    state.scene_cache.invalidate();
    state.kitty_image_id = kitty::IMAGE_IDS[0];
    let (columns, rows) = crossterm::terminal::size()?;
    if columns == 0 || rows == 0 {
        return Err(io::Error::other("terminal has no drawable area"));
    }
    if let Ok(size) = crossterm::terminal::window_size()
        && size.width >= columns
        && size.height >= rows
    {
        state.font = FontSize::new(size.width / columns, size.height / rows);
    }
    state.font.width = state.font.width.max(1);
    state.font.height = state.font.height.max(1);
    state.screen = Rect::new(0, 0, columns, rows);
    let ratio = state
        .settings
        .aspect_ratio
        .unwrap_or(f64::from(state.font.height) / f64::from(state.font.width));
    let layout = fit_square_viewport(rows, columns, ratio);
    let (width, height) = (layout.width.max(1) as u16, layout.height.max(1) as u16);
    // floor the centering offset: encoders clear one row past the image before returning to its origin
    state.area = Rect::new((columns - width) / 2, (rows - height) / 2, width, height);
    state.viewport = Viewport {
        width: usize::from(state.area.width) * usize::from(state.font.width),
        height: usize::from(state.area.height) * usize::from(state.font.height),
    };
    if state.protocol != ProtocolType::Halfblocks {
        validate_frame_size(state.screen, state.font)?;
    } else if state
        .viewport
        .width
        .checked_mul(state.viewport.height)
        .is_none_or(|size| size > 16_777_216)
    {
        return Err(io::Error::other("terminal image exceeds 16 megapixels"));
    }
    clear_image(session.output(), state.protocol == ProtocolType::Kitty, state.tmux)
}

