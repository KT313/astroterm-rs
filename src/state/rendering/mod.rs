//! Application-owned character/pixel rendering buffers. The terminal writer and cleanup guard live outside state.
pub(crate) use crate::terminal::SharedMemoryImage;                                   // transport resource, owned by the Kitty transport since chunk 3
pub use crate::scene::TextRasterizer;                                                 // font state, owned by the text drawing code since chunk 3

use crate::astro::Observer;
use crate::canvas::Canvas;
use crate::model::{
    TerminalSettings, MetadataField, ObserverTimeZone, Frame, RenderOptions, ProjectionViewport as Viewport,
};
use ratatui::layout::Rect;
use ratatui_image::{FontSize, picker::ProtocolType};

/// Explicit terminal-dependent initialization: opening the session replaces Pending exactly once.
#[derive(Default)]
pub enum RenderingState {
    #[default]
    Pending,
    Chars(Box<CharacterState>),
    Pixels(Box<PixelState>),
}

pub use crate::terminal::transport::graphics::detection::CompressionSupport;        // detection result, owned by the transport since chunk 13

/// Character backend storage. Scene keys/results retain their existing cache semantics; frame and presenter
/// canvases use terminal cell coordinates. Resize resets frame/screen and invalidates the scene cache.
/// Metadata vectors are cleared/refilled in place; their strings are rebuilt. Notice strings own no catalog records.
pub struct CharacterState {
    pub(crate) scene_cache: crate::state::SceneCache,
    pub(crate) cache_diagnostics: [String; 2],
    pub(crate) frame: Frame,
    pub(crate) options: RenderOptions,
    pub(crate) settings: TerminalSettings,
    pub(crate) time_zone: Option<(Observer, ObserverTimeZone)>,
    pub(crate) startup_notice: Option<String>,
    pub(crate) presenter: Presenter,
    pub(crate) fields: Vec<MetadataField>,
    pub(crate) step_fields: Vec<MetadataField>,
}

/// Pixel storage in terminal cells and physical pixels. Completed text and Kitty RGB remain available for reuse.
/// Kitty retains RGBA composition work; other graphics protocols consume RGBA during encoding. Metadata vectors
/// retain capacity, but strings rebuild as needed. Resize invalidates results and resets the Kitty image ID.
pub struct PixelState {
    pub(crate) scene_cache: crate::state::SceneCache,
    pub(crate) cache_diagnostics: [String; 2],
    pub(crate) reuse_assets: bool,
    pub(crate) protocol: ProtocolType,
    pub(crate) compression: CompressionSupport,
    pub(crate) shared_memory: bool, // enabled only after a successful local transfer probe
    pub(crate) shared_upload: Option<SharedMemoryImage>, // at most one immutable RGB copy awaiting consumption
    pub(crate) kitty_image_id: u32,
    pub(crate) font: FontSize,
    pub(crate) tmux: bool,
    pub(crate) screen: Rect,
    pub(crate) area: Rect,
    pub(crate) viewport: Viewport,
    pub(crate) options: RenderOptions,
    pub(crate) settings: TerminalSettings,
    pub(crate) time_zone: Option<(Observer, ObserverTimeZone)>,
    pub(crate) raster_text: Option<TextRasterizer>,
    pub(crate) text_scale: f64,
    // Full-frame opaque sky/text bitmap; Kitty uploads it as RGBA without any conversion.
    pub(crate) frame_image: Option<image::RgbaImage>,
    pub(crate) frame_version: crate::model::RenderResultVersion,
    pub(crate) frame_key: Option<crate::model::PixelFrameKey>,
    pub(crate) encoding_key: Option<crate::model::KittyEncodingKey>,
    pub(crate) displayed_key: Option<crate::model::KittyDisplayKey>, // last successful submission; preserved on errors
    pub(crate) display_valid: bool, // resize, clearing or a failed write makes the terminal image unknown
    pub(crate) fields: Vec<MetadataField>,
    pub(crate) text: ratatui::buffer::Buffer,
    pub(crate) text_cache: crate::model::PixelTextCache, // bounded labels and dependencies for the retained text grid
    pub(crate) text_version: crate::model::RenderResultVersion, // valid only while the completed text grid is retained
    pub(crate) composed: ratatui::buffer::Buffer,
    // Refilled only after prior writes/flushes finish. Flat buffers retain capacity; no protocol/key recycling.
    pub(crate) upload: String,
    pub(crate) rgb: Vec<u8>, // alpha-stripped frame, filled when a Kitty upload is prepared (shared object or stream)
    pub(crate) compressed: Vec<u8>,
    pub(crate) compressor: Option<flate2::Compress>, // lazy zlib engine; reset between independent images, retain its working memory
    pub(crate) encoded: Option<ratatui_image::protocol::Protocol>,
    pub(crate) serialization_blank: ratatui::buffer::Buffer,
    pub(crate) serialized: Vec<u8>,
}

/// Screen composition and previous-frame snapshot in terminal cells. The diff algorithm borrows these from
/// terminal::present; successful presentation clones screen into previous, while resize discards previous.
#[derive(Debug)]
pub struct Presenter {
    pub(crate) screen: Canvas,
    pub(crate) previous: Option<Canvas>,
    pub(crate) sky_origin: (u16, u16),
}

impl Default for Presenter {
    fn default() -> Presenter {
        Presenter {
            screen: Canvas::new(0, 0),
            previous: None,
            sky_origin: (0, 0),
        }
    }
}


