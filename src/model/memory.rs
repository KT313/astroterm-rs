//! Feature-only enumeration of model payloads; no collector or history lives here.
use crate::cache::buffers::{report_fields, report_flat};
use super::{objects::*, observation::*, projection::*, rendering::*, simulation::*, config::*};

report_flat!(Star, ObservedStar, Planet, Moon, PlanetKind, CorrectionStats, ObserverState, Anchor, MoonIllumination,
    crate::model::SkyRegion, crate::astro::MoonPhase, SelectedStar, StellarWork, ValidityCounts, View, ViewCenter, ProjectionKind, ArcPart, CartesianCamera,
    ScreenPoint, Polar, ProjectionViewport, ProjectedPlanet, ProjectedMoon, DrawRecord,
    PixelStarKey, CharacterStarKey, StarDisplay, RenderOptions, TerminalViewport, TerminalSettings,
    RendererKind, GraphicsProtocol, FrameTime, ModelFamily, StateRequest, CachePolicy, RefreshCounts,
    InterpolationLimits, crate::model::grid::CellCap, crate::model::grid::SelectionStats,
    crate::astro::Vector3, crate::astro::Matrix3, crate::astro::Observer, crate::astro::models::BodyState,
    crate::astro::models::stars::StellarClass, crate::astro::models::stars::StellarSample,
    crate::catalog::StarId, crate::canvas::Cell, crate::cache::Group, crate::cache::GroupPolicy);

report_fields!(Constellation { segments });
report_fields!(crate::model::SkyCatalog { stars, grid, endpoint_indices, always_checked, names, constellations });
report_fields!(ObservedSky { catalog, stars, candidate_indices, planets, names, constellations });
report_fields!(BodySamples { planets });
report_fields!(CorrectionSelection { indices });
report_fields!(ProjectedArc { points });
report_fields!(ProjectedConstellation { arcs });
report_fields!(SceneKey { stars, planets, constellations, horizon, labels });
report_fields!(PreparedScene { catalog, stars });
report_fields!(Glyph { coverage });
report_fields!(Frame { sky, panel });

impl crate::cache::buffers::ReportBuffers for StarKeys {
    fn report_buffers(&self, sink: &mut dyn crate::cache::buffers::BufferSink) {
        use crate::cache::buffers::report_field;
        match self {
            Self::Pixels(stars) => report_field(sink, "pixels", stars),
            Self::Characters { glyphs, labels } => {
                report_field(sink, "glyphs", glyphs);
                report_field(sink, "labels", labels);
            }
        }
    }
}
impl crate::cache::buffers::ReportBuffers for Config {
    fn report_buffers(&self, sink: &mut dyn crate::cache::buffers::BufferSink) {
        use crate::cache::buffers::{report_field, Quality};
        report_field(sink, "cache", &self.cache);
        if let Some(crate::catalog::datasets::Dataset::Path(path)) = &self.dataset
            && sink.enter("dataset_path", std::mem::size_of_val(path)) {
            sink.payload(path.as_os_str().as_encoded_bytes().len(), path.capacity(), 1, Quality::ExactPayload, "platform path bytes");
            sink.leave();
        }
    }
}
