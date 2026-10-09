//! Local read-only combinations of original metadata, samples and final direction buffers; no row storage.
use crate::model::ObservedStarState;
use crate::{astro::{Vector3, models::stars::StellarSample}, cache::Cache,
    model::{ObservedSky, ObservedStar, ObservedStarView, ObservedRegion, ObservationRegion, SelectedStar, StarStorage, Directions, Planet, Moon}};

#[derive(Clone, Copy, Debug)]
pub struct ObservedStars<'a> {
    catalog: &'a StarStorage,
    source: StarSource<'a>,
}
#[derive(Clone, Copy, Debug)]
enum StarSource<'a> {
    Owned(&'a [ObservedStar]),
    Regional { descriptors: &'a [ObservedRegion], records: &'a [ObservationRegion], samples: &'a [Cache<(), Vec<StellarSample>>], offsets: &'a [usize], directions: &'a [Vector3] },
}
impl<'a> ObservedStars<'a> {
    pub fn owned(rows: &'a [ObservedStar], catalog: &'a StarStorage) -> Self { Self { catalog, source: StarSource::Owned(rows) } }
    pub(crate) fn regional(catalog: &'a StarStorage, descriptors: &'a [ObservedRegion], records: &'a [ObservationRegion], samples: &'a [Cache<(), Vec<StellarSample>>], offsets: &'a [usize], directions: &'a [Vector3]) -> Self {
        Self { catalog, source: StarSource::Regional { descriptors, records, samples, offsets, directions } }
    }
    pub(crate) fn catalog(self) -> &'a StarStorage { self.catalog }
    pub fn len(self) -> usize { match self.source { StarSource::Owned(rows) => rows.len(), StarSource::Regional { directions, .. } => directions.len() } }
    pub fn is_empty(self) -> bool { self.len() == 0 }
    pub fn get(self, index: usize) -> ObservedStarView<'a> {
        match self.source {
            StarSource::Owned(rows) => ObservedStarView { state: ObservedStarState::Borrowed(&rows[index]), catalog: self.catalog },
            StarSource::Regional { descriptors, .. } => {
                let slot = descriptors.partition_point(|region| region.end <= index);
                self.get_regional(slot, index)
            }
        }
    }
    /// Explicit addressing is used by the hot projection/drawing path; no region search per star.
    pub(crate) fn get_regional(self, slot: usize, index: usize) -> ObservedStarView<'a> {
        let StarSource::Regional { descriptors, records, samples, offsets, directions } = self.source else { return self.get(index); };
        let region = &descriptors[slot];
        let row = &records[region.region].corrections.stored().expect("published correction records").0[index - region.start];
        let sample = &samples[region.region].stored().expect("published stellar samples")[row.source_index - offsets[region.region]];
        ObservedStarView { catalog: self.catalog, state: ObservedStarState::Owned(ObservedStar {
            source_index: row.source_index, drawable: row.drawable, magnitude: sample.magnitude, position: directions[index],
        }) } // a small value on access, never a retained combined array
    }
    pub fn iter(self) -> impl DoubleEndedIterator<Item = ObservedStarView<'a>> + ExactSizeIterator {
        let last_region = match self.source { StarSource::Regional { descriptors, .. } => descriptors.len().saturating_sub(1), StarSource::Owned(_) => 0 };
        ObservedStarIter { stars: self, front: 0, back: self.len(), front_region: 0, back_region: last_region }
    }
    /// Resolve one requested region's columns once: the region in `slot` of the frame's descriptor list.
    /// Returns the columns and the frame-wide index of the region's first row. Hot loops index the result
    /// instead of rebuilding a star per row; the cache checks run once per region, not once per star.
    pub(crate) fn slot_columns(self, slot: usize) -> (RegionData<'a>, usize) {
        match self.source {
            StarSource::Owned(rows) => (RegionData::Owned(rows), 0),
            StarSource::Regional { descriptors, records, samples, offsets, directions } => {
                let region = &descriptors[slot];
                let rows = records[region.region].corrections.stored().expect("published correction records").0.as_slice();
                let samples = samples[region.region].stored().expect("published stellar samples").as_slice();
                let directions = &directions[region.start..region.end];
                assert_eq!(rows.len(), directions.len(), "regional rows must match the frame's direction range");
                (RegionData::Regional { rows, directions, samples, offset: offsets[region.region] }, region.start)
            }
        }
    }
    /// Columns of the region described by `descriptor`, which must be the frame's descriptor for `slot`.
    pub(crate) fn region(self, slot: usize, descriptor: &ObservedRegion) -> RegionData<'a> {
        match self.source {
            StarSource::Owned(rows) => RegionData::Owned(&rows[descriptor.start..descriptor.end]),
            StarSource::Regional { descriptors, .. } => {
                debug_assert_eq!(descriptors[slot].region, descriptor.region, "descriptor must belong to its slot");
                self.slot_columns(slot).0
            }
        }
    }
    /// The columns holding every constellation endpoint, resolved once: the exclusive constellation region on the
    /// production path (normally the last requested region), or all stars of a flat sky. `None` when that region is
    /// not requested this frame. Endpoint lookups then cost one binary search each instead of three.
    pub(crate) fn constellation_columns(self) -> Option<RegionData<'a>> {
        match self.source {
            StarSource::Owned(rows) => Some(RegionData::Owned(rows)),
            StarSource::Regional { descriptors, .. } => {
                let slot = descriptors.iter().rposition(|region| region.region == crate::constants::CONSTELLATION_REGION)?;
                Some(self.slot_columns(slot).0)
            }
        }
    }
    /// General lookup for any star; searches the region first. Hot paths use `constellation_columns` instead.
    pub(crate) fn find(self, source_index: usize) -> Option<ObservedStarView<'a>> {
        match self.source {
            StarSource::Owned(rows) => rows.binary_search_by_key(&source_index, |row| row.source_index).ok().map(|index| self.get(index)),
            StarSource::Regional { descriptors, records, offsets, .. } => {
                if source_index >= *offsets.last()? { return None; }
                let region = offsets.partition_point(|&start| start <= source_index) - 1;
                let slot = descriptors.binary_search_by_key(&region, |entry| entry.region).ok()?;
                let local = records[region].corrections.stored()?.0.binary_search_by_key(&source_index, |row| row.source_index).ok()?;
                Some(self.get_regional(slot, descriptors[slot].start + local))
            }
        }
    }
}

/// One region's borrowed columns, resolved once by `ObservedStars::region` or `slot_columns`. Each accessor is a
/// plain slice index, so loops over a region read sequential memory and touch only the columns they ask for.
/// `Owned` serves flat skies (headless callers and tests); `Regional` serves the cached production path, where
/// `samples` covers every catalog star of the region and is addressed by `source_index - offset`.
#[derive(Clone, Copy, Debug)]
pub enum RegionData<'a> {
    Owned(&'a [ObservedStar]),
    Regional { rows: &'a [SelectedStar], directions: &'a [Vector3], samples: &'a [StellarSample], offset: usize },
}
impl RegionData<'_> {
    pub fn len(&self) -> usize {
        match self { Self::Owned(rows) => rows.len(), Self::Regional { rows, .. } => rows.len() }
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    /// Row of the star with this catalog index. Rows are sorted by `source_index`, so this is one binary search.
    pub fn find_row(&self, source_index: usize) -> Option<usize> {
        match self {
            Self::Owned(rows) => rows.binary_search_by_key(&source_index, |row| row.source_index).ok(),
            Self::Regional { rows, .. } => rows.binary_search_by_key(&source_index, |row| row.source_index).ok(),
        }
    }
    pub fn source_index(&self, row: usize) -> usize {
        match self { Self::Owned(rows) => rows[row].source_index, Self::Regional { rows, .. } => rows[row].source_index }
    }
    pub fn drawable(&self, row: usize) -> bool {
        match self { Self::Owned(rows) => rows[row].drawable, Self::Regional { rows, .. } => rows[row].drawable }
    }
    pub fn magnitude(&self, row: usize) -> f64 {
        match self {
            Self::Owned(rows) => rows[row].magnitude,
            Self::Regional { rows, samples, offset, .. } => samples[rows[row].source_index - offset].magnitude,
        }
    }
    pub fn position(&self, row: usize) -> Vector3 {
        match self { Self::Owned(rows) => rows[row].position, Self::Regional { directions, .. } => directions[row] }
    }
    /// The complete record of one row, for callers that need every column.
    pub fn star(&self, row: usize) -> ObservedStar {
        match self {
            Self::Owned(rows) => rows[row],
            Self::Regional { .. } => ObservedStar {
                source_index: self.source_index(row), drawable: self.drawable(row), magnitude: self.magnitude(row), position: self.position(row),
            },
        }
    }
}

/// Sequential scans advance a region cursor instead of searching for each star; get() also supports random access.
struct ObservedStarIter<'a> { stars: ObservedStars<'a>, front: usize, back: usize, front_region: usize, back_region: usize }
impl<'a> Iterator for ObservedStarIter<'a> {
    type Item = ObservedStarView<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back { return None; }
        if let StarSource::Regional { descriptors, .. } = self.stars.source {
            while descriptors[self.front_region].end <= self.front { self.front_region += 1; }
        }
        let star = self.stars.get_regional(self.front_region, self.front);
        self.front += 1;
        Some(star)
    }
    fn size_hint(&self) -> (usize, Option<usize>) { let len = self.back - self.front; (len, Some(len)) }
}
impl DoubleEndedIterator for ObservedStarIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back { return None; }
        if let StarSource::Regional { descriptors, .. } = self.stars.source {
            while descriptors[self.back_region].start >= self.back { self.back_region -= 1; }
        }
        self.back -= 1;
        Some(self.stars.get_regional(self.back_region, self.back))
    }
}
impl ExactSizeIterator for ObservedStarIter<'_> {}

#[derive(Clone, Copy, Debug)]
pub struct ObservedPlanets<'a> { rows: &'a [Planet], directions: Option<&'a [Vector3]> }
impl ObservedPlanets<'_> {
    pub fn len(self) -> usize { self.rows.len() }
    pub fn is_empty(self) -> bool { self.rows.is_empty() }
    pub fn get(self, index: usize) -> Planet {
        let mut planet = self.rows[index];
        if let Some(directions) = self.directions { planet.position = directions[index]; }
        planet
    }
    pub fn iter(self) -> impl ExactSizeIterator<Item = Planet> { (0..self.len()).map(move |index| self.get(index)) }
}

#[derive(Clone, Copy, Debug)]
pub struct ObservedSkyView<'a> {
    summary: &'a ObservedSky,
    pub catalog: &'a std::sync::Arc<crate::model::SkyCatalog>,
    pub corrections: crate::model::CorrectionStats,
    pub selection: crate::model::SelectionStats,
    pub magnitude_threshold: f64,
    pub runtime_singular_count: usize,
    pub outside_accuracy_range: bool,
    pub stars: ObservedStars<'a>,
    pub planets: ObservedPlanets<'a>,
    pub moon: Moon,
}
impl<'a> From<&'a ObservedSky> for ObservedSkyView<'a> {
    fn from(summary: &'a ObservedSky) -> Self {
        Self {
            summary, catalog: &summary.catalog, corrections: summary.corrections, selection: summary.selection,
            magnitude_threshold: summary.magnitude_threshold, runtime_singular_count: summary.runtime_singular_count,
            outside_accuracy_range: summary.outside_accuracy_range,
            stars: ObservedStars::owned(&summary.stars, &summary.catalog.stars),
            planets: ObservedPlanets { rows: &summary.planets, directions: None }, moon: summary.moon,
        }
    }
}
impl<'a> ObservedSkyView<'a> {
    pub(crate) fn summary(self) -> &'a ObservedSky { self.summary }
    pub(crate) fn cached(summary: &'a ObservedSky, stars: ObservedStars<'a>, directions: &'a Directions) -> Self {
        let mut view = Self::from(summary); // shared metadata is borrowed; no star rows are visited
        view.stars = stars;
        view.planets.directions = Some(&directions.1);
        view.moon.position = directions.2;
        view
    }
    pub fn figures(&self) -> &'a std::sync::Arc<crate::model::ConstellationSet> { self.summary.figures() }
    pub fn constellations(&self) -> &'a [crate::model::Constellation] { self.figures().figures() }
    pub fn star_views(&self) -> impl DoubleEndedIterator<Item = ObservedStarView<'a>> + ExactSizeIterator { self.stars.iter() }
    pub fn star_view(&self, index: usize) -> ObservedStarView<'a> { self.stars.get(index) }
    pub fn sun(&self) -> Planet { self.planets.get(0) }
    /// Explicit owned compatibility/export operation. Production projection and rendering borrow instead.
    pub fn materialize(&self) -> ObservedSky {
        let summary = self.summary;
        ObservedSky {
            corrections: summary.corrections, magnitude_threshold: summary.magnitude_threshold, selection: summary.selection,
            candidate_indices: summary.candidate_indices.clone(), runtime_singular_count: summary.runtime_singular_count,
            catalog: summary.catalog.clone(), stars: self.stars.iter().map(|star| star.state.into_owned()).collect(),
            planets: self.planets.iter().collect(), moon: self.moon, figure_override: summary.figure_override.clone(),
            refracted: summary.refracted, outside_accuracy_range: summary.outside_accuracy_range,
        }
    }
}
