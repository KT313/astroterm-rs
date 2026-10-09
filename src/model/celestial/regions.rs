//! Stable region identity and versions accompanying a borrowed observation result.
use crate::rows::row_columns;
use crate::astro::Vector3;

/// The row range addresses only this frame's observed array; retained caches must store catalog indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedRegion {
    pub region: usize,
    pub start: usize,
    pub end: usize,
    pub selection_generation: u64,
    pub motion_generation: u64,
    pub apparent_generation: u64,
}
row_columns!(ObservedRegion { region, start, end, selection_generation, motion_generation, apparent_generation });

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(ObservedRegion);

/// Each retained result addresses stable catalog rows within one region.
#[derive(Default, Debug)]
pub(crate) struct ObservationRegion {
    pub eligible: crate::cache::Cache<(u64, u64, f64), Vec<bool>>,
    pub corrections: crate::cache::Cache<(u64, u64), (Vec<crate::model::SelectedStar>, crate::model::CorrectionStats)>,
    pub apparent: crate::cache::Cache<(u64, u64, Vector3), Vec<Vector3>>,
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(ObservationRegion { eligible, corrections, apparent });
