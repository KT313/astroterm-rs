//! Local read-only combinations of original metadata, samples and final direction buffers; no row storage.
use crate::model::ObservedStarState;
use crate::{astro::{Vector3, models::stars::StellarSample}, cache::Cache,
    model::{ObservedSky, ObservedStar, ObservedStarView, ObservedRegion, ObservationRegion, StarStorage, Directions, Planet, Moon}};

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
    pub(crate) fn region(self, slot: usize, region: &ObservedRegion) -> impl DoubleEndedIterator<Item = ObservedStarView<'a>> + ExactSizeIterator {
        (region.start..region.end).map(move |index| self.get_regional(slot, index))
    }
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
