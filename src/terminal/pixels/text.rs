//! Shared text layout in cells, painted into the final bitmap for graphics protocols or merged into half-block cells.
use crate::{
    metadata::MetadataField,
    projection::ProjectedSky,
    scene::{
        RenderOptions, format_star_label,
        pixels::{planet_rgb, star_rgb},
        select_dynamically_named_stars,
    },
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Clear, Paragraph, Widget},
};

pub(super) fn compose_text(
    sky: &ProjectedSky<'_>,
    options: &RenderOptions,
    screen: Rect,
    area: Rect,
    fields: &[MetadataField],
    notice: Option<&str>,
    times: &mut crate::timing::StepTimes,
) -> Buffer {
    // assemble every text layer in memory before either image encoding or terminal output
    let mut buffer = times.measure("Text canvas", || Buffer::empty(screen));
    times.measure("Star labels", || draw_star_labels(&mut buffer, sky, options, area));
    times.measure("Body labels", || draw_body_labels(&mut buffer, sky, area));
    times.measure("Orientation labels", || {
        draw_orientation_labels(&mut buffer, sky, options, area)
    });
    times.measure("Metadata panel", || draw_metadata(&mut buffer, screen, fields));
    times.measure("Notices", || draw_notices(&mut buffer, sky, screen, notice));
    buffer
}

fn draw_star_labels(buffer: &mut Buffer, sky: &ProjectedSky<'_>, options: &RenderOptions, area: Rect) {
    let cell = |position| map_pixel_to_cell(sky, area, position);
    let named = if options.dynamic_names {
        select_dynamically_named_stars(options, sky)
    } else {
        Vec::new()
    };
    for (index, entry) in sky.stars.iter().enumerate() {
        let Some(position) = entry.cell else {
            continue;
        };
        let label = if named.contains(&index) {
            Some(format_star_label(entry.star, sky.names, true))
        } else if entry.star.magnitude <= options.label_threshold {
            sky.names.get(entry.star.name).map(std::borrow::Cow::Borrowed)
        } else {
            None
        };
        if let Some(label) = label {
            let (row, col) = cell(position);
            let [r, g, b] = star_rgb(entry.star);
            put_label(buffer, area, row - 1, col + 1, &label, Color::Rgb(r, g, b));
        }
    }
}

fn draw_body_labels(buffer: &mut Buffer, sky: &ProjectedSky<'_>, area: Rect) {
    let cell = |position| map_pixel_to_cell(sky, area, position);
    for planet in sky.planets.iter().rev() {
        if let Some(position) = planet.cell {
            let (row, col) = cell(position);
            let [r, g, b] = planet_rgb(planet.kind);
            put_label(buffer, area, row - 1, col + 1, planet.kind.name(), Color::Rgb(r, g, b));
        }
    }
    if let Some(position) = sky.moon.cell {
        let (row, col) = cell(position);
        put_label(buffer, area, row - 1, col + 1, "Moon", Color::White);
    }
}

fn draw_orientation_labels(buffer: &mut Buffer, sky: &ProjectedSky<'_>, options: &RenderOptions, area: Rect) {
    let cell = |position| map_pixel_to_cell(sky, area, position);
    if sky.facing {
        for &(position, label) in &sky.horizon_labels {
            let (row, col) = cell(position);
            put_label(buffer, area, row, col, label, Color::LightBlue);
        }
    } else {
        for (row, col, label) in [
            (area.y, area.x + area.width / 2, "N"),
            (area.bottom().saturating_sub(1), area.x + area.width / 2, "S"),
            (area.y + area.height / 2, area.x, "E"),
            (area.y + area.height / 2, area.right().saturating_sub(1), "W"),
        ] {
            put_label(buffer, area, i32::from(row), i32::from(col), label, Color::LightBlue);
        }
        if options.grid {
            for angle in (0..360).step_by(30) {
                let (s, c) = (angle as f64).to_radians().sin_cos();
                let row = f64::from(area.y) + (1.0 - s) * f64::from(area.height.saturating_sub(1)) / 2.0;
                let col = f64::from(area.x) + (1.0 + c) * f64::from(area.width.saturating_sub(3)) / 2.0;
                put_label(
                    buffer,
                    area,
                    row.round() as i32,
                    col.round() as i32,
                    &angle.to_string(),
                    Color::LightBlue,
                );
            }
        }
    }
}

fn draw_metadata(buffer: &mut Buffer, screen: Rect, fields: &[MetadataField]) {
    for (row, field) in fields.iter().take(usize::from(screen.height)).enumerate() {
        let line = format!("{}: {}", field.label, field.value);
        let width = unicode_width::UnicodeWidthStr::width(line.as_str()).min(usize::from(screen.width.min(58)));
        let area = Rect::new(0, row as u16, width as u16, 1);
        Clear.render(area, buffer); // replace overlapping labels only where this metadata line actually appears
        Paragraph::new(line)
            .style(Style::default().fg(Color::White)) // reset background leaves the underlying raster visible
            .render(area, buffer);
    }
}

fn draw_notices(buffer: &mut Buffer, sky: &ProjectedSky<'_>, screen: Rect, notice: Option<&str>) {
    if let Some(notice) = notice {
        Paragraph::new(notice)
            .style(Style::default().fg(Color::Yellow).bg(Color::Black))
            .render(Rect::new(0, screen.height.saturating_sub(2), screen.width, 1), buffer);
    }
    if sky.outside_accuracy_range {
        Paragraph::new(crate::astro::accuracy::ACCURACY_WARNING)
            .style(Style::default().fg(Color::Yellow).bg(Color::Black))
            .render(Rect::new(0, screen.height.saturating_sub(1), screen.width, 1), buffer);
    }
}

fn map_pixel_to_cell(sky: &ProjectedSky<'_>, area: Rect, (row, col): (i32, i32)) -> (i32, i32) {
    (
        i32::from(area.y) + (f64::from(row) / sky.viewport.height.max(1) as f64 * f64::from(area.height)) as i32,
        i32::from(area.x) + (f64::from(col) / sky.viewport.width.max(1) as f64 * f64::from(area.width)) as i32,
    )
}

fn put_label(buffer: &mut Buffer, area: Rect, row: i32, col: i32, text: &str, color: Color) {
    if row < i32::from(area.y)
        || col < i32::from(area.x)
        || row >= i32::from(area.bottom())
        || col >= i32::from(area.right())
    {
        return;
    }
    Paragraph::new(text)
        .style(Style::default().fg(color).bg(Color::Rgb(3, 6, 14)))
        .render(
            Rect::new(
                col as u16,
                row as u16,
                (area.right() - col as u16).min(unicode_width::UnicodeWidthStr::width(text) as u16),
                1,
            )
            .intersection(buffer.area),
            buffer,
        );
}
