//! Line drawing in three styles: ASCII slashes, smooth Unicode box-drawing arcs, and braille dots.
//!
//! The ASCII and smooth styles step one cell at a time along the major axis and insert "joint" characters wherever
//! the line moves to a new row or column. Zero-length lines draw nothing.

use super::Canvas;

/// Draw an ASCII line from (row_a, col_a) to (row_b, col_b) using `|`, `-`, `_`, `/` and `\`.
pub fn draw_line_ascii(canvas: &mut Canvas, row_a: i32, col_a: i32, row_b: i32, col_b: i32) {
    let (dy, dx) = (row_b - row_a, col_b - col_a);
    if dy == 0 && dx == 0 {
        return;
    }
    let slope = if (dx > 0) == (dy > 0) { '\\' } else { '/' };

    // mostly vertical: one row per step, slope character where the column changes
    if dy.abs() >= dx.abs() {
        let step_y = if dy > 0 { 1 } else { -1 };
        let step_x = f64::from(dx) / f64::from(dy.abs());
        let (mut y, mut x) = (0_i32, 0.0_f64);
        while y.abs() <= dy.abs() {
            let (row, col) = (row_a + y, col_a + x.round() as i32);
            let next_col = col_a + (x + step_x).round() as i32;
            canvas.put_char(row, col, if next_col != col { slope } else { '|' }, None);
            y += step_y;
            x += step_x;
        }
        return;
    }

    // mostly horizontal: one column per step. '_' sits low in the cell, so slopes going down are drawn in the next
    // cell and slopes going up in the current one
    let step_x = if dx > 0 { 1 } else { -1 };
    let step_y = f64::from(dy) / f64::from(dx.abs());
    let horizontal = if row_a == row_b { '-' } else { '_' };
    let (mut y, mut x) = (0.0_f64, 0_i32);
    while x.abs() <= dx.abs() {
        let (row, col) = (row_a + y.round() as i32, col_a + x);
        let next_row = row_a + (y + step_y).round() as i32;
        canvas.put_char(row, col, horizontal, None);
        if next_row != row {
            if dy < 0 {
                canvas.put_char(row, col, slope, None);
            } else if row != row_b {
                canvas.put_char(next_row, col + step_x, slope, None);
                y += step_y; // the next cell is already drawn
                x += step_x;
            }
        }
        y += step_y;
        x += step_x;
    }
}

/// Draw a smooth Unicode line from (row_a, col_a) to (row_b, col_b) using `│`, `─` and rounded corners.
pub fn draw_line_smooth(canvas: &mut Canvas, row_a: i32, col_a: i32, row_b: i32, col_b: i32) {
    let (dy, dx) = (row_b - row_a, col_b - col_a);
    if dy == 0 && dx == 0 {
        return;
    }

    // mostly vertical: one row per step, a pair of corners where the column changes
    if dy.abs() > dx.abs() {
        let (joint_a, joint_b) = match (dx > 0, dy > 0) {
            (true, true) => ('╰', '╮'),
            (true, false) => ('╭', '╯'),
            (false, true) => ('╯', '╭'),
            (false, false) => ('╮', '╰'),
        };
        let step_y = if dy > 0 { 1 } else { -1 };
        let step_x = f64::from(dx) / f64::from(dy.abs());
        let (mut y, mut x) = (0_i32, 0.0_f64);
        while y.abs() <= dy.abs() {
            let (row, col) = (row_a + y, col_a + x.round() as i32);
            let next_col = col_a + (x + step_x).round() as i32;
            canvas.put_char(row, col, '│', None);
            if next_col != col && col != col_b {
                canvas.put_char(row, col, joint_a, None);
                canvas.put_char(row, next_col, joint_b, None);
            }
            y += step_y;
            x += step_x;
        }
        return;
    }

    // mostly horizontal: one column per step, a pair of corners where the row changes
    let (joint_a, joint_b) = match (dx > 0, dy > 0) {
        (true, true) => ('╮', '╰'),
        (false, true) => ('╭', '╯'),
        (true, false) => ('╯', '╭'),
        (false, false) => ('╰', '╮'),
    };
    let step_x = if dx > 0 { 1 } else { -1 };
    let step_y = f64::from(dy) / f64::from(dx.abs());
    let (mut y, mut x) = (0.0_f64, 0_i32);
    while x.abs() <= dx.abs() {
        let (row, col) = (row_a + y.round() as i32, col_a + x);
        let next_row = row_a + (y + step_y).round() as i32;
        canvas.put_char(row, col, '─', None);
        if next_row != row && row != row_b {
            canvas.put_char(row, col, joint_a, None);
            canvas.put_char(next_row, col, joint_b, None);
        }
        y += step_y;
        x += step_x;
    }
}

/// Draw a line from (row_a, col_a) to (row_b, col_b) with braille dots, at 2x4 dots per cell (Bresenham).
///
/// The endpoints are placed on the dots of their cells that face each other, so lines meeting in a cell don't overlap.
pub fn draw_line_braille(canvas: &mut Canvas, row_a: i32, col_a: i32, row_b: i32, col_b: i32) {
    // dot coordinates of the endpoints
    let (mut x, x_end) = match col_a.cmp(&col_b) {
        std::cmp::Ordering::Less => (col_a * 2 + 1, col_b * 2),
        std::cmp::Ordering::Greater => (col_a * 2, col_b * 2 + 1),
        std::cmp::Ordering::Equal => (col_a * 2, col_b * 2),
    };
    let (mut y, y_end) = match row_a.cmp(&row_b) {
        std::cmp::Ordering::Less => (row_a * 4 + 2, row_b * 4 + 1),
        std::cmp::Ordering::Greater => (row_a * 4 + 1, row_b * 4 + 2),
        std::cmp::Ordering::Equal => (row_a * 4 + 1, row_b * 4 + 1),
    };

    // Bresenham over dots, flushing the accumulated dot mask whenever the line leaves a cell
    let (dx, dy) = ((x_end - x).abs(), (y_end - y).abs());
    let (step_x, step_y) = (if x < x_end { 1 } else { -1 }, if y < y_end { 1 } else { -1 });
    let mut error = (if dx > dy { dx } else { -dy }) / 2;
    let (mut cell_row, mut cell_col) = (row_a, col_a);
    let mut dots = 0_u8;
    loop {
        let (row, col) = (y.div_euclid(4), x.div_euclid(2));
        if (row, col) != (cell_row, cell_col) {
            canvas.put_braille(cell_row, cell_col, dots);
            (dots, cell_row, cell_col) = (0, row, col);
        }
        dots |= braille_dot(y.rem_euclid(4), x.rem_euclid(2));

        if x == x_end && y == y_end {
            break;
        }
        let previous_error = error;
        if previous_error > -dx {
            error -= dy;
            x += step_x;
        }
        if previous_error < dy {
            error += dx;
            y += step_y;
        }
    }
    canvas.put_braille(cell_row, cell_col, dots);
}

/// Bit of the braille dot at (dot_row 0..4, dot_col 0..2) within a cell.
fn braille_dot(dot_row: i32, dot_col: i32) -> u8 {
    const DOTS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    DOTS[dot_row as usize][dot_col as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(height: usize, width: usize, draw_line: impl FnOnce(&mut Canvas)) -> Vec<String> {
        let mut canvas = Canvas::new(height, width);
        draw_line(&mut canvas);
        canvas.to_lines()
    }

    #[test]
    fn ascii_diagonals() {
        let down = draw(10, 10, |canvas| draw_line_ascii(canvas, 0, 0, 9, 9));
        let expected: Vec<String> = (0..10)
            .map(|i| format!("{}\\{}", " ".repeat(i), " ".repeat(9 - i)))
            .collect();
        assert_eq!(down, expected);

        let up = draw(10, 10, |canvas| draw_line_ascii(canvas, 9, 0, 0, 9));
        let expected: Vec<String> = (0..10)
            .map(|i| format!("{}/{}", " ".repeat(9 - i), " ".repeat(i)))
            .collect();
        assert_eq!(up, expected);
    }

    #[test]
    fn ascii_vertical_and_horizontal() {
        let vertical = draw(11, 11, |canvas| draw_line_ascii(canvas, 0, 5, 10, 5));
        assert!(vertical.iter().all(|line| line == "     |     "));

        let horizontal = draw(11, 11, |canvas| draw_line_ascii(canvas, 5, 0, 5, 10));
        for (row, line) in horizontal.iter().enumerate() {
            assert_eq!(line, if row == 5 { "-----------" } else { "           " });
        }
    }

    #[test]
    fn smooth_diagonals() {
        let down = draw(10, 10, |canvas| draw_line_smooth(canvas, 0, 0, 9, 9));
        let expected = [
            "╮         ",
            "╰╮        ",
            " ╰╮       ",
            "  ╰╮      ",
            "   ╰╮     ",
            "    ╰╮    ",
            "     ╰╮   ",
            "      ╰╮  ",
            "       ╰╮ ",
            "        ╰─",
        ];
        assert_eq!(down, expected);

        let up = draw(10, 10, |canvas| draw_line_smooth(canvas, 9, 0, 0, 9));
        let expected = [
            "        ╭─",
            "       ╭╯ ",
            "      ╭╯  ",
            "     ╭╯   ",
            "    ╭╯    ",
            "   ╭╯     ",
            "  ╭╯      ",
            " ╭╯       ",
            "╭╯        ",
            "╯         ",
        ];
        assert_eq!(up, expected);
    }

    #[test]
    fn smooth_vertical_and_horizontal() {
        let vertical = draw(11, 11, |canvas| draw_line_smooth(canvas, 0, 5, 10, 5));
        assert!(vertical.iter().all(|line| line == "     │     "));

        let horizontal = draw(11, 11, |canvas| draw_line_smooth(canvas, 5, 0, 5, 10));
        for (row, line) in horizontal.iter().enumerate() {
            assert_eq!(
                line,
                if row == 5 {
                    "───────────"
                } else {
                    "           "
                }
            );
        }
    }

    #[test]
    fn braille_lines() {
        let vertical = draw(11, 11, |canvas| draw_line_braille(canvas, 0, 5, 10, 5));
        for (row, line) in vertical.iter().enumerate() {
            let expected = match row {
                0 => "     ⡄     ",
                10 => "     ⠃     ",
                _ => "     ⡇     ",
            };
            assert_eq!(line, expected, "row {row}");
        }

        let horizontal = draw(11, 11, |canvas| draw_line_braille(canvas, 5, 0, 5, 10));
        assert_eq!(horizontal[5], "⠐⠒⠒⠒⠒⠒⠒⠒⠒⠒⠂");

        let diagonal = draw(6, 11, |canvas| draw_line_braille(canvas, 0, 0, 5, 10));
        let expected = [
            "⠠⡀         ",
            " ⠈⠢⡀       ",
            "   ⠈⠢⡀     ",
            "     ⠈⠢⡀   ",
            "       ⠈⠢⡀ ",
            "         ⠈⠂",
        ];
        assert_eq!(diagonal, expected);
    }

    #[test]
    fn zero_length_lines_draw_nothing() {
        let blank = Canvas::new(3, 3).to_lines();
        assert_eq!(draw(3, 3, |canvas| draw_line_ascii(canvas, 1, 1, 1, 1)), blank);
        assert_eq!(draw(3, 3, |canvas| draw_line_smooth(canvas, 1, 1, 1, 1)), blank);
    }
}
