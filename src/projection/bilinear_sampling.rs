//! Unit-size box filtering inside a tile. The four overlap weights are the bilinear weights;
//! keep the box-edge arithmetic so rounding at texel boundaries matches the general filter.
use crate::model::Texel;

/// Both coordinates must be in [0.5, edge - 0.5), so all four neighbours are valid tile indices.
/// Positions use edge coordinates: texel zero is centred at 0.5, not zero.
#[inline]
pub(super) fn sample_bilinear_interior(texels: &[Texel], edge: usize, position: [f64; 2]) -> Option<[f32; 4]> {
    // Find the first texel touched by the unit box and the boundary to its next neighbour.
    let [column, row] = position;
    let (left, top) = (column - 0.5, row - 0.5);
    let (first_column, first_row) = (left as usize, top as usize);
    let column_boundary = (first_column + 1) as f64;
    let row_boundary = (first_row + 1) as f64;

    // Use the actual rounded box edges, including zero weight when an edge exactly touches a texel.
    let (right, bottom) = (column + 0.5, row + 0.5);
    let left_width = right.min(column_boundary) - left;
    let right_width = (right - column_boundary).max(0.0);
    let top_height = bottom.min(row_boundary) - top;
    let bottom_height = (bottom - row_boundary).max(0.0);
    let weights = [left_width * top_height, right_width * top_height,
        left_width * bottom_height, right_width * bottom_height];

    // Accumulate the four pixels in the same row order as the box filter, without renormalizing.
    let first = first_row * edge + first_column;
    let neighbours = [texels[first], texels[first + 1], texels[first + edge], texels[first + edge + 1]];
    let mut rgba = [0.0_f64; 4];
    for (texel, weight) in neighbours.into_iter().zip(weights) {
        for (sum, channel) in rgba[..3].iter_mut().zip(texel.rgb) { *sum += f64::from(channel) * weight; }
        rgba[3] += f64::from(texel.opacity) * weight;
    }

    // Check for empty support before converting, so a tiny positive contribution is not discarded.
    if rgba[3] == 0.0 { return None; }
    let rgba = rgba.map(|channel| channel as f32);
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_pixel("bilinear filter output", rgba.map(f64::from), 1.0);
    Some(rgba)
}
