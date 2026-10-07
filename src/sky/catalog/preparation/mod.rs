//! Catalog preparation details; ordering and representative choices stay identical to the source loader.
use crate::catalog::{CatalogStar, StarId, ConstellationFigure};
use crate::model::{StarStorage, Constellation};
use std::collections::HashMap;
use super::{grid, prepare_star};

pub(super) fn prepare_compact_stars(entries: impl Iterator<Item = CatalogStar>, source_names: &crate::catalog::StarNames) -> std::io::Result<(StarStorage, Vec<f32>, crate::catalog::StarNames)> {
    let mut stars = StarStorage::default();
    let capacity = entries.size_hint().0;
    if capacity as u64 <= u64::from(u32::MAX) { stars.reserve(capacity); }
    let mut bounds = Vec::new();
    let mut labels = crate::catalog::LabelBuilder::default();
    for entry in entries.filter(|s| s.has_data) {
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
    stars.shrink_to_fit();
    bounds.shrink_to_fit();
    Ok((stars, bounds, labels.finish()))
}

pub(super) fn sort_stars_by_region_and_brightness(stars: &mut StarStorage, bounds: &mut [f32]) {
    let cells: Vec<_> = (0..stars.len()).map(|i| grid::stored_cell(stars, i)).collect();
    let mut order: Vec<_> = (0..stars.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        cells[a]
            .cmp(&cells[b])
            .then_with(|| stars.brightness_key(a).total_cmp(&stars.brightness_key(b)))
            .then_with(|| stars.id(b).cmp(&stars.id(a)))
    });
    stars.reorder(&order, bounds);
    drop(order);
    drop(cells);
}

pub(super) fn index_representative_ids(representatives: &HashMap<u32, StarId>) -> HashMap<StarId, u32> {
    representatives.iter().map(|(&hr, &id)| (id, hr)).collect()
}

pub(super) fn index_representative_positions(stars: &StarStorage, hr_by_id: &HashMap<StarId, u32>) -> HashMap<u32, usize> {
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
    use crate::model::Sky;
    use crate::catalog::{Catalog, StarNames, CatalogStar, ConstellationFigure, load_embedded_catalog};

    fn build_sky() -> Sky {
        crate::sky::create_sky_from_catalog(&load_embedded_catalog().expect("embedded catalog loads")).unwrap()
    }

    #[test]
    fn ids_survive_compaction_and_brightness_sorting() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert_eq!(
            sky.star_name(&sky.star_views().find(|star| star.id().0 == 7001).unwrap()),
            Some("Vega")
        );
        let source = load_embedded_catalog().unwrap();
        for star in sky.star_views() {
            let original = &source.stars[star.id().0 as usize - 1];
            let expected = source.names.get(original.name).map(str::to_owned)
                .unwrap_or_else(|| original.designation.unwrap().format(true));
            assert_eq!(sky.star_name(&star), Some(expected.as_str()));
        }
        for range in sky.catalog.grid.offsets.windows(2) {
            let stars = &sky.catalog.stars;
            for i in range[0] + 1..range[1] {
                assert!(
                    stars.brightness_key(i - 1) < stars.brightness_key(i)
                        || (stars.brightness_key(i - 1) == stars.brightness_key(i) && stars.id(i - 1) > stars.id(i))
                );
            }
        }
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let projected_data = crate::projection::project_sky(
            &sky,
            &crate::model::View::default(),
            crate::model::ProjectionViewport { height: 41, width: 81 },
        );
        let projected = projected_data.view(&sky);
        let drawn: Vec<_> = projected.stars.iter().map(|s| s.star.id().0).collect();
        assert_eq!(&drawn[..3], &[1894, 365, 3313]);
        assert_eq!(
            drawn.iter().rev().take(3).copied().collect::<Vec<_>>(),
            [2491, 2326, 5340]
        );
    }

    #[test]
    fn stars_keep_their_names_and_prepared_colors() {
        let sky = build_sky();
        let star = |id| sky.star_views().find(|star| star.id().0 == id).unwrap();
        assert_eq!(
            (sky.star_name(&star(2061)), star(2061).display_color()),
            (Some("Betelgeuse"), crate::model::StarColor::RedOrange)
        );
        assert_eq!(
            (sky.star_name(&star(5340)), star(5340).display_color()),
            (Some("Arcturus"), crate::model::StarColor::Orange)
        );
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert!(!sky.star_views().any(|star| star.id().0 == 92)); // HR 92 has no data
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
        let sky = crate::sky::create_sky_from_catalog(&catalog).unwrap();
        assert_eq!(sky.constellations().len(), 1);
        let [a, b] = sky.constellations()[0].segments[0];
        assert_eq!((sky.star_view(a).id().0, sky.star_view(b).id().0), (10, 20));
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations().len(), 88);
        let [a, b] = sky.constellations()[19].segments[0];
        assert_eq!((sky.star_view(a).id().0, sky.star_view(b).id().0), (4785, 4915));
    }
}
