//! Placing the canvases on the screen and writing them to the terminal.

use std::io::{self, IsTerminal, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{self, Print, ResetColor, SetForegroundColor};
use crossterm::terminal;
use unicode_width::UnicodeWidthChar;

use crate::canvas::{Canvas, Color};

/// Cell aspect ratio assumed when it can't be detected.
const DEFAULT_CELL_ASPECT_RATIO: f64 = 2.0;

/// Where the canvas sits on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    Viewport {
        origin_row: origin_row.max(0) as u16,
        origin_col: origin_col.max(0) as u16,
        height,
        width,
    }
}

/// The terminal cell aspect ratio (height / width), from the pixel size of the terminal if it reports one.
///
/// Many environments (e.g. some multiplexers, Docker) don't report pixel sizes; then a ratio of 2 is assumed.
pub fn detect_cell_aspect_ratio() -> f64 {
    if !io::stdout().is_terminal() {
        return DEFAULT_CELL_ASPECT_RATIO;
    }
    match terminal::window_size() {
        Ok(size) if size.width > 0 && size.height > 0 && size.rows > 0 && size.columns > 0 => {
            let cell_height = f64::from(size.height) / f64::from(size.rows);
            let cell_width = f64::from(size.width) / f64::from(size.columns);
            cell_height / cell_width
        }
        _ => DEFAULT_CELL_ASPECT_RATIO,
    }
}

/// The canvases drawn each frame: the square sky view, and an optional panel drawn over it in the top left corner of
/// the screen (cut off at the screen edges).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sky: Canvas,
    pub panel: Option<Canvas>,
}

/// Composes frames onto a screen-sized canvas and writes them to the terminal, skipping cells that are unchanged
/// since the previous frame.
#[derive(Debug)]
pub struct Presenter {
    screen: Canvas,
    previous: Option<Canvas>,
    sky_origin: (u16, u16),
}

impl Default for Presenter {
    fn default() -> Presenter {
        Presenter {
            screen: Canvas::new(0, 0),
            previous: None,
            sky_origin: (0, 0),
        }
    }
}

impl Presenter {
    /// Start over on a screen of `rows` x `columns` with the sky at `viewport`; the next frame is written in full.
    pub fn reset(&mut self, rows: u16, columns: u16, viewport: Viewport) {
        self.screen = Canvas::new(rows as usize, columns as usize);
        self.previous = None;
        self.sky_origin = (viewport.origin_row, viewport.origin_col);
    }

    /// Queue the changed cells of the frame to `out`. The panel is drawn over the sky.
    pub fn present(&mut self, out: &mut impl Write, frame: &Frame) -> io::Result<()> {
        // compose the screen
        self.screen.clear();
        self.screen
            .blit(&frame.sky, i32::from(self.sky_origin.0), i32::from(self.sky_origin.1));
        if let Some(panel) = &frame.panel {
            self.screen.blit(panel, 0, 0);
        }

        // write what changed
        queue_changed_cells(out, &self.screen, self.previous.as_ref())?;
        self.previous = Some(self.screen.clone());
        Ok(())
    }
}

/// Queue every cell of `screen` that differs from `previous` (all cells without one).
fn queue_changed_cells(out: &mut impl Write, screen: &Canvas, previous: Option<&Canvas>) -> io::Result<()> {
    let mut current_color: Option<Option<Color>> = None; // unknown until the first cell is written
    for row in 0..screen.height() {
        let previous_cells = previous.map(|previous| previous.row(row));
        let mut cursor_col = None; // column the terminal cursor is at after the last write in this row

        for (col, cell) in screen.row(row).iter().enumerate() {
            if cell.is_continuation() || previous_cells.is_some_and(|previous| previous[col] == *cell) {
                continue;
            }
            if cursor_col != Some(col) {
                queue!(out, MoveTo(col as u16, row as u16))?;
            }
            if current_color != Some(cell.color) {
                queue_color(out, cell.color)?;
                current_color = Some(cell.color);
            }
            queue!(out, Print(cell.symbol))?;
            cursor_col = Some(col + cell.symbol.width().unwrap_or(1));
        }
    }
    Ok(())
}

fn queue_color(out: &mut impl Write, color: Option<Color>) -> io::Result<()> {
    match color {
        Some(color) => queue!(out, SetForegroundColor(to_terminal_color(color))),
        None => queue!(out, ResetColor),
    }
}

/// The basic ANSI colors (30-37), as used by curses.
fn to_terminal_color(color: Color) -> style::Color {
    match color {
        Color::Black => style::Color::Black,
        Color::Red => style::Color::DarkRed,
        Color::Green => style::Color::DarkGreen,
        Color::Yellow => style::Color::DarkYellow,
        Color::Blue => style::Color::DarkBlue,
        Color::Magenta => style::Color::DarkMagenta,
        Color::Cyan => style::Color::DarkCyan,
        Color::White => style::Color::Grey,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_is_limited_by_height_on_wide_terminals() {
        let viewport = fit_square_viewport(40, 200, 2.0);
        assert_eq!((viewport.height, viewport.width), (40, 80));
        assert_eq!((viewport.origin_row, viewport.origin_col), (0, 60));
    }

    #[test]
    fn viewport_is_limited_by_width_on_tall_terminals() {
        let viewport = fit_square_viewport(60, 80, 2.0);
        assert_eq!((viewport.height, viewport.width), (40, 80));
        assert_eq!((viewport.origin_row, viewport.origin_col), (10, 0));
    }

    #[test]
    fn presenter_writes_only_changed_cells() {
        let mut presenter = Presenter::default();
        let viewport = Viewport {
            origin_row: 0,
            origin_col: 1,
            height: 2,
            width: 3,
        };
        presenter.reset(2, 5, viewport);
        let mut frame = Frame {
            sky: Canvas::new(2, 3),
            panel: None,
        };

        let mut first = Vec::new();
        presenter.present(&mut first, &frame).unwrap();
        assert!(!first.is_empty());

        let mut unchanged = Vec::new();
        presenter.present(&mut unchanged, &frame).unwrap();
        assert!(unchanged.is_empty());

        frame.sky.put_char(1, 2, '*', Some(Color::Red));
        let mut changed = Vec::new();
        presenter.present(&mut changed, &frame).unwrap();
        let written = String::from_utf8(changed).unwrap();
        assert!(written.contains('*') && !written.contains(' '));
        assert_eq!(presenter.screen.to_lines(), ["     ", "   * "]);
    }

    #[test]
    fn panel_is_drawn_over_the_sky() {
        let mut presenter = Presenter::default();
        presenter.reset(
            2,
            4,
            Viewport {
                origin_row: 0,
                origin_col: 0,
                height: 2,
                width: 4,
            },
        );
        let mut sky = Canvas::new(2, 4);
        sky.put_str_truncated(0, 0, "abcd", None);
        let mut panel = Canvas::new(1, 2);
        panel.put_char(0, 0, 'x', None);

        presenter
            .present(
                &mut Vec::new(),
                &Frame {
                    sky,
                    panel: Some(panel),
                },
            )
            .unwrap();
        assert_eq!(presenter.screen.to_lines(), ["x cd", "    "]);
    }
}
