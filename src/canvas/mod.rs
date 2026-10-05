//! An in-memory grid of terminal cells that the scene is drawn onto before it is presented.
//!
//! Coordinates are signed (row, column) pairs with row 0 at the top. Writes outside the grid are ignored, so drawing
//! code does not need to clip. Glyphs that take two columns (e.g. emoji) occupy their cell and the next one.

mod lines;

use unicode_width::UnicodeWidthChar;

pub use lines::{draw_line_ascii, draw_line_braille, draw_line_smooth};

/// First code point of the Unicode braille block. The low 8 bits of a braille character are its dot mask.
const BRAILLE_BASE: u32 = 0x2800;

/// Marks the second column of a wide glyph.
const CONTINUATION: char = '\0';

/// The eight basic terminal colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

/// One terminal cell. `color: None` uses the terminal's default foreground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub symbol: char,
    pub color: Option<Color>,
}

impl Cell {
    const BLANK: Cell = Cell {
        symbol: ' ',
        color: None,
    };

    /// Whether this cell is the second column of a wide glyph and is covered by the cell before it.
    pub fn is_continuation(&self) -> bool {
        self.symbol == CONTINUATION
    }
}

/// A fixed-size grid of cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    height: usize,
    width: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    /// A blank canvas with `height` rows and `width` columns.
    pub fn new(height: usize, width: usize) -> Canvas {
        Canvas {
            height,
            width,
            cells: vec![Cell::BLANK; height * width],
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// Change the size to `height` x `width`. The content is blanked if the size changes.
    pub fn resize(&mut self, height: usize, width: usize) {
        if (height, width) != (self.height, self.width) {
            *self = Canvas::new(height, width);
        }
    }

    /// Reset every cell to blank.
    pub fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
    }

    /// The cell at (row, col), or `None` outside the grid.
    pub fn cell(&self, row: i32, col: i32) -> Option<&Cell> {
        self.index_of(row, col).map(|index| &self.cells[index])
    }

    /// The cells of one row.
    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row * self.width..(row + 1) * self.width]
    }

    /// Write a glyph. Ignored if it does not fit inside the grid.
    pub fn put_char(&mut self, row: i32, col: i32, symbol: char, color: Option<Color>) {
        let wide = symbol.width() == Some(2);
        let Some(index) = self.index_of(row, col) else {
            return;
        };
        if wide && self.index_of(row, col + 1).is_none() {
            return; // the second half would fall off the right edge
        }

        // overwrite the cell, plus the next one for wide glyphs
        self.clear_wide_remnants(row, col);
        self.cells[index] = Cell { symbol, color };
        if wide {
            self.clear_wide_remnants(row, col + 1);
            self.cells[index + 1] = Cell {
                symbol: CONTINUATION,
                color,
            };
        }
    }

    /// Write a string left to right, cutting it off at the right edge instead of wrapping.
    pub fn put_str_truncated(&mut self, row: i32, col: i32, text: &str, color: Option<Color>) {
        let mut col = col;
        for symbol in text.chars() {
            if col >= self.width as i32 {
                break;
            }
            self.put_char(row, col, symbol, color);
            col += symbol.width().unwrap_or(0) as i32;
        }
    }

    /// Add braille dots to a cell. If the cell already holds a braille character, the dots are merged into it, so
    /// lines crossing the same cell combine.
    pub fn put_braille(&mut self, row: i32, col: i32, dots: u8) {
        if dots == 0 {
            return;
        }
        let Some(existing) = self.cell(row, col) else {
            return;
        };
        let existing_dots = braille_dots(existing.symbol).unwrap_or(0);
        let merged = char::from_u32(BRAILLE_BASE + u32::from(existing_dots | dots)).expect("braille block is valid");
        self.put_char(row, col, merged, None);
    }

    /// Copy every cell of `source` (blank ones included) onto this canvas with its top left corner at (row, col).
    /// Parts outside this canvas are cut off.
    pub fn blit(&mut self, source: &Canvas, row: i32, col: i32) {
        for source_row in 0..source.height {
            for (source_col, cell) in source.row(source_row).iter().enumerate() {
                if !cell.is_continuation() {
                    self.put_char(
                        row + source_row as i32,
                        col + source_col as i32,
                        cell.symbol,
                        cell.color,
                    );
                }
            }
        }
    }

    /// Render each row as a string (wide glyphs once, continuation cells skipped). Useful for tests and snapshots.
    pub fn to_lines(&self) -> Vec<String> {
        (0..self.height)
            .map(|row| {
                self.row(row)
                    .iter()
                    .filter(|cell| !cell.is_continuation())
                    .map(|cell| cell.symbol)
                    .collect()
            })
            .collect()
    }

    fn index_of(&self, row: i32, col: i32) -> Option<usize> {
        let in_bounds = row >= 0 && col >= 0 && (row as usize) < self.height && (col as usize) < self.width;
        in_bounds.then(|| row as usize * self.width + col as usize)
    }

    /// Before overwriting (row, col), blank the other half of any wide glyph that covers it.
    fn clear_wide_remnants(&mut self, row: i32, col: i32) {
        let Some(index) = self.index_of(row, col) else {
            return;
        };
        let other_half = if self.cells[index].is_continuation() {
            self.index_of(row, col - 1)
        } else if self.cells[index].symbol.width() == Some(2) {
            self.index_of(row, col + 1)
        } else {
            None
        };
        if let Some(other_half) = other_half {
            self.cells[other_half] = Cell::BLANK;
        }
    }
}

/// Dot mask of a braille character, or `None` for any other character.
fn braille_dots(symbol: char) -> Option<u8> {
    let offset = u32::from(symbol).checked_sub(BRAILLE_BASE)?;
    u8::try_from(offset).ok()
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::buffers::report_fields!(Canvas { cells });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_outside_the_grid_are_ignored() {
        let mut canvas = Canvas::new(2, 3);
        for (row, col) in [(-1, 0), (0, -1), (2, 0), (0, 3), (i32::MAX, i32::MIN)] {
            canvas.put_char(row, col, 'x', None);
            canvas.put_braille(row, col, 0x01);
        }
        assert_eq!(canvas, Canvas::new(2, 3));
    }

    #[test]
    fn put_str_truncated_cuts_off_at_right_edge() {
        let mut canvas = Canvas::new(1, 5);
        canvas.put_str_truncated(0, 2, "Vega", Some(Color::Blue));
        assert_eq!(canvas.to_lines(), ["  Veg"]);
        assert_eq!(canvas.cell(0, 2).unwrap().color, Some(Color::Blue));

        canvas.put_str_truncated(0, -2, "Moon", None);
        assert_eq!(canvas.to_lines(), ["onVeg"]);
    }

    #[test]
    fn braille_dots_merge_within_a_cell() {
        let mut canvas = Canvas::new(1, 1);
        canvas.put_braille(0, 0, 0x01);
        canvas.put_braille(0, 0, 0x80);
        assert_eq!(canvas.to_lines(), ["⢁"]);

        canvas.put_char(0, 0, '*', None); // other glyphs are replaced, not merged
        canvas.put_braille(0, 0, 0x02);
        assert_eq!(canvas.to_lines(), ["⠂"]);
    }

    #[test]
    fn wide_glyphs_take_two_columns() {
        let mut canvas = Canvas::new(1, 4);
        canvas.put_char(0, 1, '🌕', None);
        assert_eq!(canvas.to_lines(), [" 🌕 "]);
        assert!(canvas.cell(0, 2).unwrap().is_continuation());

        canvas.put_char(0, 3, '🌕', None); // would not fit
        assert_eq!(canvas.to_lines(), [" 🌕 "]);
    }

    #[test]
    fn overwriting_half_of_a_wide_glyph_blanks_the_other_half() {
        let mut canvas = Canvas::new(1, 4);
        canvas.put_char(0, 1, '🌕', None);
        canvas.put_char(0, 2, '+', None);
        assert_eq!(canvas.to_lines(), ["  + "]);

        canvas.put_char(0, 0, '🌕', None);
        canvas.put_char(0, 0, '+', None);
        assert_eq!(canvas.to_lines(), ["+ + "]);
    }

    #[test]
    fn blit_copies_cells_and_clips() {
        let mut source = Canvas::new(2, 3);
        source.put_str_truncated(0, 0, "ab", Some(Color::Cyan));
        source.put_char(1, 1, '🌕', None);

        let mut target = Canvas::new(3, 4);
        target.put_str_truncated(1, 0, "xxxx", None);
        target.put_str_truncated(2, 0, "yyyy", None);
        target.blit(&source, 1, 2);
        assert_eq!(target.to_lines(), ["    ", "xxab", "yy y"]); // blanks are copied, the moon doesn't fit
        assert_eq!(target.cell(1, 2).unwrap().color, Some(Color::Cyan));
    }

    #[test]
    fn resize_keeps_content_only_at_the_same_size() {
        let mut canvas = Canvas::new(1, 2);
        canvas.put_char(0, 0, 'x', None);
        canvas.resize(1, 2);
        assert_eq!(canvas.to_lines(), ["x "]);
        canvas.resize(2, 1);
        assert_eq!(canvas.to_lines(), [" ", " "]);
    }

    #[test]
    fn clear_blanks_every_cell() {
        let mut canvas = Canvas::new(2, 2);
        canvas.put_char(1, 1, 'x', Some(Color::Red));
        canvas.clear();
        assert_eq!(canvas, Canvas::new(2, 2));
    }
}
