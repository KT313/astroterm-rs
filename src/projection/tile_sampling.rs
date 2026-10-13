//! The colour of one merged tile under a screen pixel. The pixel's footprint in texture space is an axis-aligned
//! box of at least one texel; the texels under it are averaged by the area of the box inside each. Where the
//! tile shrinks on screen that is a box filter over a few texels, where it grows it is the bilinear mix of the
//! four texels around the position. Boxes that touch only empty texels are rejected through the tile's lit-row
//! bits before any arithmetic, which is what makes sparse tiles cheap. Nothing here is retained between frames.
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::Texel;

pub(super) struct TileSampler {
    edge: usize,
    lit: [u64; TILE_EDGE_TEXELS],           // one bit per texel per row: which texels hold light
    lit_columns: u64,                       // the rows combined: which columns hold light anywhere
    lit_rows: u64,                          // one bit per row that holds light anywhere
}

impl TileSampler {
    pub(super) fn new(edge: usize) -> Self {
        assert!(edge <= TILE_EDGE_TEXELS && edge <= 64, "lit rows are one word per tile row");
        Self { edge, lit: [0; TILE_EDGE_TEXELS], lit_columns: 0, lit_rows: 0 }
    }

    /// Adopt a tile's lit rows.
    pub(super) fn prepare(&mut self, lit_rows: &[u64]) {
        assert_eq!(lit_rows.len(), self.edge);
        self.lit[..self.edge].copy_from_slice(lit_rows);
        self.lit_columns = lit_rows.iter().fold(0, |all, &row| all | row);
        self.lit_rows = lit_rows.iter().enumerate().fold(0, |any, (index, &row)| if row != 0 { any | 1 << index } else { any });
    }

    /// Average the box of `footprint` texels (at least one) centred at `position`, in texture coordinates from
    /// 0 to edge, or `None` when every texel under it is empty. The box is clipped at the texture edges, where
    /// partially covered screen pixels may have centres outside the tile.
    #[inline]
    pub(super) fn sample(&self, texels: &[Texel], position: [f64; 2], footprint: [f64; 2]) -> Option<[f32; 4]> {
        let edge = self.edge as f64;
        let (half_width, half_height) = (footprint[0].max(1.0) * 0.5, footprint[1].max(1.0) * 0.5);
        let (cu, cv) = (position[0].clamp(0.0, edge), position[1].clamp(0.0, edge));
        let (left, right) = ((cu - half_width).max(0.0), (cu + half_width).min(edge));
        let (top, bottom) = ((cv - half_height).max(0.0), (cv + half_height).min(edge));
        if right <= left || bottom <= top { return None; }

        // Reject the box when none of the texels it touches holds light: columns, then rows, then the exact bits.
        let last = self.edge - 1;
        let (first_column, last_column) = (left as usize, (right.ceil() as usize - 1).min(last));
        let (first_row, last_row) = (top as usize, (bottom.ceil() as usize - 1).min(last));
        let columns = (u64::MAX >> (63 - (last_column - first_column))) << first_column;
        if columns & self.lit_columns == 0 { return None; }
        let rows = (u64::MAX >> (63 - (last_row - first_row))) << first_row;
        if rows & self.lit_rows == 0 { return None; }
        if self.lit[first_row..=last_row].iter().all(|bits| bits & columns == 0) { return None; }

        let rgba = average(texels, self.edge, [left, right], [top, bottom], (first_column, last_column), (first_row, last_row));
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("whole tile filter output", rgba.map(f64::from), 1.0);
        Some(rgba)
    }
}

/// The area average of the texels under a box: each texel weighted by the box area inside it over the box area.
fn average(texels: &[Texel], edge: usize, [left, right]: [f64; 2], [top, bottom]: [f64; 2], (first_column, last_column): (usize, usize), (first_row, last_row): (usize, usize)) -> [f32; 4] {
    let area = (right - left) * (bottom - top);
    let mut rgba = [0.0_f32; 4];
    for row in first_row..=last_row {
        let height = bottom.min(row as f64 + 1.0) - top.max(row as f64);
        for column in first_column..=last_column {
            let width = right.min(column as f64 + 1.0) - left.max(column as f64);
            let weight = (width * height / area) as f32;
            let texel = texels[row * edge + column];
            for (sum, value) in rgba.iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) { *sum += value * weight; }
        }
    }
    rgba
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
        let limit = edge as f64;
        let (x, y) = (x.clamp(0.0, limit), y.clamp(0.0, limit));
        let (left, right) = ((x - width.max(1.0) / 2.0).max(0.0), (x + width.max(1.0) / 2.0).min(limit));
        let (top, bottom) = ((y - height.max(1.0) / 2.0).max(0.0), (y + height.max(1.0) / 2.0).min(limit));
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
    fn boxes_of_any_size_match_direct_texel_area_averages_including_clipped_ones() {
        let texels = gradient();
        let sampler = sampler(&texels);
        for (position, footprint) in [([1.7, 2.2], [2.3, 1.8]), ([0.0, 4.0], [1.0, 1.0]), ([2.0, 2.0], [0.1, 0.1]), ([1.3, 2.6], [1.0, 1.0]),
            ([3.9, 0.2], [0.5, 0.5]), ([-2.0, 1.5], [1.0, 1.0]), ([4.0, 4.0], [3.0, 3.0]), ([2.0, 2.0], [40.0, 40.0])] {
            let actual = sampler.sample(&texels, position, footprint).unwrap();
            for (actual, expected) in actual.into_iter().zip(direct(&texels, 4, position, footprint)) {
                assert!((f64::from(actual) - expected).abs() < 1e-6, "{position:?} {footprint:?}: {actual} != {expected}");
            }
        }
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
}
