//! Feature-only enumeration of model payloads; no collector or history lives here.
use crate::cache::{report_fields, report_flat};
use crate::model::{
    Star, ObservedStar, Planet, Moon, PlanetKind, CorrectionStats, ObserverState,
    Anchor, MoonIllumination, SelectedStar, StellarWork, ValidityCounts, View, ViewCenter,
    ProjectionKind, ArcPart, CartesianCamera, ScreenPoint, Polar, ProjectionViewport, ProjectedPlanet,
    ProjectedMoon, DrawRecord, PixelStarKey, CharacterStarKey, RenderOptions, TerminalViewport,
    TerminalSettings, RendererKind, GraphicsProtocol, FrameTime, ModelFamily, StateRequest, CachePolicy,
    RefreshCounts, InterpolationLimits, Constellation, ObservedSky, BodySamples, CorrectionSelection, ProjectedArc,
    ProjectedConstellation, SceneKey, Glyph, Frame, StarKeys, Config,
};

report_flat!(crate::model::StarException, Star, ObservedStar, Planet, Moon, PlanetKind, CorrectionStats, ObserverState, Anchor, MoonIllumination,
    crate::model::SkyRegion, crate::astro::MoonPhase, SelectedStar, StellarWork, ValidityCounts, View, ViewCenter, ProjectionKind, ArcPart, CartesianCamera,
    ScreenPoint, Polar, ProjectionViewport, ProjectedPlanet, ProjectedMoon, DrawRecord,
    crate::model::StarColor, PixelStarKey, CharacterStarKey, RenderOptions, TerminalViewport, TerminalSettings,
    RendererKind, GraphicsProtocol, FrameTime, ModelFamily, StateRequest, CachePolicy, RefreshCounts,
    InterpolationLimits, crate::model::CellCap, crate::model::SelectionStats,
    crate::astro::Vector3, crate::astro::Matrix3, crate::astro::Observer, crate::astro::models::BodyState,
    crate::astro::models::stars::StellarClass, crate::astro::models::stars::StellarSample,
    crate::catalog::StarId, crate::canvas::Cell, crate::cache::Group, crate::cache::GroupPolicy);

report_fields!(Constellation { segments });
report_fields!(crate::model::SkyCatalog { stars, star_exceptions, grid, names, figures });
report_fields!(ObservedSky { catalog, stars, candidate_indices, planets, figure_override });
report_fields!(BodySamples { planets });
report_fields!(CorrectionSelection { indices });
report_fields!(ProjectedArc { points });
report_fields!(ProjectedConstellation { arcs });
report_fields!(SceneKey { stars, planets, constellations, horizon, labels });
report_fields!(Glyph { coverage });
report_fields!(Frame { sky, panel });

impl crate::cache::ReportBuffers for StarKeys {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::report_field;
        match self {
            Self::Pixels(stars) => report_field(sink, "pixels", stars),
            Self::Characters { glyphs, labels } => {
                report_field(sink, "glyphs", glyphs);
                report_field(sink, "labels", labels);
            }
        }
    }
}
impl crate::cache::ReportBuffers for Config {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::{report_field, Quality};
        report_field(sink, "cache", &self.cache);
        if let Some(crate::catalog::datasets::Dataset::Path(path)) = &self.dataset
            && sink.enter("dataset_path", std::mem::size_of_val(path)) {
            sink.payload(path.as_os_str().as_encoded_bytes().len(), path.capacity(), 1, Quality::ExactPayload, "platform path bytes");
            sink.leave();
        }
    }
}
