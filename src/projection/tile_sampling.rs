//! The colour of one merged tile under a screen pixel. The pixel's footprint in texture space is an axis-aligned
//! box of at least one texel; each texel contributes its overlap divided by the full box area. Space outside
//! the tile is transparent, so partially covered pixels retain only their covered share. Where the
//! tile shrinks on screen that is a box filter over a few texels, where it grows it is the bilinear mix of the
//! four texels around the position. General boxes use lit-row bits to reject empty areas; the optional
//! interior unit-box shortcut checks the opacity of its four weighted texels. Whole-tile and block integrals are
//! borrowed from the merged image; partially covered borders still use its original texels.
use crate::constants::{TILE_EDGE_TEXELS, TILE_FILTER_BLOCK_EDGE, TILE_FILTER_MIN_TEXELS};
use crate::model::{Texel, TileFilterSummary};
use super::bilinear_sampling::sample_bilinear_interior;
use super::options::ProjectionOptions;

/// Frame-local sampling data for the most recently prepared image. Every prepare replaces the masks and
/// borrowed summary together, even when only colours changed. Summary validity belongs to the owning
/// RegionTileCache's image key; this scratch has no independent lifetime beyond the current frame.
pub(super) struct TileSampler<'a> {
    bilinear_fast_path: bool,               // immutable for this task; the generic filter remains the fallback
    summary: Option<&'a TileFilterSummary>, // belongs to the same image as the copied light masks
    edge: usize,
    lit: [u64; TILE_EDGE_TEXELS],           // one bit per texel per row: which texels hold light
    lit_columns: u64,                       // the rows combined: which columns hold light anywhere
    lit_rows: u64,                          // one bit per row that holds light anywhere
    near: [u64; TILE_EDGE_TEXELS],          // lit bits spread two texels each way: whether light lies within a small box
}

impl<'a> TileSampler<'a> {
    pub(super) fn new(edge: usize, options: &ProjectionOptions) -> Self {
        assert!(edge <= TILE_EDGE_TEXELS && edge <= 64, "lit rows are one word per tile row");
        Self { bilinear_fast_path: options.bilinear_fast_path, summary: None, edge, lit: [0; TILE_EDGE_TEXELS], lit_columns: 0, lit_rows: 0, near: [0; TILE_EDGE_TEXELS] }
    }

    /// Adopt a tile's lit rows, replacing the preceding tile's masks completely.
    pub(super) fn prepare(&mut self, lit_rows: &[u64], summary: Option<&'a TileFilterSummary>) {
        assert_eq!(lit_rows.len(), self.edge);
        self.summary = summary;
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
        // Unit footprints inside the tile need only four interpolation weights. Keep edges and wider boxes exact.
        let last_center = self.edge as f64 - 0.5;
        if self.bilinear_fast_path && footprint[0] <= 1.0 && footprint[1] <= 1.0
            && position[0] >= 0.5 && position[0] < last_center
            && position[1] >= 0.5 && position[1] < last_center
        {
            return sample_bilinear_interior(texels, self.edge, position);
        }

        self.sample_box(texels, position, footprint)
    }

    /// General box integration, also used for tile boundaries and when the fast path is disabled.
    #[inline]
    fn sample_box(&self, texels: &[Texel], position: [f64; 2], footprint: [f64; 2]) -> Option<[f32; 4]> {
        let edge = self.edge as f64;
        let (half_width, half_height) = (footprint[0].max(1.0) * 0.5, footprint[1].max(1.0) * 0.5);
        let [cu, cv] = position;
        let (left, right) = ((cu - half_width).max(0.0), (cu + half_width).min(edge));
        let (top, bottom) = ((cv - half_height).max(0.0), (cv + half_height).min(edge));
        if right <= left || bottom <= top { return None; }

        // Reuse the image's total only when all four tile edges lie inside the footprint.
        if left == 0.0 && right == edge && top == 0.0 && bottom == edge {
            if self.lit_rows == 0 { return None; }
            if let Some(summary) = self.summary {
                let full_area = (2.0 * half_width) * (2.0 * half_height);
                let rgba = summary.rgba_sum.map(|sum| (sum / full_area) as f32);
                #[cfg(feature = "pixel-debug")]
                crate::pixel_debug::check_pixel("whole tile summary output", rgba.map(f64::from), 1.0);
                return Some(rgba);
            }
        }

        // Reject the box when none of the texels it touches holds light: columns, then rows, then the exact bits.
        let last = self.edge - 1;
        let (first_column, last_column) = (left as usize, (right.ceil() as usize - 1).min(last));
        let (first_row, last_row) = (top as usize, (bottom.ceil() as usize - 1).min(last));
        if !self.has_lit_texels(first_column, last_column, first_row, last_row) { return None; }

        // Small boxes stay on the direct path; larger ones may reuse complete blocks from the image.
        let full_area = (2.0 * half_width) * (2.0 * half_height);
        let touched_texels = (last_column - first_column + 1) * (last_row - first_row + 1);
        let mut summarized = None;
        if touched_texels >= TILE_FILTER_MIN_TEXELS && let Some(summary) = self.summary {
            summarized = average_with_blocks(summary, texels, self.edge, [left, right], [top, bottom], full_area);
        }
        let rgba = match summarized {
            Some(rgba) => rgba,
            None => average(texels, self.edge, [left, right], [top, bottom], (first_column, last_column), (first_row, last_row), full_area),
        };
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("whole tile filter output", rgba.map(f64::from), 1.0);
        Some(rgba)
    }
}

/// Integrate only overlapping texels, weighted by the full filter area including transparent space.
fn average(texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], (first_column, last_column): (usize, usize), (first_row, last_row): (usize, usize), area: f64) -> [f32; 4] {
    let mut rgba = [0.0_f64; 4];
    accumulate_texels(&mut rgba, texels, edge, [left, right], [top, bottom], (first_column, last_column), (first_row, last_row), area);
    rgba.map(|value| value as f32)
}

/// Add a rectangle's contribution in f64; callers may combine disjoint rectangles before converting to f32.
#[inline]
#[allow(clippy::too_many_arguments)]
fn accumulate_texels(rgba: &mut [f64; 4], texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], (first_column, last_column): (usize, usize), (first_row, last_row): (usize, usize), area: f64) {
    // Only the first and last columns can have fractional widths. Interior columns share one row weight.
    let first_width = right.min(first_column as f64 + 1.0) - left;
    let last_width = right - last_column as f64;
    for row in first_row..=last_row {
        let height = bottom.min(row as f64 + 1.0) - top.max(row as f64);
        let offset = row * edge;
        add_weighted_texel(rgba, texels[offset + first_column], first_width * height / area);
        if first_column == last_column { continue; }

        // Preserve the original accumulation order, avoiding a division for every full-width texel.
        let weight = height / area;
        for column in first_column + 1..last_column {
            add_weighted_texel(rgba, texels[offset + column], weight);
        }
        add_weighted_texel(rgba, texels[offset + last_column], last_width * height / area);
    }
}

fn average_with_blocks(summary: &TileFilterSummary, texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], area: f64) -> Option<[f32; 4]> {
    // Use summaries only for complete blocks lying wholly inside this box.
    let block_edge = TILE_FILTER_BLOCK_EDGE;
    if block_edge == 0 || !edge.is_multiple_of(block_edge) { return None; }
    let blocks_per_edge = edge / block_edge;
    if summary.block_sums.len() != blocks_per_edge * blocks_per_edge { return None; }
    let first_column = (left / block_edge as f64).ceil() as usize;
    let end_column = (right / block_edge as f64).floor() as usize;
    let first_row = (top / block_edge as f64).ceil() as usize;
    let end_row = (bottom / block_edge as f64).floor() as usize;
    if first_column >= end_column || first_row >= end_row { return None; }

    // Complete interior blocks contribute their stored integrals, without subtracting large prefixes.
    let mut rgba = [0.0_f64; 4];
    for row in first_row..end_row {
        for column in first_column..end_column {
            for (sum, value) in rgba.iter_mut().zip(summary.block_sums[row * blocks_per_edge + column]) {
                *sum += value / area;
            }
        }
    }

    // Four disjoint border strips retain exact fractional coverage and tiny contributions.
    let inner_left = (first_column * block_edge) as f64;
    let inner_right = (end_column * block_edge) as f64;
    let inner_top = (first_row * block_edge) as f64;
    let inner_bottom = (end_row * block_edge) as f64;
    accumulate_border(&mut rgba, texels, edge, [left, right], [top, inner_top], area);
    accumulate_border(&mut rgba, texels, edge, [left, right], [inner_bottom, bottom], area);
    accumulate_border(&mut rgba, texels, edge, [left, inner_left], [inner_top, inner_bottom], area);
    accumulate_border(&mut rgba, texels, edge, [inner_right, right], [inner_top, inner_bottom], area);
    Some(rgba.map(|value| value as f32))
}

fn accumulate_border(rgba: &mut [f64; 4], texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], area: f64) {
    if right <= left || bottom <= top { return; }
    let columns = (left as usize, (right.ceil() as usize - 1).min(edge - 1));
    let rows = (top as usize, (bottom.ceil() as usize - 1).min(edge - 1));
    accumulate_texels(rgba, texels, edge, [left, right], [top, bottom], columns, rows, area);
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
    fn sampler(texels: &[Texel]) -> TileSampler<'static> {
        let mut sampler = TileSampler::new(4, &Default::default());
        sampler.prepare(&lit_rows_of(texels, 4), None);
        sampler
    }

    #[test]
    fn bilinear_preserves_box_support_at_texel_boundaries_and_tiny_weights() {
        // Probe both sides of texel centres and tile edges, including immediately adjacent f64 values.
        let mut positions = Vec::new();
        for coordinate in [-0.5_f64, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5] {
            positions.extend([coordinate.next_down(), coordinate, coordinate.next_up()]);
        }
        let mut fast = TileSampler::new(4, &ProjectionOptions { bilinear_fast_path: true });
        let mut exact = TileSampler::new(4, &ProjectionOptions { bilinear_fast_path: false });

        // Isolate each possible contributing texel, including opacity small enough to underflow after weighting.
        for opacity in [0.75, f32::from_bits(1)] {
            for lit_index in 0..16 {
                let mut texels = vec![Texel::default(); 16];
                texels[lit_index] = Texel { rgb: [opacity, opacity * 0.5, 0.0], opacity };
                let lit = lit_rows_of(&texels, 4);
                fast.prepare(&lit, None);
                exact.prepare(&lit, None);
                for &x in &positions {
                    for &y in &positions {
                        for footprint in [[1.0; 2], [0.1, 0.9]] {
                            let expected = exact.sample(&texels, [x, y], footprint);
                            let actual = fast.sample(&texels, [x, y], footprint);
                            assert_eq!(actual, expected, "lit={lit_index} pos={x},{y} opacity={opacity}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bilinear_matches_dense_box_integration_and_falls_back_for_larger_footprints() {
        // Exercise the fast interior path and exact boundary path with a changing colour field.
        let texels = gradient();
        let lit = lit_rows_of(&texels, 4);
        let mut fast = TileSampler::new(4, &ProjectionOptions { bilinear_fast_path: true });
        let mut exact = TileSampler::new(4, &ProjectionOptions { bilinear_fast_path: false });
        fast.prepare(&lit, None);
        exact.prepare(&lit, None);
        for index in 0..1000 {
            let position = [(index * 71 % 600) as f64 / 100.0 - 1.0, (index * 37 % 600) as f64 / 100.0 - 1.0];
            for footprint in [[0.3, 1.0], [1.0, 1.0], [1.0_f64.next_up(), 1.0], [1.0, 1.01], [4.0, 7.0]] {
                let expected = exact.sample(&texels, position, footprint);
                let actual = fast.sample(&texels, position, footprint);
                assert_eq!(actual.is_some(), expected.is_some());
                for (value, reference) in actual.unwrap_or([0.0; 4]).into_iter().zip(direct(&texels, 4, position, footprint)) {
                    assert!((f64::from(value) - reference).abs() < 1e-6);
                }
                if footprint[0] > 1.0 || footprint[1] > 1.0 { assert_eq!(actual, expected); }
            }
        }

        // A one-texel image cannot provide four interior neighbours and must use the boundary fallback.
        let mut single = TileSampler::new(1, &ProjectionOptions { bilinear_fast_path: true });
        single.prepare(&[1], None);
        assert_eq!(single.sample(&texels[..1], [0.5; 2], [1.0; 2]), Some([texels[0].rgb[0], texels[0].rgb[1], 0.0, texels[0].opacity]));
        fast.prepare(&[0; 4], None);
        assert_eq!(fast.sample(&vec![Texel::default(); 16], [2.0; 2], [1.0; 2]), None);
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
        let mut sampler = TileSampler::new(64, &Default::default());
        sampler.prepare(&[u64::MAX; 64], None);
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
        let mut sampler = TileSampler::new(edge, &Default::default());
        sampler.prepare(&lit_rows_of(&texels, edge), None);
        for position in [[32.0, 32.0], [0.0, 0.0], [62.99, 0.25], [31.3, 24.7]] {
            for footprint in [[64.0, 64.0], [7.5, 13.2], [1.0, 0.1], [12.0, 5.0]] {
                let expected = direct(&texels, edge, position, footprint);
                let actual = sampler.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
                for axis in 0..4 { assert!((f64::from(actual[axis]) - expected[axis]).abs() < 1e-7); }
            }
        }

        // Bright data before the queried rectangle must not leak into an empty interior through cancellation.
        for row in 16..48 { for column in 16..48 { texels[row * edge + column] = Texel::default(); } }
        sampler.prepare(&lit_rows_of(&texels, edge), None);
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
        sampler.prepare(&lit_rows_of(&texels, edge), None);
        assert_eq!(sampler.sample(&texels, [32.0, 32.0], [64.0; 2]), None);
    }

    #[test]
    fn cached_totals_require_full_coverage_and_keep_the_full_footprint_area() {
        let texels = gradient();
        let mut sum = [0.0_f64; 4];
        for texel in &texels {
            for (sum, value) in sum.iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) { *sum += f64::from(value); }
        }
        let summary = TileFilterSummary { rgba_sum: sum, ..Default::default() };
        let mut sampler = TileSampler::new(4, &Default::default());
        sampler.prepare(&lit_rows_of(&texels, 4), Some(&summary));

        // A wide footprint can still cover only part of a tile when its centre is displaced.
        for (position, footprint) in [([2.0, 2.0], [4.0, 4.0]), ([2.0, 2.0], [8.0, 8.0]),
            ([5.0, 2.0], [8.0, 8.0]), ([2.0, -1.0], [8.0, 8.0]), ([1.0, 1.0], [2.0, 7.0]),
            ([4.0, 4.0], [1.0, 1.0]), ([-50.0, 0.0], [2.0, 2.0]), ([2.0, 2.0], [1e6, 1e6])]
        {
            let expected = direct(&texels, 4, position, footprint);
            let actual = sampler.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
            for channel in 0..4 {
                assert!((f64::from(actual[channel]) - expected[channel]).abs() <= 1e-6 * expected[channel].abs().max(1e-30));
            }
        }

        // Returning to an image without a summary must not reuse the previous image's totals.
        let replacement = vec![Texel { rgb: [0.1, 0.2, 0.3], opacity: 0.5 }; 16];
        sampler.prepare(&lit_rows_of(&replacement, 4), None);
        assert_eq!(sampler.sample(&replacement, [2.0; 2], [4.0; 2]), Some([0.1, 0.2, 0.3, 0.5]));
        sampler.prepare(&[0; 4], Some(&summary));
        assert_eq!(sampler.sample(&vec![Texel::default(); 16], [2.0; 2], [8.0; 2]), None);
    }

}
