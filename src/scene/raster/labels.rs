//! Global label selection from the bright end of each independently sorted drawing region.
use crate::{constants::DYNAMIC_NAME_COUNT, model::{Cell, ProjectedSky, RenderOptions}};
use std::cmp::Ordering;

/// A small stack-held set of indices, yielded dimmest-first for label painting.
#[derive(Clone, Copy)]
pub(crate) struct StarLabels {
    indices: [usize; DYNAMIC_NAME_COUNT],
    start: usize,
    end: usize,
    pub(crate) examined: usize,
    pub(crate) regions: usize,
    pub(crate) eligible: usize,
}
impl StarLabels {
    pub(crate) fn contains(&self, index: &usize) -> bool { self.indices[self.start..self.end].contains(index) }
}
impl Iterator for StarLabels {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.start == self.end { return None; }
        let index = self.indices[self.start]; self.start += 1; Some(index)
    }
    fn size_hint(&self) -> (usize, Option<usize>) { let len = self.end - self.start; (len, Some(len)) }
}
impl DoubleEndedIterator for StarLabels {
    fn next_back(&mut self) -> Option<usize> {
        if self.start == self.end { return None; }
        self.end -= 1; Some(self.indices[self.end])
    }
}
impl ExactSizeIterator for StarLabels {}

pub(crate) fn select_star_labels(options: &RenderOptions, sky: &ProjectedSky<'_>, eligible_cell: impl Fn(Cell) -> bool) -> StarLabels {
    let mut labels = StarLabels { indices: [0; DYNAMIC_NAME_COUNT], start: 0, end: 0, examined: 0, regions: 0, eligible: 0 };
    if !options.dynamic_names || DYNAMIC_NAME_COUNT == 0 { return labels; }
    let mut candidates_by_rank = [(0u16, 0u32); DYNAMIC_NAME_COUNT];
    let catalog = sky.stars.catalog();
    let threshold = crate::catalog::magnitude_code(options.magnitude_threshold);          // records hold magnitude codes
    for range in sky.stars.sorted_ranges() {
        labels.regions += 1;
        let mut candidates = 0;
        sky.stars.visit_range(&range, true, |index, star| {                                // bright end first, straight from the region's drawn records
            labels.examined += 1;
            if star.magnitude > threshold { return false; }                                // the remaining stars in this region are dimmer
            if !eligible_cell(star.cell) { return true; }
            labels.eligible += 1;
            keep_brightest(&mut labels, &mut candidates_by_rank, index, star.magnitude, || catalog.id(star.source_index as usize).0);
            candidates += 1;
            candidates < DYNAMIC_NAME_COUNT                                                // no sixth candidate from this region can enter the global top five
        });
    }
    labels.indices[..labels.end].reverse();                                                // paint the selected labels dimmest-first
    labels
}

/// The identifier is only read when the star can enter the list: most candidates are dimmer than the current fifth.
fn keep_brightest(labels: &mut StarLabels, ranks: &mut [(u16, u32); DYNAMIC_NAME_COUNT], index: usize, magnitude: u16, id: impl FnOnce() -> u32) {
    let mut position = labels.end;
    let rank;
    if position == DYNAMIC_NAME_COUNT {
        position -= 1;
        if magnitude > ranks[position].0 { return; }                                        // dimmer than the fifth: no identifier needed
        rank = (magnitude, id());
        if compare_rank(rank, ranks[position]) != Ordering::Less { return; }
    } else {
        labels.end += 1;
        rank = (magnitude, id());
    }
    labels.indices[position] = index;
    ranks[position] = rank;
    while position > 0 && compare_rank(rank, ranks[position - 1]) == Ordering::Less {
        labels.indices.swap(position, position - 1);
        ranks.swap(position, position - 1);
        position -= 1;
    }
}

fn compare_rank(a: (u16, u32), b: (u16, u32)) -> Ordering {
    if a.0 == b.0 { b.1.cmp(&a.1) } else { a.0.cmp(&b.0) }       // brighter code first; equal codes (a thousandth of a magnitude) by id
}

#[cfg(test)]
fn compare_brightest(sky: &ProjectedSky<'_>, a: usize, b: usize) -> Ordering {
    let a = sky.stars.get(a).star;
    let b = sky.stars.get(b).star;
    compare_rank((crate::catalog::magnitude_code(a.magnitude), a.id().0), (crate::catalog::magnitude_code(b.magnitude), b.id().0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectedStars, View, ProjectionViewport};

    /// Regional caches and spans for hand-ordered rows: each group is one region, already in its paint order.
    fn regional(sky: &crate::model::ObservedSky, groups: &[&[(usize, Cell)]]) -> (Vec<crate::cache::Cache<crate::model::RegionalProjectionKey, Vec<crate::model::DrawnStar>>>, Vec<crate::model::DrawnSpan>) {
        let (mut regions, mut spans, mut start) = (Vec::new(), Vec::new(), 0);
        for (slot, group) in groups.iter().enumerate() {
            let stars: Vec<_> = group.iter().map(|&(row, cell)| crate::model::DrawnStar { source_index: sky.stars[row].source_index as u32, cell,
                magnitude: crate::catalog::magnitude_code(sky.stars[row].magnitude), color: sky.star_view(row).display_color().index() }).collect();
            let mut cache = crate::cache::Cache::default();
            cache.store(((1, 1, 1), crate::astro::Matrix3::IDENTITY, false, View::default(), ProjectionViewport { width: 40, height: 40 }), 0.0, 0.0, stars);
            spans.push(crate::model::DrawnSpan { slot, region: slot, start, end: start + group.len(), generation: cache.generation });
            start += group.len();
            regions.push(cache);
        }
        (regions, spans)
    }
    #[test]
    fn regional_top_five_matches_full_selection_with_edges_and_ties() {
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        sky.stars.truncate(18);
        let magnitudes = [14., 12., 10., 8., 6., 4., 15., 13., 11., 9., 7., 5., 20., 18., 16., 3., 2., 1.];
        for (star, magnitude) in sky.stars.iter_mut().zip(magnitudes) { star.magnitude = magnitude; }
        let mut cells: Vec<_> = (0..18).map(|index| (index, (3, 3))).collect();
        cells[17].1 = (0, 3);                                                              // the brightest star cannot paint its full pixel footprint
        let (regions, spans) = regional(&sky, &[&cells[..6], &cells[6..12], &cells[12..], &[]]);
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 40, height: 40 });
        let mut projected = data.view(&sky);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &regions, &spans);
        let options = RenderOptions { magnitude_threshold: 20., dynamic_names: true, unicode: true, braille: false, color: true, constellations: true, grid: false };
        for pixels in [false, true] {
            let accepts = |cell| !pixels || crate::scene::pixel_star_fits(cell, projected.viewport);
            let selected: Vec<_> = select_star_labels(&options, &projected, accepts).collect();
            let mut expected: Vec<_> = (0..cells.len()).filter(|&index| accepts(cells[index].1)).collect();
            expected.sort_unstable_by(|&a, &b| compare_brightest(&projected, a, b));
            expected.truncate(DYNAMIC_NAME_COUNT); expected.reverse();
            assert_eq!(selected, expected);
        }
        let calls = std::cell::Cell::new(0);
        select_star_labels(&options, &projected, |_| { calls.set(calls.get() + 1); true });
        assert_eq!(calls.get(), 3 * DYNAMIC_NAME_COUNT);
        let disabled = RenderOptions { dynamic_names: false, ..options };
        assert_eq!(select_star_labels(&disabled, &projected, |_| panic!("disabled labels must not read candidates")).len(), 0);
    }
    #[test]
    fn equal_brightness_labels_use_global_ids_not_region_priority() {
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        sky.stars.truncate(9);
        for (index, star) in sky.stars.iter_mut().enumerate() { star.magnitude = if index % 2 == 0 { 0.0 } else { -0.0 }; }
        let mut cells: Vec<_> = (0..9).map(|index| (index, (3, 3))).collect();
        for region in cells.chunks_mut(3) { region.sort_unstable_by_key(|&(row, _)| sky.star_view(row).id()); }
        let (regions, spans) = regional(&sky, &[&cells[..3], &cells[3..6], &cells[6..]]);
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 40, height: 40 });
        let mut projected = data.view(&sky);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &regions, &spans);
        let options = RenderOptions { magnitude_threshold: 20., dynamic_names: true, unicode: true, braille: false, color: true, constellations: false, grid: false };
        let selected = select_star_labels(&options, &projected, |_| true).map(|i| projected.stars.get(i).star.id()).collect::<Vec<_>>();
        let mut expected = sky.star_views().map(|star| star.id()).collect::<Vec<_>>();
        expected.sort_unstable();
        let expected = &expected[expected.len().saturating_sub(DYNAMIC_NAME_COUNT)..];
        assert_eq!(selected, expected);
        let (regions, spans) = regional(&sky, &[&[]]);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &regions, &spans);
        assert_eq!(select_star_labels(&options, &projected, |_| true).len(), 0);
    }

}
