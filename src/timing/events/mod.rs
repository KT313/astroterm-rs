//! Opt-in logical memory events. Values describe instrumented operations, never hardware memory traffic.
//! Step IDs belong to one StepTimes instance; no saved address or working-data reference enters the trace.
use super::StepTimes;
use crate::cache::{RefreshReason, Quality};
use std::mem::size_of;

/// Logical slots, stable across reallocation and catalog replacement. Names are formatted only when reporting.
/// This central vocabulary intentionally names domain buffers; additions coordinate here without importing domains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferId {
    CatalogStars, CatalogTrajectories, CatalogClassifications, CatalogGrid, CatalogNames, CatalogFigures, CatalogEndpoints,
    PlanetSamples, LunarSamples, OrientationSamples, ObserverGeometry, EmissionTimes,
    RegionSelection, BrightnessCandidates, ValidatedCandidates, WorkingStars, StellarSamples, StellarScratch, StellarRefreshRegions,
    MotionSamples, VisibilityFlags, CorrectionSelection, ObservedStars, ObservedBodies, BodySamples, RelativeBodies,
    MoonIllumination, ApparentDirections, HorizontalDirections, RefractedDirections,
    ProjectedCells, ProjectionCandidate, ProjectionBodyCandidate, ProjectionFigureCandidate, ProjectionHorizonCandidate, DrawOrderCandidate, DrawOrderScratch,
    DrawOrder, ProjectedBodies, ProjectedFigures, ProjectedHorizon, ProjectedView,
    PixelCandidate, CharacterCandidate, PixelScene, CharacterScene,
    CharacterFrame, PanelCanvas, PresenterScreen, PreviousFrame, FrameImage, HalfblockTransfer, RgbImage, MetadataFields,
    StepFields, TextCells, ComposedCells, GlyphMasks, Font, EncodedImage, UploadBytes, CompressedBytes, SerializedBytes,
    SerializationBlank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access { ReadOnly, Writable }

/// The meaning of indices, not a claim that all those entries were accessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexDomain { Catalog, Working, Observed, Visible, DrawOrder, Cells, Pixels, Bytes, Glyphs, ModelSamples, Objects, Regions, Unknown }

/// Direct element storage only; nested payload and allocator overhead are not inferred from sizeof(T).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferShape {
    pub len: Option<usize>,
    pub capacity: Option<usize>,
    pub element_bytes: Option<usize>,
    pub domain: IndexDomain,
    pub quality: Quality,
}
impl BufferShape {
    #[allow(clippy::ptr_arg)] // capacity is part of the observation
    pub fn vector<T>(values: &Vec<T>, domain: IndexDomain) -> Self {
        Self { len: Some(values.len()), capacity: Some(values.capacity()), element_bytes: Some(size_of::<T>()), domain, quality: Quality::ExactPayload }
    }
    pub fn slice<T>(values: &[T], domain: IndexDomain) -> Self {
        Self { len: Some(values.len()), capacity: None, element_bytes: Some(size_of::<T>()), domain, quality: Quality::ExactPayload }
    }
    pub fn unknown(domain: IndexDomain) -> Self {
        Self { len: None, capacity: None, element_bytes: None, domain, quality: Quality::Unknown }
    }
    pub fn logical_bytes(self) -> Option<usize> { self.len?.checked_mul(self.element_bytes?) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Copy, Build, Map, Append, Clear, Reserve, Reuse, Refresh(RefreshReason), RefreshUnknown, Compare,
    Store { value_changed: bool }, Move, Write, Output, Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryEvent {
    /// A view was granted. Neither access to every element nor a mutation is implied.
    Borrow { buffer: BufferId, access: Access, shape: BufferShape },
    /// An explicitly instrumented operation. None counts stay unknown; comparisons may exit early.
    Operation { buffer: BufferId, operation: Operation, before: Option<BufferShape>, after: Option<BufferShape>, elements: Option<usize>, logical_bytes: Option<usize>, quality: Quality },
}
impl MemoryEvent {
    pub fn borrow(buffer: BufferId, access: Access, shape: BufferShape) -> Self { Self::Borrow { buffer, access, shape } }
    pub fn operation(buffer: BufferId, operation: Operation, before: Option<BufferShape>, after: Option<BufferShape>, elements: Option<usize>, logical_bytes: Option<usize>) -> Self {
        let quality = if before.into_iter().chain(after).any(|shape| shape.quality == Quality::Unknown) {
            Quality::Unknown
        } else if before.into_iter().chain(after).any(|shape| shape.quality == Quality::LowerBound) {
            Quality::LowerBound
        } else if before.is_none() && after.is_none() && elements.is_none() && logical_bytes.is_none() {
            Quality::Unknown
        } else { Quality::ExactPayload };
        Self::Operation { buffer, operation, before, after, elements, logical_bytes, quality }
    }
    pub fn unknown_operation(buffer: BufferId, operation: Operation) -> Self {
        Self::Operation { buffer, operation, before: None, after: None, elements: None, logical_bytes: None, quality: Quality::Unknown }
    }
    #[cfg(feature = "memory-diagnostics")]
    fn counts(self) -> (Option<usize>, Option<usize>) {
        match self {
            Self::Borrow { shape, .. } => (shape.len, shape.logical_bytes()),
            Self::Operation { elements, logical_bytes, .. } => (elements, logical_bytes),
        }
    }
    #[cfg(feature = "memory-diagnostics")]
    fn same_kind(self, other: Self) -> bool {
        match (self, other) {
            (Self::Borrow { buffer: a, access: x, shape: s }, Self::Borrow { buffer: b, access: y, shape: t }) => a == b && x == y && s.domain == t.domain && s.quality == t.quality,
            (Self::Operation { buffer: a, operation: x, quality: s, .. }, Self::Operation { buffer: b, operation: y, quality: t, .. }) => a == b && x == y && s == t,
            _ => false,
        }
    }
}

#[cfg(feature = "memory-diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStepId(pub(crate) Target, pub(crate) u64);
#[cfg(feature = "memory-diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target { Trace(usize), Batch(usize) }

#[cfg(feature = "memory-diagnostics")]
#[path = "recording/enabled.rs"]
mod enabled;
#[cfg(feature = "memory-diagnostics")]
pub use enabled::RecordedMemoryEvent;
#[cfg(feature = "memory-diagnostics")]
pub(crate) use enabled::write_events;
#[cfg(not(feature = "memory-diagnostics"))]
#[path = "recording/disabled.rs"]
mod disabled;
mod hooks;

#[cfg(not(feature = "memory-diagnostics"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStepId;
