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
    let mut candidates_by_rank = [(0.0, 0u32); DYNAMIC_NAME_COUNT];
    let catalog = sky.stars.catalog();
    for range in sky.stars.sorted_ranges() {
        labels.regions += 1;
        let mut candidates = 0;
        sky.stars.visit_range(range, true, |index, star| {                                 // bright end first, region columns resolved once
            labels.examined += 1;
            if star.magnitude > options.magnitude_threshold { return false; }              // the remaining stars in this region are dimmer
            if !eligible_cell(star.cell) { return true; }
            labels.eligible += 1;
            keep_brightest(&mut labels, &mut candidates_by_rank, index, (star.magnitude, catalog.id(star.source_index).0));
            candidates += 1;
            candidates < DYNAMIC_NAME_COUNT                                                // no sixth candidate from this region can enter the global top five
        });
    }
    labels.indices[..labels.end].reverse();                                                // paint the selected labels dimmest-first
    labels
}

fn keep_brightest(labels: &mut StarLabels, ranks: &mut [(f64, u32); DYNAMIC_NAME_COUNT], index: usize, rank: (f64, u32)) {
    let mut position = labels.end;
    if position == DYNAMIC_NAME_COUNT {
        position -= 1;
        if compare_rank(rank, ranks[position]) != Ordering::Less { return; }
    } else { labels.end += 1; }
    labels.indices[position] = index;
    ranks[position] = rank;
    while position > 0 && compare_rank(rank, ranks[position - 1]) == Ordering::Less {
        labels.indices.swap(position, position - 1);
        ranks.swap(position, position - 1);
        position -= 1;
    }
}

fn compare_rank(a: (f64, u32), b: (f64, u32)) -> Ordering {
    if a.0 == b.0 { b.1.cmp(&a.1) } else { a.0.total_cmp(&b.0) } // signed zero keeps the existing magnitude tie rule
}

#[cfg(test)]
fn compare_brightest(sky: &ProjectedSky<'_>, a: usize, b: usize) -> Ordering {
    let a = sky.stars.get(a).star;
    let b = sky.stars.get(b).star;
    compare_rank((a.magnitude, a.id().0), (b.magnitude, b.id().0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectedStars, View, ProjectionViewport};

    #[test]
    fn regional_top_five_matches_full_selection_with_edges_and_ties() {
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        sky.stars.truncate(18);
        let magnitudes = [14., 12., 10., 8., 6., 4., 15., 13., 11., 9., 7., 5., 20., 18., 16., 3., 2., 1.];
        for (star, magnitude) in sky.stars.iter_mut().zip(magnitudes) { star.magnitude = magnitude; }
        let mut cells: Vec<_> = (0..18).map(|index| (index, (3, 3))).collect();
        cells[17].1 = (0, 3);                                                              // the brightest star cannot paint its full pixel footprint
        let ranges = [(0, 6), (6, 12), (12, 18), (18, 18)];
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 40, height: 40 });
        let cells: Vec<_> = cells.iter().map(|&(row, cell)| (crate::model::RegionalStarIndex { region_slot: 0, observed_index: row as u32 }, cell)).collect();
        let mut projected = data.view(&sky);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &cells, &ranges);
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
        let ranges = [(0, 3), (3, 6), (6, 9)];
        let data = crate::projection::project_sky(&sky, &View::default(), ProjectionViewport { width: 40, height: 40 });
        let cells: Vec<_> = cells.iter().map(|&(row, cell)| (crate::model::RegionalStarIndex { region_slot: 0, observed_index: row as u32 }, cell)).collect();
        let mut projected = data.view(&sky);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &cells, &ranges);
        let options = RenderOptions { magnitude_threshold: 20., dynamic_names: true, unicode: true, braille: false, color: true, constellations: false, grid: false };
        let selected = select_star_labels(&options, &projected, |_| true).map(|i| projected.stars.get(i).star.id()).collect::<Vec<_>>();
        let mut expected = sky.star_views().map(|star| star.id()).collect::<Vec<_>>();
        expected.sort_unstable();
        let expected = &expected[expected.len().saturating_sub(DYNAMIC_NAME_COUNT)..];
        assert_eq!(selected, expected);
        projected.stars = ProjectedStars::from_regions(crate::model::ObservedStars::owned(&sky.stars, &sky.catalog.stars), &[], &[(0, 0)]);
        assert_eq!(select_star_labels(&options, &projected, |_| true).len(), 0);
    }

}
