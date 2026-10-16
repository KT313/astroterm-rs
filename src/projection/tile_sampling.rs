//! The colour of one merged tile under a screen pixel. The pixel's footprint in texture space is an axis-aligned
//! box of at least one texel; each texel contributes its overlap divided by the full box area. Space outside
//! the tile is transparent, so partially covered pixels retain only their covered share. Where the
//! tile shrinks on screen that is a box filter over a few texels, where it grows it is the bilinear mix of the
//! four texels around the position. Boxes that touch only empty texels are rejected through the tile's lit-row
//! bits before any arithmetic, which is what makes sparse tiles cheap. Nothing here is retained between frames.
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::Texel;

/// Frame-local light masks for the tile most recently supplied to prepare. The supplied rows are the key;
/// every prepare call replaces all derived data, and the owner drops the scratch at frame end.
pub(super) struct TileSampler {
    edge: usize,
    lit: [u64; TILE_EDGE_TEXELS],           // one bit per texel per row: which texels hold light
    lit_columns: u64,                       // the rows combined: which columns hold light anywhere
    lit_rows: u64,                          // one bit per row that holds light anywhere
    near: [u64; TILE_EDGE_TEXELS],          // lit bits spread two texels each way: whether light lies within a small box
}

impl TileSampler {
    pub(super) fn new(edge: usize) -> Self {
        assert!(edge <= TILE_EDGE_TEXELS && edge <= 64, "lit rows are one word per tile row");
        Self { edge, lit: [0; TILE_EDGE_TEXELS], lit_columns: 0, lit_rows: 0, near: [0; TILE_EDGE_TEXELS] }
    }

    /// Adopt a tile's lit rows, replacing the preceding tile's masks completely.
    pub(super) fn prepare(&mut self, lit_rows: &[u64]) {
        assert_eq!(lit_rows.len(), self.edge);
        self.lit[..self.edge].copy_from_slice(lit_rows);
        self.lit_columns = lit_rows.iter().fold(0, |all, &row| all | row);
        self.lit_rows = lit_rows.iter().enumerate().fold(0, |any, (index, &row)| if row != 0 { any | 1 << index } else { any });
        for (index, near) in self.near[..self.edge].iter_mut().enumerate() {
            let spread = |bits: u64| bits | bits << 1 | bits >> 1 | bits << 2 | bits >> 2;
            *near = lit_rows[index.saturating_sub(2)..(index + 3).min(self.edge)].iter().fold(0, |all, &row| all | spread(row));
        }
    }

    /// Whether any texel within two of the texel under `position` holds light: a quick rejection for filter
    /// boxes of up to three texels, whose texels all lie within that reach.
    #[inline]
    pub(super) fn may_hold_light_near(&self, position: [f64; 2]) -> bool {
        let column = ((position[0] + 0.5) as usize).min(self.edge - 1);
        let row = ((position[1] + 0.5) as usize).min(self.edge - 1);
        self.near[row] >> column & 1 != 0
    }

    /// Whether any texel in tile rows `first..=last` holds light.
    #[inline]
    pub(super) fn any_light_in_rows(&self, first: usize, last: usize) -> bool {
        let last = last.min(self.edge - 1);
        first <= last && (u64::MAX >> (63 - (last - first))) << first & self.lit_rows != 0
    }

    fn has_lit_texels(&self, first_column: usize, last_column: usize, first_row: usize, last_row: usize) -> bool {
        // Check broad column/row masks first, then only the rows that the footprint overlaps.
        let columns = (u64::MAX >> (63 - (last_column - first_column))) << first_column;
        if columns & self.lit_columns == 0 { return false; }
        let rows = (u64::MAX >> (63 - (last_row - first_row))) << first_row;
        if rows & self.lit_rows == 0 { return false; }
        self.lit[first_row..=last_row].iter().any(|bits| bits & columns != 0)
    }

    /// Average the box of `footprint` texels (at least one) centred at `position`, in texture coordinates from
    /// 0 to edge, or `None` when every texel under it is empty. Space outside the tile is transparent:
    /// clip the integration bounds, but divide by the full footprint area to retain partial coverage.
    #[inline]
    pub(super) fn sample(&self, texels: &[Texel], position: [f64; 2], footprint: [f64; 2]) -> Option<[f32; 4]> {
        let edge = self.edge as f64;
        let (half_width, half_height) = (footprint[0].max(1.0) * 0.5, footprint[1].max(1.0) * 0.5);
        let [cu, cv] = position;
        let (left, right) = ((cu - half_width).max(0.0), (cu + half_width).min(edge));
        let (top, bottom) = ((cv - half_height).max(0.0), (cv + half_height).min(edge));
        if right <= left || bottom <= top { return None; }

        // Reject the box when none of the texels it touches holds light: columns, then rows, then the exact bits.
        let last = self.edge - 1;
        let (first_column, last_column) = (left as usize, (right.ceil() as usize - 1).min(last));
        let (first_row, last_row) = (top as usize, (bottom.ceil() as usize - 1).min(last));
        if !self.has_lit_texels(first_column, last_column, first_row, last_row) { return None; }

        let full_area = (2.0 * half_width) * (2.0 * half_height);
        let rgba = average(texels, self.edge, [left, right], [top, bottom], (first_column, last_column), (first_row, last_row), full_area);
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("whole tile filter output", rgba.map(f64::from), 1.0);
        Some(rgba)
    }
}

/// Integrate only overlapping texels, weighted by the full filter area including transparent space.
fn average(texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], (first_column, last_column): (usize, usize), (first_row, last_row): (usize, usize), area: f64) -> [f32; 4] {
    // Only the first and last columns can have fractional widths. Interior columns share one row weight.
    let first_width = right.min(first_column as f64 + 1.0) - left;
    let last_width = right - last_column as f64;
    let mut rgba = [0.0_f64; 4];
    for row in first_row..=last_row {
        let height = bottom.min(row as f64 + 1.0) - top.max(row as f64);
        let offset = row * edge;
        add_weighted_texel(&mut rgba, texels[offset + first_column], first_width * height / area);
        if first_column == last_column { continue; }

        // Preserve the original accumulation order, avoiding a division for every full-width texel.
        let weight = height / area;
        for column in first_column + 1..last_column {
            add_weighted_texel(&mut rgba, texels[offset + column], weight);
        }
        add_weighted_texel(&mut rgba, texels[offset + last_column], last_width * height / area);
    }
    rgba.map(|value| value as f32)
}

#[inline]
fn add_weighted_texel(sum: &mut [f64; 4], texel: Texel, weight: f64) {
    // RGB and opacity use the same area weight, with f64 accumulation to keep dense averages bounded.
    for (sum, value) in sum.iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) {
        *sum += f64::from(value) * weight;
    }
}

/// The lit-row bits of a tile, as the region cache computes them.
#[cfg(test)]
pub(super) fn lit_rows_of(texels: &[Texel], edge: usize) -> Vec<u64> {
    texels.chunks_exact(edge).map(|row| row.iter().enumerate().filter(|(_, texel)| !texel.is_empty()).fold(0_u64, |bits, (x, _)| bits | 1 << x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The box average computed over every texel of the tile, for comparison.
    fn direct(texels: &[Texel], edge: usize, position: [f64; 2], footprint: [f64; 2]) -> [f64; 4] {
        let [x, y] = position;
        let [width, height] = footprint;
        let (left, right) = (x - width.max(1.0) / 2.0, x + width.max(1.0) / 2.0);
        let (top, bottom) = (y - height.max(1.0) / 2.0, y + height.max(1.0) / 2.0);
        let mut expected = [0.0; 4];
        for row in 0..edge {
            for column in 0..edge {
                let area = (right.min(column as f64 + 1.0) - left.max(column as f64)).max(0.0) * (bottom.min(row as f64 + 1.0) - top.max(row as f64)).max(0.0);
                let texel = texels[row * edge + column];
                for (sum, value) in expected.iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) {
                    *sum += value as f64 * area / ((right - left) * (bottom - top));
                }
            }
        }
        expected
    }
    fn gradient() -> Vec<Texel> {
        (0..16).map(|index| { let value = (index + 1) as f32 / 17.0; Texel { rgb: [value, value * 0.5, 0.0], opacity: value } }).collect()
    }
    fn sampler(texels: &[Texel]) -> TileSampler {
        let mut sampler = TileSampler::new(4);
        sampler.prepare(&lit_rows_of(texels, 4));
        sampler
    }

    #[test]
    fn reduction_includes_a_bright_texel_away_from_the_sample_center() {
        let mut texels = vec![Texel::default(); 16];
        texels[0] = Texel { rgb: [1.0, 0.5, 0.25], opacity: 1.0 };
        let sampler = sampler(&texels);
        assert_eq!(sampler.sample(&texels, [2.0, 2.0], [4.0, 4.0]), Some([1.0 / 16.0, 0.5 / 16.0, 0.25 / 16.0, 1.0 / 16.0]));
    }

    #[test]
    fn partial_footprints_keep_transparent_space_in_their_area() {
        let texels = vec![Texel { rgb: [0.5, 0.25, 0.125], opacity: 1.0 }; 16];
        let sampler = sampler(&texels);
        for (position, footprint, coverage) in [
            ([2.0, 2.0], [8.0, 8.0], 0.25),       // entire 4 × 4 tile inside an 8 × 8 footprint
            ([-0.5, 2.0], [2.0, 2.0], 0.25),     // centre is outside, but a quarter of the footprint overlaps
            ([0.0, 0.0], [1.0, 1.0], 0.25),      // one quarter of the box touches the tile corner
            ([2.0, 2.0], [1.0, 1.0], 1.0),       // interior pixels keep the existing value
        ] {
            assert_eq!(sampler.sample(&texels, position, footprint), Some([0.5 * coverage, 0.25 * coverage, 0.125 * coverage, coverage]));
        }
        assert_eq!(sampler.sample(&texels, [-1.0, 2.0], [2.0, 2.0]), None); // touching without overlap contributes nothing
    }

    #[test]
    fn dense_reductions_stay_within_the_opacity_range() {
        let texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; 64 * 64];
        let mut sampler = TileSampler::new(64);
        sampler.prepare(&[u64::MAX; 64]);
        assert_eq!(sampler.sample(&texels, [32.0, 32.0], [40.0, 40.0]), Some([1.0; 4]));
    }

    #[test]
    fn boxes_of_any_size_match_direct_texel_area_averages_including_clipped_ones() {
        let texels = gradient();
        let sampler = sampler(&texels);
        for (position, footprint) in [([1.7, 2.2], [2.3, 1.8]), ([0.0, 4.0], [1.0, 1.0]), ([2.0, 2.0], [0.1, 0.1]), ([1.3, 2.6], [1.0, 1.0]),
            ([3.9, 0.2], [0.5, 0.5]), ([-2.0, 1.5], [1.0, 1.0]), ([4.0, 4.0], [3.0, 3.0]), ([2.0, 2.0], [40.0, 40.0])] {
            let actual = sampler.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
            for (actual, expected) in actual.into_iter().zip(direct(&texels, 4, position, footprint)) {
                assert!((f64::from(actual) - expected).abs() < 1e-6, "{position:?} {footprint:?}: {actual} != {expected}");
            }
        }
    }

    #[test]
    fn the_near_mask_covers_two_texels_around_every_lit_one() {
        let mut texels = vec![Texel::default(); 16];
        texels[5] = Texel { rgb: [0.5, 0.5, 0.5], opacity: 0.5 };                            // row 1, column 1
        let sampler = sampler(&texels);
        for row in 0..4 {
            for column in 0..4 {
                let near = (row - 1_i32).abs() <= 2 && (column - 1_i32).abs() <= 2;
                assert_eq!(sampler.may_hold_light_near([column as f64, row as f64]), near, "{column} {row}");
            }
        }
        assert!(sampler.may_hold_light_near([3.49, 3.49]) && sampler.may_hold_light_near([-0.5, -0.5]));
        assert!(!self::sampler(&vec![Texel::default(); 16]).may_hold_light_near([1.0, 1.0]));
    }

    #[test]
    fn boxes_over_empty_texels_are_skipped_by_the_lit_bits() {
        let mut texels = vec![Texel::default(); 16];
        texels[5] = Texel { rgb: [0.5, 0.5, 0.5], opacity: 0.5 };                            // row 1, column 1
        let sampler = sampler(&texels);
        assert_eq!(sampler.sample(&texels, [3.5, 3.5], [1.0, 1.0]), None);                   // a box in the dark corner
        assert_eq!(sampler.sample(&texels, [3.0, 1.5], [2.0, 1.0]), None);                   // columns 2 and 3 of row 1
        assert_eq!(sampler.sample(&texels, [1.5, 3.0], [1.0, 2.0]), None);                   // rows 2 and 3 of column 1
        assert!(sampler.sample(&texels, [2.4, 1.5], [1.0, 1.0]).is_some());                  // columns 1 and 2
        assert!(sampler.sample(&texels, [2.0, 2.0], [4.0, 4.0]).is_some());
        assert_eq!(sampler.sample(&texels, [4.0, 4.0], [0.2, 0.2]), None);                   // the clipped corner box holds texel 15 only
        let empty = vec![Texel::default(); 16];
        assert_eq!(self::sampler(&empty).sample(&empty, [2.0, 2.0], [4.0, 4.0]), None);
    }
    #[test]
    fn row_weight_reuse_matches_direct_averages_even_at_tiny_borders() {
        // Exercise full and fractional rectangles, including nearly zero overlap with an illuminated texel.
        let edge = 64;
        let mut texels: Vec<_> = (0..edge * edge).map(|index| {
            let alpha = if index % 7 == 0 { 0.0 } else { (index % 19 + 1) as f32 / 21.0 };
            Texel { rgb: [alpha * 0.2, alpha * 0.7, alpha], opacity: alpha }
        }).collect();
        let mut sampler = TileSampler::new(edge);
        sampler.prepare(&lit_rows_of(&texels, edge));
        for position in [[32.0, 32.0], [0.0, 0.0], [62.99, 0.25], [31.3, 24.7]] {
            for footprint in [[64.0, 64.0], [7.5, 13.2], [1.0, 0.1], [12.0, 5.0]] {
                let expected = direct(&texels, edge, position, footprint);
                let actual = sampler.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
                for axis in 0..4 { assert!((f64::from(actual[axis]) - expected[axis]).abs() < 1e-7); }
            }
        }

        // Bright data before the queried rectangle must not leak into an empty interior through cancellation.
        for row in 16..48 { for column in 16..48 { texels[row * edge + column] = Texel::default(); } }
        sampler.prepare(&lit_rows_of(&texels, edge));
        assert_eq!(sampler.sample(&texels, [32.0, 32.0], [20.0, 20.0]), None);
        for epsilon in [0.0, 1e-12, 1e-9, 1e-6] {
            let position = [32.0, 32.0];
            let footprint = [32.0 + epsilon, 32.0 + epsilon];
            let expected = direct(&texels, edge, position, footprint);
            let actual = sampler.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
            for axis in 0..4 {
                let tolerance = 1e-6 * expected[axis].abs().max(1e-30);
                assert!((f64::from(actual[axis]) - expected[axis]).abs() <= tolerance, "{epsilon}: {actual:?} != {expected:?}");
            }
        }

        // Replacing a dense tile with an empty one invalidates all light masks.
        texels.fill(Texel::default());
        sampler.prepare(&lit_rows_of(&texels, edge));
        assert_eq!(sampler.sample(&texels, [32.0, 32.0], [64.0; 2]), None);
    }

}
