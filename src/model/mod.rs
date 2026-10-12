//! Shared pipeline records and representation accessors. Import these types through `crate::model`.
//! Private folders group catalog, celestial, presentation and configuration data; processing lives above this layer.

mod catalog;
mod celestial;
mod presentation;
mod configuration;

pub use catalog::{
    SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, SkyRegion, SelectionStats, SkyGrid,
    hash_direction, STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage, Star, Constellation, StarColor,
    Stars, GroupTable, LabelColumn, Constellations, Bodies, Body, Satellite, SkyTables,
};
pub(crate) use catalog::{SelectedRegion, StellarFields, unsupported_star_data, CellCap, build_caps};
pub use celestial::{
    ObservedSkyView, ObservedPlanets, ObservedStars, RegionData, ApparentFrame, ObservedRegion, SelectedStar, ObservedStar, ObservedStarState, ObservedStarView, PlanetKind, Planet, Moon, create_planets, create_moon,
    CorrectionStats, ObservedSky, Sky, MoonIllumination, Anchor, ObserverState, FrameTime, ModelFamily, StateRequest,
    SimulationError, InterpolationLimits, PLANET_LIMITS, MOON_LIMITS, ORIENTATION_LIMITS, CachePolicy, RefreshCounts,
};
pub(crate) use celestial::{
    ObservationRegion, BodySamples, StellarWork, BodyDirections, ObserverKey,
    ObservationBodyKey, Sample, SolarRequestKey,
};
pub use presentation::{
    ArcPart, ScreenPoint, CartesianCamera, Polar,
    ProjectedStar, ProjectedPlanet, ProjectedMoon, ProjectedArc, ProjectedConstellation, ProjectedSky,
    ProjectionData, ProjectedStars, TerminalViewport, Frame, Appearance, ObserverTimeZone,
    MetadataField, StarCounts, RenderProjection, RenderOutcome,
};
pub(crate) use presentation::{select_color, terminal_color};
pub(crate) use presentation::{
    DrawnStar, DrawnSpan, RegionalProjectionKey, RegionalOrderKey, RegionalDrawRecord, DrawRecord, StarKey, ProjectionBodyKey, ConstellationKey, HorizonGeometry,
    PixelLabel, PixelLabelKey, PixelTextKey, PixelTextCache, KittyDisplayKey, RenderResultVersion, PixelFrameKey, KittyEncodingKey, ProductionRasterKey, SceneKey, StarPixel, StarOpacityTable, PixelStarKey, CharacterStarKey, StarKeys, Glyph,
};
pub use configuration::{FrameBenchmarkConfig, Config, SimulationSettings, TerminalSettings, RendererKind, GraphicsProtocol,
    ViewCenter, ProjectionKind, View, Cell, ProjectionViewport, RenderOptions};

#[cfg(test)]
pub(crate) use catalog::{interleave, direction};

// Fixed-size leaf records used by bounded table previews.
crate::rows::debug_preview!(Anchor, FrameTime, MoonIllumination, PlanetKind, ProjectionViewport, View, ProjectedMoon, CorrectionStats);

// Scalar selection totals are reported without reconstructing candidate index lists.
crate::rows::row_columns!(SelectionStats { cells, candidates, region_stars, brute_force });
