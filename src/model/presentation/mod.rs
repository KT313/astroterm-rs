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
pub use rendering::{RenderOptions, TerminalViewport, Frame, Appearance};
pub use metadata::{ObserverTimeZone, MetadataField};
pub(crate) use projection::{
    DrawRecord, StarKey, BodyKey as ProjectionBodyKey, ConstellationKey,
    HorizonGeometry,
};
pub(crate) use rendering::{SceneKey, PixelStarKey, CharacterStarKey, StarKeys, Glyph};
