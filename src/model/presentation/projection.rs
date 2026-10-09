//! Camera settings and projected records; algorithms live in projection.
use crate::constants::DEFAULT_FOV_DEGREES;
use crate::rows::row_columns;
use crate::astro::{Horizontal, Vector3};
use std::f64::consts::{PI, FRAC_PI_2};
use crate::model::{ObservedStarView, PlanetKind};
use crate::catalog::{StarNames, StarId};
use crate::cache::Cache;


/// Where the center of the view points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewCenter {
    /// Lying on your back looking straight up, with North at the top.
    Zenith,
    /// Looking towards `azimuth`, tilted up by `tilt` from the horizon (radians).
    Facing { azimuth: f64, tilt: f64 },
}

/// Azimuthal projection used to flatten the sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionKind {
    Stereographic,
    Equidistant,
}

impl ProjectionKind {
    /// Largest field of view in degrees. The stereographic projection sends the point directly behind the center to
    /// infinity, so it can't show the full 360°.
    pub fn max_fov_degrees(self) -> f64 {
        match self {
            ProjectionKind::Stereographic => 359.0,
            ProjectionKind::Equidistant => 360.0,
        }
    }
}

/// A visible part of a great-circle arc: angles (radians) from the arc's start, and whether it reaches the arc's ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcPart {
    pub start: f64,
    pub end: f64,
    pub includes_start: bool,
    pub includes_end: bool,
}

/// A view of the sky. Projected points with radius > 1 are out of view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: ViewCenter,
    pub projection: ProjectionKind,
    /// Angular diameter of the rendered circle in degrees.
    pub fov_degrees: f64,
}

impl Default for View {
    fn default() -> View {
        View {
            center: ViewCenter::Zenith,
            projection: ProjectionKind::Stereographic,
            fov_degrees: DEFAULT_FOV_DEGREES,
        }
    }
}

impl View {
    /// Whether the view faces a direction (with the horizon across it) rather than the zenith.
    pub fn is_facing(&self) -> bool {
        matches!(self.center, ViewCenter::Facing { .. })
    }

    /// Tilt of the view above the horizon in radians, π/2 for the zenith view.
    pub fn tilt(&self) -> f64 {
        match self.center {
            ViewCenter::Zenith => FRAC_PI_2,
            ViewCenter::Facing { tilt, .. } => tilt,
        }
    }

    /// The direction the center of the view points at.
    pub fn center_direction(&self) -> Horizontal {
        match self.center {
            ViewCenter::Zenith => Horizontal {
                azimuth: PI,
                altitude: FRAC_PI_2,
            },
            ViewCenter::Facing { azimuth, tilt } => Horizontal {
                azimuth,
                altitude: tilt,
            },
        }
    }

}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}
impl ScreenPoint {
    pub fn radius(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn is_visible(self) -> bool {
        self.x * self.x + self.y * self.y <= 1.0 + 8.0 * f64::EPSILON
    }
    pub fn clamp_to_edge(self) -> Self {
        let radius = self.radius();
        if radius > 1.0 {
            Self {
                x: self.x / radius,
                y: self.y / radius,
            }
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CartesianCamera {
    pub(crate) right: Vector3,
    pub(crate) up: Vector3,
    pub(crate) forward: Vector3,
    pub(crate) scale: f64,
    pub(crate) kind: ProjectionKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Polar {
    pub radius: f64,
    pub theta: f64,
}

impl Polar {
    /// Cartesian (x right, y up) coordinates.
    pub fn to_cartesian(self) -> (f64, f64) {
        (self.radius * self.theta.cos(), self.radius * self.theta.sin())
    }

    /// Polar coordinates of a cartesian point.
    pub fn from_cartesian(x: f64, y: f64) -> Polar {
        Polar {
            radius: x.hypot(y),
            theta: y.atan2(x),
        }
    }
}

pub type Cell = (i32, i32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionViewport {
    pub height: usize,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedStar<'a> {
    pub star: ObservedStarView<'a>,
    pub cell: Option<Cell>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedPlanet {
    pub kind: PlanetKind,
    pub cell: Option<Cell>,
}
row_columns!(ProjectedPlanet { kind, cell });
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedMoon {
    pub illumination: crate::model::MoonIllumination,
    pub phase: crate::astro::MoonPhase,
    pub cell: Option<Cell>,
    /// Unit direction toward the Sun: x right, y up. None at a degenerate projection.
    pub light_direction: Option<ScreenPoint>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedArc {
    pub start: Cell,
    pub end: Cell,
    /// Sampled projected great-circle vertices, in this viewport (cells or pixels).
    pub points: Vec<Cell>,
    pub includes_start: bool,
    pub includes_end: bool,
}
row_columns!(ProjectedArc { start, end, points, includes_start, includes_end });
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedConstellation {
    pub maximum_magnitude: f64,
    pub arcs: Vec<ProjectedArc>,
}
row_columns!(ProjectedConstellation { maximum_magnitude, arcs });
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedSky<'a> {
    pub outside_accuracy_range: bool,
    pub selection: crate::model::SelectionStats,
    pub evaluated_stars: usize,
    pub correction_stats: crate::model::CorrectionStats,
    pub catalog_singular_count: usize,
    pub runtime_singular_count: usize,
    pub stars: ProjectedStars<'a>,
    pub planets: &'a [ProjectedPlanet],
    pub moon: &'a ProjectedMoon,
    pub constellations: &'a [ProjectedConstellation],
    pub names: &'a StarNames,
    pub facing: bool,
    /// Angular view width, carried through for zoom-dependent pixel-star brightness.
    pub fov_degrees: f64,
    pub viewport: ProjectionViewport,
    pub horizon: &'a [[Cell; 2]],
    pub horizon_labels: &'a [(Cell, &'static str)],
}

impl ProjectedSky<'_> {
    pub fn magnitude_clipping(&self) -> crate::catalog::MagnitudeClipping {
        self.stars.observed.catalog().magnitude_clipping()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionData {
    pub outside_accuracy_range: bool,
    pub selection: crate::model::SelectionStats,
    pub evaluated_stars: usize,
    pub correction_stats: crate::model::CorrectionStats,
    pub catalog_singular_count: usize,
    pub runtime_singular_count: usize,
    pub order: Vec<usize>,
    pub stars: Vec<(usize, Cell)>,
    pub planets: Vec<ProjectedPlanet>,
    pub moon: ProjectedMoon,
    pub constellations: Vec<ProjectedConstellation>,
    pub facing: bool,
    /// Angular view width, carried through for zoom-dependent pixel-star brightness.
    pub fov_degrees: f64,
    pub viewport: ProjectionViewport,
    pub horizon: Vec<[Cell; 2]>,
    pub horizon_labels: Vec<(Cell, &'static str)>,
}

impl ProjectionData {
    /// Borrow geometry and resolve observed indices only when a consumer asks for a star.
    pub fn view<'a>(&'a self, observed: impl Into<crate::model::ObservedSkyView<'a>>) -> ProjectedSky<'a> {
        let observed = observed.into();
        let summary = observed.summary();
        ProjectedSky {
            outside_accuracy_range: self.outside_accuracy_range,
            selection: self.selection,
            evaluated_stars: self.evaluated_stars,
            correction_stats: self.correction_stats,
            catalog_singular_count: self.catalog_singular_count,
            runtime_singular_count: self.runtime_singular_count,
            facing: self.facing,
            fov_degrees: self.fov_degrees,
            viewport: self.viewport,
            stars: ProjectedStars::with_order(observed.stars, &self.stars, &self.order),
            planets: &self.planets, moon: &self.moon, constellations: &self.constellations,
            names: &summary.catalog.names, horizon: &self.horizon, horizon_labels: &self.horizon_labels,
        }
    }
}

/// Read-only drawing order: either a globally sorted permutation (headless and editable callers) or the regions'
/// own drawn records, each region already in its paint order. Regional spans identify the independently
/// brightness-sorted runs for bounded global label selection.
#[derive(Clone, Copy, Debug)]
pub struct ProjectedStars<'a> {
    observed: crate::model::ObservedStars<'a>,
    source: DrawSource<'a>,
}
#[derive(Clone, Copy, Debug)]
enum DrawSource<'a> {
    Ordered { cells: &'a [(usize, Cell)], order: &'a [usize] },
    Regional { regions: &'a [Cache<RegionalProjectionKey, Vec<DrawnStar>>], spans: &'a [DrawnSpan] },
}
impl<'a> ProjectedStars<'a> {
    pub fn new(observed: &'a crate::model::ObservedSky, cells: &'a [(usize, Cell)], order: &'a [usize]) -> Self {
        Self { observed: crate::model::ObservedStars::owned(&observed.stars, &observed.catalog.stars), source: DrawSource::Ordered { cells, order } }
    }
    pub(crate) fn with_order(observed: crate::model::ObservedStars<'a>, cells: &'a [(usize, Cell)], order: &'a [usize]) -> Self {
        Self { observed, source: DrawSource::Ordered { cells, order } }
    }
    /// `spans` lists the painted regions in paint order; each one's drawn records are `regions[span.region]`.
    pub(crate) fn from_regions(observed: crate::model::ObservedStars<'a>, regions: &'a [Cache<RegionalProjectionKey, Vec<DrawnStar>>], spans: &'a [DrawnSpan]) -> Self {
        Self { observed, source: DrawSource::Regional { regions, spans } }
    }
    pub fn len(&self) -> usize {
        match self.source { DrawSource::Ordered { order, .. } => order.len(), DrawSource::Regional { spans, .. } => spans.last().map_or(0, |span| span.end) }
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn get(&self, index: usize) -> ProjectedStar<'a> {
        match self.source {
            DrawSource::Ordered { cells, order } => {
                let (row, cell) = cells[order[index]];
                ProjectedStar { star: self.observed.get(row), cell: Some(cell) }
            }
            DrawSource::Regional { regions, spans } => {
                let span = &spans[spans.partition_point(|span| span.end <= index)];
                self.resolve(span, regions[span.region].value()[index - span.start])
            }
        }
    }
    /// The full star view behind one drawn record: its row is found among its region's rows (one binary search).
    fn resolve(&self, span: &DrawnSpan, star: DrawnStar) -> ProjectedStar<'a> {
        let (columns, base) = self.observed.slot_columns(span.slot);
        let row = columns.find_row(star.source_index as usize).expect("drawn star belongs to its region's rows");
        ProjectedStar { star: self.observed.get_regional(span.slot, base + row), cell: Some(star.cell) }
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = ProjectedStar<'a>> + ExactSizeIterator + '_ {
        let spans = match self.source { DrawSource::Regional { spans, .. } => spans.len(), DrawSource::Ordered { .. } => 0 };
        ProjectedStarIter { stars: *self, front: 0, back: self.len(), front_span: 0, back_span: spans.saturating_sub(1) }
    }
    /// Every drawn star in paint order, read straight from the regional records (a plain slice walk per region);
    /// the ordered path builds each record from its star view.
    pub(crate) fn drawn(&self) -> DrawnStars<'a> {
        match self.source {
            DrawSource::Ordered { .. } => DrawnStars::Ordered { stars: *self, index: 0 },
            DrawSource::Regional { regions, spans } => DrawnStars::Regional { regions, spans: spans.iter(), current: Default::default() },
        }
    }
    /// Each range is dimmest-to-brightest: one region's records, or the whole ordered list.
    pub(crate) fn sorted_ranges(&self) -> impl Iterator<Item = SortedRange> + '_ {
        let (spans, whole) = match self.source {
            DrawSource::Regional { spans, .. } => (spans, None),
            DrawSource::Ordered { .. } => (&[][..], Some(SortedRange { indices: 0..self.len(), span: 0 })),
        };
        spans.iter().enumerate().map(|(span, entry)| SortedRange { indices: entry.start..entry.end, span }).chain(whole)
    }
    pub(crate) fn catalog(&self) -> &'a crate::model::StarStorage { self.observed.catalog() }
    /// Visit the drawn stars of one sorted range in paint order, or from the bright end when `brightest_first`.
    /// On the regional path the range is a slice of its region's records, so each star is one sequential read.
    /// `visit` gets the paint index and returns false to stop the range early.
    pub(crate) fn visit_range(&self, range: &SortedRange, brightest_first: bool, mut visit: impl FnMut(usize, DrawnStar) -> bool) {
        match self.source {
            DrawSource::Regional { regions, spans } => {
                let span = &spans[range.span];
                debug_assert!(span.start <= range.indices.start && range.indices.end <= span.end, "one sorted range lies within one region");
                let entries = &regions[span.region].value()[range.indices.start - span.start..range.indices.end - span.start];
                let indexed = range.indices.clone().zip(entries);
                if brightest_first { for (index, &star) in indexed.rev() { if !visit(index, star) { return; } } }
                else { for (index, &star) in indexed { if !visit(index, star) { return; } } }
            }
            DrawSource::Ordered { .. } => {
                let mut step = |index: usize| visit(index, drawn_record(&self.get(index)));
                if brightest_first { for index in range.indices.clone().rev() { if !step(index) { return; } } }
                else { for index in range.indices.clone() { if !step(index) { return; } } }
            }
        }
    }
}
impl PartialEq for ProjectedStars<'_> {
    fn eq(&self, other: &Self) -> bool { self.iter().eq(other.iter()) }
}

/// The ordered path has no stored records; one is built from the star view when a drawn record is asked for.
fn drawn_record(star: &ProjectedStar<'_>) -> DrawnStar {
    DrawnStar { source_index: star.star.source_index as u32, color: star.star.display_color().rgb(), cell: star.cell.expect("drawn stars have cells"), magnitude: star.star.magnitude }
}

/// Sequential walks advance a span cursor instead of searching the span of every index as `get` does.
struct ProjectedStarIter<'a> { stars: ProjectedStars<'a>, front: usize, back: usize, front_span: usize, back_span: usize }
impl<'a> Iterator for ProjectedStarIter<'a> {
    type Item = ProjectedStar<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back { return None; }
        let star = match self.stars.source {
            DrawSource::Ordered { .. } => self.stars.get(self.front),
            DrawSource::Regional { regions, spans } => {
                while spans[self.front_span].end <= self.front { self.front_span += 1; }
                let span = &spans[self.front_span];
                self.stars.resolve(span, regions[span.region].value()[self.front - span.start])
            }
        };
        self.front += 1;
        Some(star)
    }
    fn size_hint(&self) -> (usize, Option<usize>) { let len = self.back - self.front; (len, Some(len)) }
}
impl DoubleEndedIterator for ProjectedStarIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back { return None; }
        self.back -= 1;
        Some(match self.stars.source {
            DrawSource::Ordered { .. } => self.stars.get(self.back),
            DrawSource::Regional { regions, spans } => {
                while spans[self.back_span].start > self.back { self.back_span -= 1; }
                let span = &spans[self.back_span];
                self.stars.resolve(span, regions[span.region].value()[self.back - span.start])
            }
        })
    }
}
impl ExactSizeIterator for ProjectedStarIter<'_> {}

/// Paint-order walk of drawn records; see `ProjectedStars::drawn`.
pub(crate) enum DrawnStars<'a> {
    Ordered { stars: ProjectedStars<'a>, index: usize },
    Regional { regions: &'a [Cache<RegionalProjectionKey, Vec<DrawnStar>>], spans: std::slice::Iter<'a, DrawnSpan>, current: std::slice::Iter<'a, DrawnStar> },
}
impl Iterator for DrawnStars<'_> {
    type Item = DrawnStar;
    #[inline]
    fn next(&mut self) -> Option<DrawnStar> {
        match self {
            Self::Ordered { stars, index } => {
                if *index == stars.len() { return None; }
                let star = drawn_record(&stars.get(*index));
                *index += 1;
                Some(star)
            }
            Self::Regional { regions, spans, current } => loop {
                if let Some(&star) = current.next() { return Some(star); }
                let span = spans.next()?;
                *current = regions[span.region].value().iter();                            // the next region's records; empty regions fall through
            },
        }
    }
}

/// One dimmest-to-brightest run of paint indices and, on the regional path, the span it belongs to, so visiting
/// it needs no search for its region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SortedRange { pub(crate) indices: std::ops::Range<usize>, span: usize }

/// One drawn star of a region, stored in the region's paint order: the columns the raster and label passes read,
/// in 24 bytes, so neither pass resolves region columns or looks anything up per star.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawnStar {
    pub(crate) source_index: u32, // catalog star index; catalog loading limits star ids, hence rows, to u32
    pub(crate) color: [u8; 3],    // catalog display colour, resolved when the star was projected
    pub(crate) cell: Cell,
    pub(crate) magnitude: f64,
}
row_columns!(DrawnStar { source_index, color, cell, magnitude });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(DrawnStar);

/// One painted region of a frame: paint indices `start..end` are `regional_stars[region]`, requested through
/// descriptor `slot`; `generation` is that cell cache's version, the region's raster dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrawnSpan { pub(crate) slot: usize, pub(crate) region: usize, pub(crate) start: usize, pub(crate) end: usize, pub(crate) generation: u64 }
row_columns!(DrawnSpan { slot, region, start, end, generation });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(DrawnSpan);

#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawRecord {
    pub(crate) magnitude: f64,
    pub(crate) id: StarId,
    pub projected_index: usize,
}
row_columns!(DrawRecord { magnitude, id, projected_index });

pub(crate) type StarKey = (Vec<(Vector3, bool)>, View, ProjectionViewport);
pub(crate) type BodyKey = (Vec<(PlanetKind, Vector3)>, crate::model::Moon, View, ProjectionViewport);
pub(crate) type ConstellationKey = (
    Vec<(usize, Vector3, f64)>,
    std::sync::Arc<crate::model::ConstellationSet>,
    f64,
    View,
    ProjectionViewport,
);
pub(crate) type HorizonGeometry = (Vec<[Cell; 2]>, Vec<(Cell, &'static str)>);

/// Regional dependencies contain only versions and shared camera settings, never copied star rows. A cell key's
/// versions are the region's membership, apparent-direction and draw-order versions: cells are stored in draw order.
pub(crate) type RegionalProjectionKey = ((u64, u64, u64), crate::astro::Matrix3, bool, View, ProjectionViewport);
pub(crate) type RegionalOrderKey = (u64, u64);

/// One drawable row of a region in its draw order (dimmest first, ties by ascending catalog id): the
/// membership-versioned row, its catalog index and current magnitude, 16 bytes. The cells pass reads these
/// sequentially and needs no other column of the region but the directions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RegionalDrawRecord { pub(crate) row: u32, pub(crate) source_index: u32, pub(crate) magnitude: f64 }
row_columns!(RegionalDrawRecord { row, source_index, magnitude });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(RegionalDrawRecord);
