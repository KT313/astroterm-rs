//! Projection, drawing and metadata records shared between stages.
mod projection;
mod rendering;
pub use projection::{
    ArcPart, ScreenPoint, CartesianCamera, Polar,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars,
};
pub use rendering::{RenderOutcome, RenderProjection, TerminalViewport, Frame, Appearance};
pub(crate) use rendering::{select_color, terminal_color};
pub use crate::metadata::{ObserverTimeZone, MetadataField, StarCounts};                 // records owned by the metadata module since chunk 3
pub(crate) use projection::{
    DrawnStar, DrawnSpan, RegionalProjectionKey, RegionalOrderKey, RegionalDrawRecord, DrawRecord, StarKey, BodyKey as ProjectionBodyKey, ConstellationKey,
    HorizonGeometry,
};
pub(crate) use rendering::{KittyDisplayKey, RenderResultVersion, PixelFrameKey, KittyEncodingKey, ProductionRasterKey, SceneKey, CharacterStarKey, StarKeys};
pub(crate) use crate::scene::{StarPixel, StarOpacityTable, PixelStarKey, Glyph};             // pixel appearance and text types live with their drawing code since chunk 3

mod text;
pub(crate) use text::{PixelLabel, PixelLabelKey, PixelTextKey, PixelTextCache};
