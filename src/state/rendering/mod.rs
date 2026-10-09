//! Application-owned character/pixel rendering buffers. The terminal writer and cleanup guard live outside state.
mod shared_memory;
pub(crate) use shared_memory::SharedMemoryImage;

use crate::astro::Observer;
use crate::canvas::Canvas;
use crate::model::{
    TerminalSettings, MetadataField, ObserverTimeZone, Frame, Glyph, RenderOptions, ProjectionViewport as Viewport,
};
use fontdue::Font;
use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompressionSupport { Supported, Unsupported, #[default] Unknown }

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
    // Full-frame sky/text bitmap and the temporary opaque RGB conversion result.
    pub(crate) frame_image: Option<image::RgbaImage>,
    pub(crate) rgb: image::RgbImage,
    pub(crate) rgb_version: crate::model::RenderResultVersion,
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

/// Font handle and glyph masks at one cell size. Masks are cleared on resize, bypass, or the existing 512-glyph
/// bound. Coverage bytes are owned here; fontdue's inaccessible tables are reported as partial.
pub struct TextRasterizer {
    pub(crate) font: Font,
    pub(crate) glyphs: HashMap<char, Glyph>,
    pub(crate) cell: (u16, u16),
    pub(crate) size: f32,
    pub(crate) baseline: f32,
    #[cfg(feature = "memory-diagnostics")]
    pub(crate) glyph_operations: Option<GlyphOperations>,

}

#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for TextRasterizer {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        crate::cache::report_field(sink, "glyphs", &self.glyphs);
        sink.unknown("fontdue Font internals are opaque; glyph coverage Vecs are counted above");
    }
}


#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Presenter { screen, previous });

#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for RenderingState {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        match self {
            Self::Pending => {},
            Self::Chars(value) => crate::cache::report_field(sink, "characters", value),
            Self::Pixels(value) => crate::cache::report_field(sink, "pixels", value),
        }
    }
}

#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for CharacterState {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::report_field;
        report_field(sink, "scene_cache", &self.scene_cache);
        report_field(sink, "cache_diagnostics", &self.cache_diagnostics);
        report_field(sink, "frame", &self.frame);
        report_field(sink, "presenter", &self.presenter);
        report_field(sink, "startup_notice", &self.startup_notice);
        report_field(sink, "fields", &self.fields);
        report_field(sink, "step_fields", &self.step_fields);
        if self.time_zone.is_some() { sink.unknown("timezone rules and process-global boundary finder are opaque"); }
    }
}

#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for PixelState {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::report_field;
        report_field(sink, "scene_cache", &self.scene_cache);
        report_field(sink, "cache_diagnostics", &self.cache_diagnostics);
        report_field(sink, "raster_text", &self.raster_text);
        report_field(sink, "frame_image", &self.frame_image);
        report_field(sink, "rgb_pixels", self.rgb.as_raw());
        report_field(sink, "rgb_version", &self.rgb_version);
        report_field(sink, "frame_key", &self.frame_key);
        report_field(sink, "encoding_key", &self.encoding_key);
        report_field(sink, "displayed_key", &self.displayed_key);
        report_field(sink, "display_valid", &self.display_valid);
        report_field(sink, "fields", &self.fields);
        report_field(sink, "upload", &self.upload);
        report_field(sink, "compressed", &self.compressed);
        report_field(sink, "shared_upload", &self.shared_upload);
        if sink.enter("compressor", std::mem::size_of_val(&self.compressor)) {
            if self.compressor.is_some() { sink.unknown("retained zlib engine working memory; backend allocation sizes are opaque"); }
            sink.leave();
        }
        if self.encoded.is_some() { sink.unknown("ratatui-image encoded protocol internals are opaque"); }
        report_cell_buffer(sink, "serialization_blank", &self.serialization_blank);
        report_field(sink, "serialized", &self.serialized);
        report_field(sink, "text_cache", &self.text_cache);
        report_cell_buffer(sink, "text", &self.text);
        report_cell_buffer(sink, "composed", &self.composed);
        if self.time_zone.is_some() { sink.unknown("timezone rules and process-global boundary finder are opaque"); }
    }
}

/// Ratatui exposes cell storage, but not each CompactString's allocator capacity.
#[cfg(feature = "memory-diagnostics")]
fn report_cell_buffer(sink: &mut dyn crate::cache::BufferSink, name: &str, buffer: &ratatui::buffer::Buffer) {
    if sink.enter(name, std::mem::size_of_val(buffer)) {
        sink.payload(buffer.content.len(), buffer.content.capacity(), std::mem::size_of::<ratatui::buffer::Cell>(), crate::cache::Quality::ExactPayload, "ratatui cell vector; indirect symbol allocations excluded");
        sink.unknown("ratatui cell symbol storage is opaque; inline symbols are included in the cell vector");
        sink.leave();
    }
}

/// Temporary aggregate counters while a measured text pass runs; no per-glyph trace allocation or clock.
#[cfg(feature = "memory-diagnostics")]
#[derive(Default)]
pub(crate) struct GlyphOperations {
    pub reused: usize,
    pub built: usize,
    pub coverage_bytes: usize,
    pub cleared: usize,
    pub cleared_masks: usize,
}
