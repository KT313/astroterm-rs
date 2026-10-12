//! Catalog preparation details; ordering and representative choices stay identical to the source loader.
use crate::catalog::{CatalogStar, StarId, ConstellationFigure};
use crate::model::{StarStorage, Constellation};
use std::collections::{HashMap, HashSet};
use super::prepare_star;
mod sorting;
pub(super) use sorting::{create_sort_pool, sort_source_ids, sort_stars_by_region_and_brightness};

pub(super) fn prepare_compact_stars(entries: impl Iterator<Item = CatalogStar>, source_names: &crate::catalog::StarNames, progress: &mut dyn crate::progress::DatasetProgress) -> std::io::Result<(StarStorage, Vec<f32>, crate::catalog::StarNames)> {
    let mut stars = StarStorage::default();
    let capacity = entries.size_hint().0;
    let total = entries.size_hint().1.filter(|&upper| upper == capacity).map(|n| n as u64);
    progress.begin_stage(crate::progress::Stage::Convert, "", crate::progress::Unit::Stars, total);
    if capacity as u64 <= u64::from(u32::MAX) { stars.reserve(capacity); }
    let mut bounds = Vec::new();
    let mut labels = crate::catalog::LabelBuilder::default();
    let mut processed = 0_u64;
    for entry in entries {
        processed += 1;
        if processed.is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS) { progress.set_position(processed); }
        if !entry.has_data { continue; }
        crate::catalog::Catalog::check_star_count(stars.len() as u64 + 1).map_err(std::io::Error::other)?;
        let mut star = prepare_star(&entry);
        star.name = if let Some(name) = entry.name {
            let text = source_names.get(Some(name)).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid source label reference"))?;
            Some(labels.insert(text.to_owned(), None)?)
        } else if let Some(designation) = entry.designation {
            Some(labels.insert(designation.format(true), Some(designation.format(false)))?)
        } else { None };
        bounds.push(stars.push(star)?);
    }
    progress.set_position(processed);
    progress.set_detail("Finalizing compact star storage");
    stars.shrink_to_fit();
    bounds.shrink_to_fit();
    let names = labels.finish();
    progress.finish_stage(crate::progress::Outcome::Complete);
    Ok((stars, bounds, names))
}

/// Resolve retained segments before sorting, so only actual endpoint IDs enter the dedicated region.
pub(super) fn collect_endpoint_ids(stars: &StarStorage, hr_by_id: &HashMap<StarId, u32>, figures: &[ConstellationFigure]) -> HashSet<StarId> {
    if figures.is_empty() || hr_by_id.is_empty() { return HashSet::new(); }
    let indices = index_representative_positions(stars, hr_by_id);
    let resolved = resolve_constellation_figures(figures, &indices);
    collect_constellation_endpoints(&resolved).into_iter().map(|i| stars.id(i)).collect()
}

pub(super) fn index_representative_ids(representatives: &HashMap<u32, StarId>) -> HashMap<StarId, u32> {
    representatives.iter().map(|(&hr, &id)| (id, hr)).collect()
}

pub(super) fn index_representative_positions(stars: &StarStorage, hr_by_id: &HashMap<StarId, u32>) -> HashMap<u32, usize> {
    if hr_by_id.is_empty() { return HashMap::new(); }
    stars.iter().enumerate().filter_map(|(index, star)| Some((*hr_by_id.get(&star.id)?, index))).collect()
}

pub(super) fn resolve_constellation_figures(figures: &[ConstellationFigure], index_by_hr: &HashMap<u32, usize>) -> Vec<Constellation> {
    figures.iter().filter_map(|figure| resolve_constellation_figure(figure, index_by_hr)).collect()
}

pub(super) fn collect_constellation_endpoints(constellations: &[Constellation]) -> Vec<usize> {
    let mut endpoints: Vec<_> = constellations.iter().flat_map(|figure| figure.segments.iter().flatten()).copied().collect();
    endpoints.sort_unstable();
    endpoints.dedup();
    endpoints
}


/// A figure with its stars as indices into the star table. Segments with a star missing from the dataset are left
/// out, and so is a figure without any segments left.
fn resolve_constellation_figure(
    figure: &ConstellationFigure,
    index_by_hr: &HashMap<u32, usize>,
) -> Option<Constellation> {
    let segments: Vec<[usize; 2]> = figure
        .segments
        .iter()
        .filter_map(|[a, b]| Some([*index_by_hr.get(a)?, *index_by_hr.get(b)?]))
        .collect();
    (!segments.is_empty()).then_some(Constellation {
        abbreviation: figure.abbreviation,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use crate::model::SkyCatalog;
    use crate::catalog::{Catalog, StarNames, CatalogStar, ConstellationFigure, load_embedded_catalog};

    #[test]
    fn empty_constellation_inputs_do_not_expand_star_rows() {
        let mut stars = crate::model::StarStorage::default();
        let row = crate::model::StarRow { u0: [1.0, 0.0, 0.0], w: [0.0; 3], magnitude: 10000,
            brightness_key: 10000, distance: 0.0, id: 7, name: 0, display_color: 255 };
        stars.push_prepared(row, [0, 0]);                             // expanding this deliberately invalid palette would panic
        let empty = std::collections::HashMap::new();
        assert!(super::index_representative_positions(&stars, &empty).is_empty());
        let mapping = std::collections::HashMap::from([(crate::catalog::StarId(7), 7)]);
        assert!(super::collect_endpoint_ids(&stars, &mapping, &[]).is_empty());
        let figure = ConstellationFigure { abbreviation: "Ori", segments: vec![[7, 8]] };
        assert!(super::collect_endpoint_ids(&stars, &empty, &[figure]).is_empty());
    }

    fn build() -> SkyCatalog {
        crate::sky::prepare_catalog(&load_embedded_catalog(&mut crate::progress::Report::quiet()).expect("embedded catalog loads"), &mut crate::progress::Report::quiet()).unwrap().catalog
    }

    /// The row of a catalog star by its HR number.
    fn row(catalog: &SkyCatalog, id: u32) -> usize {
        (0..catalog.stars.len()).find(|&index| catalog.stars.id(index).0 == id).unwrap_or_else(|| panic!("HR {id}"))
    }

    #[test]
    fn ids_survive_compaction_and_brightness_sorting() {
        let catalog = build();
        assert_eq!(catalog.stars.len(), 9110 - 14);
        assert_eq!(catalog.star_name(&catalog.stars.get(row(&catalog, 7001))), Some("Vega"));
        let source = load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap();
        for index in 0..catalog.stars.len() {
            let original = &source.stars[catalog.stars.id(index).0 as usize - 1];
            let expected = source.names.get(original.name).map(str::to_owned)
                .unwrap_or_else(|| original.designation.unwrap().format(true));
            assert_eq!(catalog.star_name(&catalog.stars.get(index)), Some(expected.as_str()));
        }
        for range in catalog.grid.offsets.windows(2) {
            let stars = &catalog.stars;
            for i in range[0] + 1..range[1] {
                assert!(
                    stars.brightness_key(i - 1) < stars.brightness_key(i)
                        || (stars.brightness_key(i - 1) == stars.brightness_key(i) && stars.id(i - 1) > stars.id(i))
                );
            }
        }
    }

    #[test]
    fn stars_keep_their_names_and_prepared_colors() {
        let catalog = build();
        let star = |id| { let index = row(&catalog, id); (catalog.star_name(&catalog.stars.get(index)), catalog.stars.display_color(index)) };
        assert_eq!(star(2061), (Some("Betelgeuse"), crate::model::StarColor::RedOrange));
        assert_eq!(star(5340), (Some("Arcturus"), crate::model::StarColor::Orange));
    }

    #[test]
    fn placeholder_stars_are_never_prepared() {
        let catalog = build();
        assert_eq!(catalog.stars.len(), 9110 - 14);
        assert!(!(0..catalog.stars.len()).any(|index| catalog.stars.id(index).0 == 92)); // HR 92 has no data
    }

    #[test]
    fn constellations_are_matched_by_hr_number_in_any_dataset() {
        let star = |hr: Option<u32>| CatalogStar {
            id: crate::catalog::StarId(hr.unwrap_or(100)),
            space_motion: None,
            hr,
            name: None,
            designation: None,
            right_ascension: 0.0,
            declination: 0.0,
            ra_motion: 0.0,
            ra_motion_cos_dec: 0.0,
            dec_motion: 0.0,
            magnitude: 1.0,
            spectral_type: *b"  ",
            color_index: None,
            has_data: true,
        };
        let figure = |abbreviation, segments| ConstellationFigure { abbreviation, segments };
        let catalog = Catalog::new(
            vec![star(Some(30)), star(None), star(Some(10)), star(Some(20))],
            StarNames::default(),
            vec![
                figure("Abc", vec![[10, 20], [20, 99]]), // HR 99 isn't in the dataset
                figure("Def", vec![[98, 99]]),
            ],
        );
        let catalog = crate::sky::prepare_catalog(&catalog, &mut crate::progress::Report::quiet()).unwrap().catalog;
        assert_eq!(catalog.constellations().len(), 1);
        let [a, b] = catalog.constellations()[0].segments[0];
        assert_eq!((catalog.stars.id(a).0, catalog.stars.id(b).0), (10, 20));
    }

    #[test]
    fn constellations_reference_star_indices() {
        let catalog = build();
        assert_eq!(catalog.constellations().len(), 88);
        let [a, b] = catalog.constellations()[19].segments[0];
        assert_eq!((catalog.stars.id(a).0, catalog.stars.id(b).0), (4785, 4915));
    }
}
