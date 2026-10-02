//! Lossless cell snapshot: display text, color for every column, and wide-glyph continuation markers.

use astroterm::canvas::{Canvas, Color};

pub fn describe_canvas(canvas: &Canvas) -> String {
    let mut text = format!("{} rows x {} columns\n", canvas.height(), canvas.width());
    text.push_str("glyphs:\n");
    for line in canvas.to_lines() {
        text.push_str(&format!("|{line}|\n"));
    }
    text.push_str("colors (. default; K R G Y B M C W):\n");
    for row in 0..canvas.height() {
        let colors: String = canvas
            .row(row)
            .iter()
            .map(|cell| match cell.color {
                None => '.',
                Some(Color::Black) => 'K',
                Some(Color::Red) => 'R',
                Some(Color::Green) => 'G',
                Some(Color::Yellow) => 'Y',
                Some(Color::Blue) => 'B',
                Some(Color::Magenta) => 'M',
                Some(Color::Cyan) => 'C',
                Some(Color::White) => 'W',
            })
            .collect();
        text.push_str(&format!("|{colors}|\n"));
    }
    text.push_str("continuations (> continuation, . ordinary cell):\n");
    for row in 0..canvas.height() {
        let marks: String = canvas
            .row(row)
            .iter()
            .map(|cell| if cell.is_continuation() { '>' } else { '.' })
            .collect();
        text.push_str(&format!("|{marks}|\n"));
    }
    text
}
