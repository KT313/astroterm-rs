//! Small owned display facts for benchmark reports; no image or text-buffer copies.
use crate::state::RenderingState;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CaptureGeometry {
    pub screen_cells: [u32; 2],
    pub font_pixels: Option<[u32; 2]>,
    pub image_pixels: Option<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CaptureDisplay {
    pub renderer: &'static str,
    pub graphics_protocol: &'static str,
    pub geometry: CaptureGeometry,
    pub transport: &'static str,
    pub compression_support: &'static str,
    pub tmux: bool,
}

pub fn capture_display(state: &RenderingState) -> std::io::Result<CaptureDisplay> {
    let display = match state {
        RenderingState::Chars(state) => CaptureDisplay { renderer: "chars", graphics_protocol: "characters", transport: "terminal_cells", compression_support: "not_applicable", tmux: false,
            geometry: CaptureGeometry { screen_cells: [state.presenter.screen.width() as u32, state.presenter.screen.height() as u32], font_pixels: None, image_pixels: None } },
        RenderingState::Pixels(state) => {
            use ratatui_image::picker::ProtocolType;
            let protocol = match state.protocol { ProtocolType::Kitty => "kitty", ProtocolType::Sixel => "sixel", ProtocolType::Iterm2 => "iterm2", ProtocolType::Halfblocks => "halfblocks" };
            let compression_support = match state.compression {
                crate::state::CompressionSupport::Supported => "supported", crate::state::CompressionSupport::Unsupported => "unsupported",
                crate::state::CompressionSupport::Unknown => "unknown",
            };
            CaptureDisplay { renderer: "pixels", graphics_protocol: protocol, compression_support, tmux: state.tmux,
                transport: if state.protocol == ProtocolType::Kitty && state.shared_memory { "shared_memory" } else { "streaming" },
                geometry: CaptureGeometry { screen_cells: [state.screen.width.into(), state.screen.height.into()],
                    font_pixels: Some([state.font.width.into(), state.font.height.into()]),
                    image_pixels: Some([u32::from(state.screen.width) * u32::from(state.font.width), u32::from(state.screen.height) * u32::from(state.font.height)]) } }
        },
        RenderingState::Pending => return Err(std::io::Error::other("capture requires an initialized renderer")),
    };
    let (columns, rows) = crossterm::terminal::size()?;
    if display.geometry.screen_cells != [u32::from(columns), u32::from(rows)] {
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "benchmark terminal resized during frame processing"));
    }
    Ok(display)
}
