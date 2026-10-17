//! The SSE2 merge and lit-row scan against their scalar oracles, and the lit-only filter summary against a full
//! scan of every texel. All comparisons are bit for bit.
use super::*;
use crate::constants::{TILE_EDGE_TEXELS, TILE_FILTER_BLOCK_EDGE};
use crate::model::TileFilterSummary;
#[cfg(target_arch = "x86_64")]
use crate::regions::merge_sse2;

const TEXELS: usize = TILE_EDGE_TEXELS * TILE_EDGE_TEXELS;

/// The filter summary as it was before it used the lit rows: every texel in row-major order.
fn build_filter_summary_reference(texels: &[Texel]) -> TileFilterSummary {
    let blocks_per_edge = TILE_EDGE_TEXELS / TILE_FILTER_BLOCK_EDGE;
    let mut block_sums = vec![[0.0_f64; 4]; blocks_per_edge * blocks_per_edge];
    let mut rgba_sum = [0.0_f64; 4];
    for (index, texel) in texels.iter().enumerate() {
        let block_row = (index / TILE_EDGE_TEXELS) / TILE_FILTER_BLOCK_EDGE;
        let block_column = (index % TILE_EDGE_TEXELS) / TILE_FILTER_BLOCK_EDGE;
        let block = &mut block_sums[block_row * blocks_per_edge + block_column];
        for (channel, value) in texel.rgb.into_iter().chain([texel.opacity]).enumerate() {
            let value = f64::from(value);
            rgba_sum[channel] += value;
            block[channel] += value;
        }
    }
    TileFilterSummary { rgba_sum, block_sums }
}

/// A seeded group image: about `lit_fraction` of its texels hold a premultiplied colour, the rest are empty
/// (some of them with opacity -0.0, which also counts as empty).
fn random_texels(count: usize, lit_fraction: f64, seed: &mut u32) -> Vec<Texel> {
    let mut next = || { *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); f64::from(*seed >> 8) / 16777215.0 };
    (0..count).map(|_| {
        if next() >= lit_fraction { return if next() < 0.1 { Texel { rgb: [0.0; 3], opacity: -0.0 } } else { Texel::default() }; }
        let opacity = (next() as f32).max(f32::MIN_POSITIVE);
        Texel { rgb: [next() as f32 * opacity, next() as f32 * opacity, next() as f32 * opacity], opacity }
    }).collect()
}

fn texel_bits(texels: &[Texel]) -> Vec<[u32; 4]> {
    texels.iter().map(|texel| [texel.rgb[0].to_bits(), texel.rgb[1].to_bits(), texel.rgb[2].to_bits(), texel.opacity.to_bits()]).collect()
}

fn summary_bits(summary: &TileFilterSummary) -> (Vec<u64>, Vec<[u64; 4]>) {
    (summary.rgba_sum.map(f64::to_bits).to_vec(), summary.block_sums.iter().map(|block| block.map(f64::to_bits)).collect())
}

fn scan_scalar(texels: &[Texel]) -> Vec<u64> {
    let mut lit_rows = Vec::new();
    scan_lit_rows_scalar(texels, &mut lit_rows);
    lit_rows
}

const LIT_FRACTIONS: [f64; 5] = [0.0, 0.002, 0.06, 0.17, 1.0];                             // empty, fast group, medium group, slow group, full

#[cfg(target_arch = "x86_64")]
#[test]
fn simd_merge_matches_scalar_bits_for_sparse_and_dense_tiles() {
    let mut seed = 29_u32;
    for length in [TEXELS, 4, 5, 7, 3, 1, 0] {                                              // whole groups and every tail length
        for base_fraction in LIT_FRACTIONS {
            for next_fraction in LIT_FRACTIONS {
                let base = random_texels(length, base_fraction, &mut seed);
                let next = random_texels(length, next_fraction, &mut seed);
                let (mut simd, mut scalar) = (base.clone(), base);
                merge_sse2::merge_group_texels(&mut simd, &next);
                merge_group_texels_scalar(&mut scalar, &next);
                assert_eq!(texel_bits(&simd), texel_bits(&scalar), "length {length}, lit {base_fraction} under {next_fraction}");
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
#[should_panic(expected = "a group image must match the merged image")]
fn simd_merge_rejects_images_of_different_sizes() {
    merge_sse2::merge_group_texels(&mut [Texel::default(); 8], &[Texel::default(); 4]);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn simd_lit_scan_matches_scalar_for_every_column_and_special_opacities() {
    // A single lit texel at every position of the tile; the reused storage is cleared first.
    let mut simd = vec![u64::MAX; 3];
    let mut texels = vec![Texel::default(); TEXELS];
    for position in 0..TEXELS {
        texels[position] = Texel { rgb: [0.25; 3], opacity: 0.5 };
        merge_sse2::scan_lit_rows(&texels, &mut simd);
        assert_eq!(simd, scan_scalar(&texels), "lit texel {position}");
        assert_eq!(simd[position / TILE_EDGE_TEXELS], 1 << (position % TILE_EDGE_TEXELS));
        texels[position] = Texel::default();
    }

    // -0.0 is empty; NaN and the smallest subnormal hold light, in every lane of a group of four.
    for (opacity, lit) in [(-0.0, false), (f32::NAN, true), (f32::from_bits(1), true), (f32::INFINITY, true)] {
        for column in 0..TILE_EDGE_TEXELS {
            texels[TILE_EDGE_TEXELS + column] = Texel { rgb: [0.0; 3], opacity };
            merge_sse2::scan_lit_rows(&texels, &mut simd);
            assert_eq!(simd, scan_scalar(&texels), "opacity {opacity:?} at column {column}");
            assert_eq!(simd[1], u64::from(lit) << column);
            texels[TILE_EDGE_TEXELS + column] = Texel::default();
        }
    }
    merge_sse2::scan_lit_rows(&[], &mut simd);
    assert!(simd.is_empty());
}

#[test]
fn lit_only_summary_matches_full_scan_bits() {
    let mut seed = 31_u32;
    for base_fraction in LIT_FRACTIONS {
        for next_fraction in LIT_FRACTIONS {
            let mut merged = random_texels(TEXELS, base_fraction, &mut seed);
            merge_group_texels_scalar(&mut merged, &random_texels(TEXELS, next_fraction, &mut seed));
            let summary = build_filter_summary(&merged, &scan_scalar(&merged));
            assert_eq!(summary_bits(&summary), summary_bits(&build_filter_summary_reference(&merged)), "lit {base_fraction} under {next_fraction}");
        }
    }
}

#[test]
fn merged_regions_of_the_whole_bsc5_sky_match_the_scalar_reference() {
    // Merge every region of the embedded catalog through the real refresh.
    let quiet = || crate::progress::Report::quiet();
    let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut quiet()).unwrap(), &mut quiet()).unwrap();
    let mut data = crate::sky::build_sky_tables(&prepared, &mut quiet()).unwrap();
    let every_region: Vec<usize> = (0..data.star_regions.len()).collect();
    crate::regions::refresh_regions(
        &mut data.star_regions, &data.constellation_stars,
        &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
        &mut data.star_opacities, &mut data.tile_bytes,
        &crate::selection::Selected::of_regions(every_region), crate::formulas::utc_jd_to_time_ms(crate::formulas::J2000).unwrap(), 180.0, 8.0,
        &mut crate::time::StepTimes::default(),
    );

    // Rebuild each region image with the scalar functions and the full-scan summary.
    let mut merged_groups = 0;
    for region in &data.star_regions {
        let mut parts = region.speed_groups.iter().map(|group| group.tile.texels()).filter(|texels| !texels.is_empty());
        let Some(first) = parts.next() else { assert!(region.merged_tile.is_empty()); continue };
        let mut texels = first.to_vec();
        for part in parts { merge_group_texels_scalar(&mut texels, part); merged_groups += 1; }
        let lit_rows = scan_scalar(&texels);
        let image = &region.merged_tile;
        assert_eq!(texel_bits(image.texels()), texel_bits(&texels));
        assert_eq!(image.lit_rows(), lit_rows);
        assert_eq!(image.content_radius.map(f64::to_bits), Some(calculate_content_radius(&lit_rows).to_bits()));
        assert_eq!(summary_bits(image.filter_summary.as_ref().unwrap()), summary_bits(&build_filter_summary_reference(&texels)));
    }
    assert!(merged_groups > 0, "the catalog should give some region more than one group image");
}

#[cfg(feature = "pixel-debug")]
#[test]
#[should_panic(expected = "pixel-debug: unlit texel holds colour")]
fn checked_summary_rejects_colour_in_an_unlit_texel() {
    let mut texels = vec![Texel::default(); TEXELS];
    texels[TILE_EDGE_TEXELS + 3] = Texel { rgb: [0.5, 0.0, 0.0], opacity: 0.0 };
    build_filter_summary(&texels, &scan_scalar(&texels));
}
