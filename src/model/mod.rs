//! Shared pipeline records and representation accessors. Import these types through `crate::model`.
//! Private folders group catalog, celestial, presentation and configuration data; processing lives above this layer.

mod catalog;
mod celestial;
mod presentation;
mod configuration;
#[cfg(feature = "memory-diagnostics")]
mod diagnostics;

pub use catalog::{
    SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, SkyRegion, SelectionStats, SkyGrid,
    hash_direction, STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage,
};
pub(crate) use catalog::{SelectedRegion, StellarFields, build_caps, unsupported_star_data};
pub use celestial::{
    ObservedSkyView, ObservedPlanets, ObservedStars, RegionData, ApparentFrame, ObservedRegion, Star, SelectedStar, ObservedStar, ObservedStarState, ObservedStarView, PlanetKind, Planet, Moon, Constellation, create_planets, create_moon,
    CorrectionStats, ObservedSky, Sky, MoonIllumination, Anchor, ObserverState, FrameTime, ModelFamily, StateRequest,
    SimulationError, InterpolationLimits, PLANET_LIMITS, MOON_LIMITS, ORIENTATION_LIMITS, CachePolicy, RefreshCounts,
};
pub(crate) use celestial::{
    ObservationRegion, BodySamples, StellarWork, BodyDirections, ObserverKey,
    ObservationBodyKey, Sample, SolarRequestKey,
};
pub use presentation::{
    ViewCenter, ProjectionKind, ArcPart, View, ScreenPoint, CartesianCamera, Polar, Cell, ProjectionViewport,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars, RenderOptions, TerminalViewport, Frame, Appearance, ObserverTimeZone,
    MetadataField, StarColor, RenderProjection, RenderOutcome,
};
pub(crate) use presentation::{
    RegionalStarIndex, RegionalProjectionKey, RegionalOrderKey, RegionalDrawRecord, DrawRecord, StarKey, ProjectionBodyKey, ConstellationKey, HorizonGeometry,
    PixelLabel, PixelLabelKey, PixelTextKey, PixelTextCache, KittyDisplayKey, RenderResultVersion, PixelFrameKey, KittyEncodingKey, ProductionRasterKey, RasterRegion, SceneKey, StarPixel, StarOpacityTable, PixelStarKey, CharacterStarKey, StarKeys, Glyph,
};
pub use configuration::{Config, SimulationSettings, TerminalSettings, RendererKind, GraphicsProtocol};

#[cfg(test)]
pub(crate) use catalog::{interleave, direction};

#[cfg(feature = "memory-diagnostics")]
pub(crate) use catalog::CellCap;

// Fixed-size leaf records used by bounded table previews.
crate::rows::debug_preview!(Anchor, FrameTime, MoonIllumination, PlanetKind, ProjectionViewport, View, ProjectedMoon, CorrectionStats);

// Scalar selection totals are reported without reconstructing candidate index lists.
crate::rows::row_columns!(SelectionStats { cells, candidates, brute_force });
