//! Shared pipeline records and representation accessors. Import these types through `crate::model`.
//! Private folders group catalog, celestial, presentation and configuration data; processing lives above this layer.

mod catalog;
mod celestial;
mod presentation;
mod configuration;
#[cfg(feature = "memory-diagnostics")]
mod diagnostics;

pub use catalog::{
    SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, GRID_DEPTH, CELL_COUNT, CONSTELLATION_REGION, SIMULATION_REGION_COUNT, STELLAR_DRIFT_MARGIN, REFRACTION_MARGIN, ABERRATION_MARGIN, SkyRegion, SelectionStats, SkyGrid,
    hash_direction, QUANTIZATION_MARGIN, STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage,
};
pub(crate) use catalog::{SelectedRegion, StellarFields, build_caps, unsupported_star_data};
pub use celestial::{
    Star, SelectedStar, ObservedStar, ObservedStarView, PlanetKind, Planet, Moon, Constellation, create_planets, create_moon,
    CorrectionStats, ObservedSky, Sky, MoonIllumination, Anchor, ObserverState, FrameTime, ModelFamily, StateRequest,
    SimulationError, InterpolationLimits, PLANET_LIMITS, MOON_LIMITS, ORIENTATION_LIMITS, CachePolicy, RefreshCounts,
};
pub(crate) use celestial::{
    BodySamples, CorrectionSelection, StellarWork, Directions, ObserverKey,
    ObservationBodyKey, Sample,
};
pub use presentation::{
    ViewCenter, ProjectionKind, ArcPart, View, ScreenPoint, CartesianCamera, Polar, Cell, ProjectionViewport,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars, RenderOptions, TerminalViewport, Frame, Appearance, ObserverTimeZone,
    MetadataField, StarColor,
};
pub(crate) use presentation::{
    DEFAULT_FOV_DEGREES, MIN_FOV_DEGREES, DrawRecord, StarKey, ProjectionBodyKey, ConstellationKey, HorizonGeometry,
    SceneKey, PixelStarKey, CharacterStarKey, StarKeys, Glyph,
};
pub use configuration::{Config, SimulationSettings, TerminalSettings, RendererKind, GraphicsProtocol};

#[cfg(test)]
pub(crate) use catalog::{interleave, direction};

#[cfg(feature = "memory-diagnostics")]
pub(crate) use catalog::CellCap;

// Fixed-size leaf records used by bounded table previews.
crate::rows::debug_preview!(Anchor, FrameTime, MoonIllumination, PlanetKind, ProjectionViewport, View, SelectionStats, ProjectedMoon, CorrectionStats);
