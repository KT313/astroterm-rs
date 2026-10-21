//! Image-derived projection bounds, calculated only when a region's merged tile changes.
use crate::constants::TILE_EDGE_TEXELS;

/// Enclose every lit texel and the minimum filter's support in a circle around the tile centre.
/// Each row needs only its two outermost lit columns. Empty images have radius zero.
pub(super) fn calculate_content_radius(lit_rows: &[u64]) -> f64 {
    // The half-texel edge and half-texel filter support extend each lit centre by one texel.
    let middle = (TILE_EDGE_TEXELS as f64 - 1.0) * 0.5;
    let mut greatest_squared_distance = 0.0_f64;

    // Find the farthest supported corner without scanning the individual texels in each row.
    for (row, &lit_columns) in lit_rows.iter().enumerate() {
        if lit_columns == 0 { continue; }
        let first_column = lit_columns.trailing_zeros() as usize;
        let last_column = (u64::BITS - 1 - lit_columns.leading_zeros()) as usize;
        let horizontal_distance = (first_column as f64 - middle).abs().max((last_column as f64 - middle).abs()) + 1.0;
        let vertical_distance = (row as f64 - middle).abs() + 1.0;
        let squared_distance = horizontal_distance * horizontal_distance + vertical_distance * vertical_distance;
        greatest_squared_distance = greatest_squared_distance.max(squared_distance);
    }

    // One square root gives the radius shared by every future projection of this image.
    greatest_squared_distance.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_endpoints_enclose_every_supported_texel_corner() {
        let edge = TILE_EDGE_TEXELS;
        let middle = (edge as f64 - 1.0) * 0.5;
        let mut rows = vec![0_u64; edge];

        // Check every single texel, including all four outer corners and central subpixel images.
        for row in 0..edge {
            for column in 0..edge {
                rows[row] = 1 << column;
                let expected = ((column as f64 - middle).abs() + 1.0).hypot((row as f64 - middle).abs() + 1.0);
                assert!((calculate_content_radius(&rows) - expected).abs() < 1e-12);
                rows[row] = 0;
            }
        }

        // Disconnected islands and filled rows must include their most distant corner as well.
        rows[3] = (1 << 9) | (1 << 25) | (1 << 60);
        rows[40] = u64::MAX;
        let mut expected = 0.0_f64;
        for (row, &bits) in rows.iter().enumerate() {
            for column in 0..edge {
                if bits & (1 << column) == 0 { continue; }
                expected = expected.max(((column as f64 - middle).abs() + 1.0).hypot((row as f64 - middle).abs() + 1.0));
            }
        }
        assert!((calculate_content_radius(&rows) - expected).abs() < 1e-12);
    }

    #[test]
    fn empty_and_full_tiles_have_the_expected_support() {
        assert_eq!(calculate_content_radius(&[]), 0.0);
        assert_eq!(calculate_content_radius(&[0; TILE_EDGE_TEXELS]), 0.0);
        let full_radius = (TILE_EDGE_TEXELS as f64 * 0.5 + 0.5) * std::f64::consts::SQRT_2;
        assert!((calculate_content_radius(&[u64::MAX; TILE_EDGE_TEXELS]) - full_radius).abs() < 1e-12);
    }
}
