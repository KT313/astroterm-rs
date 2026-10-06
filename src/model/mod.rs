//! Shared pipeline records and representation accessors. Import these types through `crate::model`.
//! Private folders group catalog, celestial, presentation and configuration data; processing lives above this layer.

mod catalog;
mod celestial;
mod presentation;
mod configuration;
#[cfg(feature = "memory-diagnostics")]
mod diagnostics;

pub use catalog::{
    SkyCatalog, GRID_DEPTH, CELL_COUNT, REFRACTION_MARGIN, ABERRATION_MARGIN, SkyRegion, SelectionStats, SkyGrid,
    hash_direction, QUANTIZATION_MARGIN, StarStorage,
};
pub(crate) use catalog::{SelectedRegion, build_caps};
pub use celestial::{
    Star, ObservedStar, ObservedStarView, PlanetKind, Planet, Moon, Constellation, create_planets, create_moon,
    CorrectionStats, ObservedSky, Sky, MoonIllumination, Anchor, ObserverState, FrameTime, ModelFamily, StateRequest,
    SimulationError, InterpolationLimits, PLANET_LIMITS, MOON_LIMITS, ORIENTATION_LIMITS, CachePolicy, RefreshCounts,
};
pub(crate) use celestial::{
    BodySamples, SelectedStar, CorrectionSelection, StellarWork, ValidityCounts, Directions, ObserverKey,
    ObservationBodyKey, Sample,
};
pub use presentation::{
    ViewCenter, ProjectionKind, ArcPart, View, ScreenPoint, CartesianCamera, Polar, Cell, ProjectionViewport,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars, RenderOptions, TerminalViewport, Frame, Appearance, ObserverTimeZone,
    MetadataField,
};
pub(crate) use presentation::{
    DEFAULT_FOV_DEGREES, MIN_FOV_DEGREES, DrawRecord, StarKey, ProjectionBodyKey, ConstellationKey, HorizonGeometry,
    SceneKey, PixelStarKey, CharacterStarKey, StarKeys, StarDisplay, PreparedScene, Glyph,
};
pub use configuration::{Config, SimulationSettings, TerminalSettings, RendererKind, GraphicsProtocol};

#[cfg(test)]
pub(crate) use catalog::{interleave, direction};

#[cfg(feature = "memory-diagnostics")]
pub(crate) use catalog::CellCap;
