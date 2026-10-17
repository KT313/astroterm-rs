//! Filtering data prepared with a merged image, never during a pixel lookup.
use crate::model::{Texel, TileFilterSummary};
use crate::constants::{TILE_EDGE_TEXELS, TILE_FILTER_BLOCK_EDGE};

/// Sum the tile's channels per block and in total, visiting only the texels set in `lit_rows` (one word per tile
/// row). Unlit texels hold zero in every channel, and adding zero never changes a sum that started at +0.0, so
/// the result has the bits of a full scan in row-major order.
pub(super) fn build_filter_summary(texels: &[Texel], lit_rows: &[u64]) -> TileFilterSummary {
    if texels.is_empty() { return TileFilterSummary::default(); }
    assert_eq!(texels.len(), TILE_EDGE_TEXELS * TILE_EDGE_TEXELS);
    assert_eq!(lit_rows.len(), TILE_EDGE_TEXELS, "a filter summary needs one lit-row word per tile row");
    assert!(TILE_FILTER_BLOCK_EDGE > 0 && TILE_EDGE_TEXELS.is_multiple_of(TILE_FILTER_BLOCK_EDGE));
    #[cfg(feature = "pixel-debug")]
    check_unlit_texels_are_zero(texels, lit_rows);

    // Sum the stored channels without changing their premultiplication or applying the display tone map.
    let blocks_per_edge = TILE_EDGE_TEXELS / TILE_FILTER_BLOCK_EDGE;
    let mut block_sums = vec![[0.0_f64; 4]; blocks_per_edge * blocks_per_edge];
    let mut rgba_sum = [0.0_f64; 4];
    for (row, &row_bits) in lit_rows.iter().enumerate() {
        let mut lit_columns = row_bits;
        while lit_columns != 0 {                                                            // ascending column: row-major order kept
            let column = lit_columns.trailing_zeros() as usize;
            lit_columns &= lit_columns - 1;
            let texel = texels[row * TILE_EDGE_TEXELS + column];
            let block = &mut block_sums[row / TILE_FILTER_BLOCK_EDGE * blocks_per_edge + column / TILE_FILTER_BLOCK_EDGE];
            for (channel, value) in texel.rgb.into_iter().chain([texel.opacity]).enumerate() {
                let value = f64::from(value);
                rgba_sum[channel] += value;
                block[channel] += value;
            }
        }
    }
    TileFilterSummary { rgba_sum, block_sums }
}

/// Skipping unlit texels is exact only while they hold no colour; merging never leaves any there.
#[cfg(feature = "pixel-debug")]
fn check_unlit_texels_are_zero(texels: &[Texel], lit_rows: &[u64]) {
    for (row, (texels, &row_bits)) in texels.chunks_exact(TILE_EDGE_TEXELS).zip(lit_rows).enumerate() {
        for (column, texel) in texels.iter().enumerate() {
            if row_bits & 1 << column != 0 { continue; }
            assert!(texel.rgb.into_iter().chain([texel.opacity]).all(|value| value == 0.0),
                "pixel-debug: unlit texel holds colour at row {row}, column {column}: {texel:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_keep_all_four_stored_channels() {
        let mut texels = vec![Texel::default(); TILE_EDGE_TEXELS * TILE_EDGE_TEXELS];
        texels[0] = Texel { rgb: [0.25, 0.0, 0.125], opacity: 0.5 };
        texels[TILE_FILTER_BLOCK_EDGE] = Texel { rgb: [0.5, 0.25, 0.125], opacity: 1.0 };
        let mut lit_rows = vec![0_u64; TILE_EDGE_TEXELS];
        lit_rows[0] = 1 | 1 << TILE_FILTER_BLOCK_EDGE;
        let summary = build_filter_summary(&texels, &lit_rows);
        assert_eq!(summary.rgba_sum, [0.75, 0.25, 0.25, 1.5]);
        assert_eq!(summary.block_sums[0], [0.25, 0.0, 0.125, 0.5]);
        assert_eq!(summary.block_sums[1], [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(build_filter_summary(&[], &[]).rgba_sum, [0.0; 4]);
    }
}
