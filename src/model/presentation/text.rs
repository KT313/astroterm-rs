//! Small owned descriptions and dependencies for retained pixel text; never references into a sibling owner.
use crate::model::{Cell, MetadataField, PlanetKind, ProductionRasterKey, ProjectionViewport, RenderResultVersion};
use crate::rows::row_columns;
use ratatui::layout::Rect;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PixelLabel {
    pub text: String,
    pub row: i32,
    pub col: i32,
    pub rgb: [u8; 3],
}
row_columns!(PixelLabel { text, row, col, rgb });

pub(crate) struct PixelLabelKey {
    pub projection: ProductionRasterKey,
    pub viewport: ProjectionViewport,
    pub area: Rect,
    pub threshold: f64,
    pub enabled: bool,
}

pub(crate) struct PixelTextKey {
    pub labels: u64,
    pub screen: Rect,
    pub area: Rect,
    pub viewport: ProjectionViewport,
    pub facing: bool,
    pub grid: bool,
    pub planets: Vec<(PlanetKind, Option<Cell>)>,
    pub moon: Option<Cell>,
    pub horizon: Vec<(Cell, &'static str)>,
    pub fields: Vec<MetadataField>,
    pub notice: Option<String>,
    pub accuracy_warning: bool,
    pub brightness_warning: bool,
}

/// Labels are bounded by DYNAMIC_NAME_COUNT; only region dependencies and displayed metadata can grow.
#[derive(Default)]
pub(crate) struct PixelTextCache {
    pub labels: Vec<PixelLabel>,
    pub labels_key: Option<PixelLabelKey>,
    pub labels_version: RenderResultVersion,
    pub text_key: Option<PixelTextKey>,
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(PixelLabel { text });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(PixelLabelKey { projection });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(PixelTextKey { planets, horizon, fields, notice });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(PixelTextCache { labels, labels_key, text_key });
