//! The metadata panel's layout: one line per field, values aligned at tab stops as in the original curses version.

use crate::canvas::Canvas;
use crate::metadata::MetadataField;

/// Columns of the metadata panel, enough for the longest line (elapsed time).
const PANEL_WIDTH: usize = 45;

/// Tab stops are every 8 columns, as in curses.
const TAB_WIDTH: usize = 8;

/// Values start at this column at the earliest, so they line up for short labels too.
const VALUE_COLUMN: usize = 16;

/// Draw the metadata fields onto the panel canvas, resizing it to fit the lines.
pub fn draw_metadata_panel(canvas: &mut Canvas, fields: &[MetadataField]) {
    canvas.resize(fields.len(), PANEL_WIDTH);
    canvas.clear();
    for (row, field) in fields.iter().enumerate() {
        let line = format_field(&format!("{}: ", field.label), &field.value);
        canvas.put_str_truncated(row as i32, 0, &line, None);
    }
}

/// `label` followed by a tab (expanded to the next tab stop, at least the value column) and `value`.
fn format_field(label: &str, value: &str) -> String {
    let width = label.chars().count();
    let tab_stop = ((width / TAB_WIDTH + 1) * TAB_WIDTH).max(VALUE_COLUMN);
    format!("{label}{}{value}", " ".repeat(tab_stop - width))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields_from(pairs: &[(&str, &str)]) -> Vec<MetadataField> {
        pairs
            .iter()
            .map(|(label, value)| MetadataField {
                label: label.to_string(),
                value: value.to_string(),
            })
            .collect()
    }

    #[test]
    fn panel_aligns_values_and_fits_its_lines() {
        let fields = fields_from(&[
            ("Date (CET)", "02-01-2025 19:30"),
            ("Zodiac", "Capricorn ♑"),
            ("Lunar Phase", "Waxing Crescent"),
            ("Latitude", "-33° 52' 12.00\""),
            ("Longitude", "151° 12' 36.00\""),
            ("Elapsed Time", "001  year, 002 days, 02:03:04"),
            ("Speed", "-100x (paused)"),
            ("Facing", "334.0° (NNW), tilt 20.0°"),
            ("Field of View", "120.0°"),
            ("Projection", "equidistant"),
        ]);
        let mut canvas = Canvas::new(0, 0);
        draw_metadata_panel(&mut canvas, &fields);
        assert_eq!((canvas.height(), canvas.width()), (10, PANEL_WIDTH));

        let lines: Vec<String> = canvas
            .to_lines()
            .iter()
            .map(|line| line.trim_end().to_string())
            .collect();
        assert_eq!(
            lines,
            [
                "Date (CET):     02-01-2025 19:30",
                "Zodiac:         Capricorn ♑",
                "Lunar Phase:    Waxing Crescent",
                "Latitude:       -33° 52' 12.00\"",
                "Longitude:      151° 12' 36.00\"",
                "Elapsed Time:   001  year, 002 days, 02:03:04",
                "Speed:          -100x (paused)",
                "Facing:         334.0° (NNW), tilt 20.0°",
                "Field of View:  120.0°",
                "Projection:     equidistant",
            ]
        );
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(format_field("Zodiac: ", "Leo"), "Zodiac:         Leo");
        assert_eq!(format_field("Speed: ", "1x"), "Speed:          1x");
        assert_eq!(format_field("Date (+05:30): ", "x"), "Date (+05:30):  x");
        assert_eq!(format_field("A label of sixteen", "x"), "A label of sixteen      x");
    }
}
