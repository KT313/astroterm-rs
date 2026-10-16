//! Filtering data prepared with a merged image, never during a pixel lookup.
use crate::model::{Texel, TileFilterSummary};
use crate::constants::{TILE_EDGE_TEXELS, TILE_FILTER_BLOCK_EDGE};

pub(super) fn build_filter_summary(texels: &[Texel]) -> TileFilterSummary {
    if texels.is_empty() { return TileFilterSummary::default(); }
    assert_eq!(texels.len(), TILE_EDGE_TEXELS * TILE_EDGE_TEXELS);
    assert!(TILE_FILTER_BLOCK_EDGE > 0 && TILE_EDGE_TEXELS.is_multiple_of(TILE_FILTER_BLOCK_EDGE));

    // Sum the stored channels without changing their premultiplication or applying the display tone map.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_keep_all_four_stored_channels() {
        let mut texels = vec![Texel::default(); TILE_EDGE_TEXELS * TILE_EDGE_TEXELS];
        texels[0] = Texel { rgb: [0.25, 0.0, 0.125], opacity: 0.5 };
        texels[TILE_FILTER_BLOCK_EDGE] = Texel { rgb: [0.5, 0.25, 0.125], opacity: 1.0 };
        let summary = build_filter_summary(&texels);
        assert_eq!(summary.rgba_sum, [0.75, 0.25, 0.25, 1.5]);
        assert_eq!(summary.block_sums[0], [0.25, 0.0, 0.125, 0.5]);
        assert_eq!(summary.block_sums[1], [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(build_filter_summary(&[]).rgba_sum, [0.0; 4]);
    }
}
