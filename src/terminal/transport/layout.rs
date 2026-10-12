//! Where the square sky sits on the terminal: the cell aspect ratio and the largest centred square.
use crate::constants::DEFAULT_CELL_ASPECT_RATIO;
use crossterm::terminal;
use std::io::{self, IsTerminal};

/// A square sky area in terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub origin_row: u16,
    pub origin_col: u16,
    pub height: usize,
    pub width: usize,
}

/// The largest viewport that looks square on screen, centered in a terminal of `rows` x `columns` cells.
/// `aspect_ratio` is the cell height divided by the cell width.
pub fn fit_square_viewport(rows: u16, columns: u16, aspect_ratio: f64) -> Viewport {
    // size: limited by the width or the height of the terminal
    let (rows_f, columns_f) = (f64::from(rows), f64::from(columns));
    let (height, width) = if columns_f < rows_f * aspect_ratio {
        ((columns_f / aspect_ratio) as usize, columns as usize)
    } else {
        (rows as usize, (rows_f * aspect_ratio) as usize)
    };

    // position: centered
    let origin_row = (i64::from(rows) - (height as i64 - 1)) / 2;
    let origin_col = (i64::from(columns) - (width as i64 - 1)) / 2;
    Viewport { origin_row: origin_row.max(0) as u16, origin_col: origin_col.max(0) as u16, height, width }
}

/// The terminal cell aspect ratio (height / width), from the pixel size of the terminal if it reports one.
///
/// Many environments (e.g. some multiplexers, Docker) don't report pixel sizes; then a ratio of 2 is assumed.
pub fn detect_cell_aspect_ratio() -> f64 {
    if !io::stdout().is_terminal() { return DEFAULT_CELL_ASPECT_RATIO; }
    match terminal::window_size() {
        Ok(size) if size.width > 0 && size.height > 0 && size.rows > 0 && size.columns > 0 => {
            let cell_height = f64::from(size.height) / f64::from(size.rows);
            let cell_width = f64::from(size.width) / f64::from(size.columns);
            cell_height / cell_width
        }
        _ => DEFAULT_CELL_ASPECT_RATIO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_is_limited_by_the_shorter_side_and_centred() {
        let wide = fit_square_viewport(24, 100, 2.0);                                     // 24 rows × 2 = 48 columns fit
        assert_eq!((wide.height, wide.width), (24, 48));
        assert_eq!(wide.origin_col, (100 - 47) / 2);
        let tall = fit_square_viewport(60, 40, 2.0);                                      // 40 columns / 2 = 20 rows fit
        assert_eq!((tall.height, tall.width), (20, 40));
        assert_eq!(tall.origin_row, (60 - 19) / 2);
    }
}
