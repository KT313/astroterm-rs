//! Projection, drawing and metadata records shared between stages.
mod projection;
mod rendering;
mod metadata;
mod star_color;
pub use star_color::StarColor;
pub use projection::{
    ViewCenter, ProjectionKind, ArcPart, View, ScreenPoint, CartesianCamera, Polar, Cell, ProjectionViewport,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars,
};
pub use rendering::{RenderOutcome, RenderProjection, RenderOptions, TerminalViewport, Frame, Appearance};
pub use metadata::{ObserverTimeZone, MetadataField};
pub(crate) use projection::{
    DrawnStar, DrawnSpan, RegionalProjectionKey, RegionalOrderKey, RegionalDrawRecord, DrawRecord, StarKey, BodyKey as ProjectionBodyKey, ConstellationKey,
    HorizonGeometry,
};
pub(crate) use rendering::{KittyDisplayKey, RenderResultVersion, PixelFrameKey, KittyEncodingKey, ProductionRasterKey, SceneKey, StarPixel, StarOpacityTable, PixelStarKey, CharacterStarKey, StarKeys, Glyph};
#[cfg(feature = "memory-diagnostics")]
pub(crate) use rendering::RasterRegion; // only the diagnostics tables name the per-region raster dependency record

mod text;
pub(crate) use text::{PixelLabel, PixelLabelKey, PixelTextKey, PixelTextCache};
