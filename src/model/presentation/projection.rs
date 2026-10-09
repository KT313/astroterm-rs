//! Camera settings and projected records; algorithms live in projection.
use crate::constants::DEFAULT_FOV_DEGREES;
use crate::rows::row_columns;
use crate::astro::{Horizontal, Vector3};
use std::f64::consts::{PI, FRAC_PI_2};
use crate::model::{ObservedStarView, PlanetKind};
use crate::catalog::{StarNames, StarId};


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

/// Read-only drawing order: either a legacy permutation or directly ordered regional cells.
/// Regional ranges identify independently brightness-sorted runs for bounded global label selection.
#[derive(Clone, Copy, Debug)]
pub struct ProjectedStars<'a> {
    observed: crate::model::ObservedStars<'a>,
    cells: &'a [(usize, Cell)],
    regional_cells: &'a [(RegionalStarIndex, Cell)],
    order: Option<&'a [usize]>,
    regions: Option<&'a [(usize, usize)]>,
}
impl<'a> ProjectedStars<'a> {
    pub fn new(observed: &'a crate::model::ObservedSky, cells: &'a [(usize, Cell)], order: &'a [usize]) -> Self {
        Self { observed: crate::model::ObservedStars::owned(&observed.stars, &observed.catalog.stars), cells, regional_cells: &[], order: Some(order), regions: None }
    }
    pub(crate) fn with_order(observed: crate::model::ObservedStars<'a>, cells: &'a [(usize, Cell)], order: &'a [usize]) -> Self {
        Self { observed, cells, regional_cells: &[], order: Some(order), regions: None }
    }
    pub(crate) fn from_regions(observed: crate::model::ObservedStars<'a>, cells: &'a [(RegionalStarIndex, Cell)], regions: &'a [(usize, usize)]) -> Self {
        Self { observed, cells: &[], regional_cells: cells, order: None, regions: Some(regions) }
    }
    pub fn len(&self) -> usize { self.order.map_or(self.regional_cells.len(), <[usize]>::len) }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn get(&self, index: usize) -> ProjectedStar<'a> {
        if let Some(order) = self.order {
            let (row, cell) = self.cells[order[index]];
            ProjectedStar { star: self.observed.get(row), cell: Some(cell) }
        } else {
            let (row, cell) = self.regional_cells[index];
            ProjectedStar { star: self.observed.get_regional(row.region_slot as usize, row.observed_index as usize), cell: Some(cell) }
        }
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = ProjectedStar<'a>> + ExactSizeIterator + '_ {
        (0..self.len()).map(|index| self.get(index))
    }
    /// Each range is dimmest-to-brightest. The legacy path is one globally sorted range.
    pub(crate) fn sorted_ranges(&self) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
        self.regions.into_iter().flatten().map(|&(start, end)| start..end)
            .chain(self.regions.is_none().then_some(0..self.len()))
    }
    pub(crate) fn catalog(&self) -> &'a crate::model::StarStorage { self.observed.catalog() }
    /// Visit the drawn stars of one sorted range in paint order, or from the bright end when `brightest_first`.
    /// On the regional path the region's columns are resolved once for the whole range, so each star is a few
    /// sequential slice reads. `visit` gets the paint index and returns false to stop the range early.
    pub(crate) fn visit_range(&self, range: std::ops::Range<usize>, brightest_first: bool, mut visit: impl FnMut(usize, DrawnStar) -> bool) {
        let Some(order) = self.order else {
            let entries = &self.regional_cells[range.clone()];
            let Some(first) = entries.first() else { return; };
            let (columns, base) = self.observed.slot_columns(first.0.region_slot as usize);
            let mut step = |index: usize, &(address, cell): &(RegionalStarIndex, Cell)| {
                debug_assert_eq!(address.region_slot, first.0.region_slot, "one sorted range holds one region");
                let row = address.observed_index as usize - base;
                visit(index, DrawnStar { source_index: columns.source_index(row), magnitude: columns.magnitude(row), cell })
            };
            let indexed = range.zip(entries);
            if brightest_first { for (index, entry) in indexed.rev() { if !step(index, entry) { return; } } }
            else { for (index, entry) in indexed { if !step(index, entry) { return; } } }
            return;
        };
        let mut step = |index: usize| {
            let (row, cell) = self.cells[order[index]];
            let star = self.observed.get(row);
            visit(index, DrawnStar { source_index: star.source_index, magnitude: star.magnitude, cell })
        };
        if brightest_first { for index in range.rev() { if !step(index) { return; } } }
        else { for index in range { if !step(index) { return; } } }
    }
    /// Visit every drawn star in paint order with the paint index; region columns are resolved once per region.
    pub(crate) fn visit_drawn(&self, mut visit: impl FnMut(usize, DrawnStar)) {
        for range in self.sorted_ranges() {
            self.visit_range(range, false, |index, star| { visit(index, star); true });
        }
    }
}

/// The facts raster loops need about one drawn star; read from region columns, no catalog view is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawnStar {
    pub(crate) source_index: usize,
    pub(crate) magnitude: f64,
    pub(crate) cell: Cell,
}
impl PartialEq for ProjectedStars<'_> {
    fn eq(&self, other: &Self) -> bool { self.iter().eq(other.iter()) }
}

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

/// Regional dependencies contain only versions and shared camera settings, never copied star rows.
pub(crate) type RegionalProjectionKey = ((u64, u64), crate::astro::Matrix3, bool, View, ProjectionViewport);
pub(crate) type RegionalOrderKey = (u64, u64);
pub(crate) type RegionalDrawRecord = (usize, f64, StarId); // membership-versioned row within this region, current magnitude, tie-breaking identifier

/// Frame-local address: region_slot selects an observation descriptor, observed_index selects a final direction.
/// Packed to the previous usize footprint on 64-bit targets; neither value is a stable catalog index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegionalStarIndex { pub region_slot: u32, pub observed_index: u32 }
row_columns!(RegionalStarIndex { region_slot, observed_index });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(RegionalStarIndex);
