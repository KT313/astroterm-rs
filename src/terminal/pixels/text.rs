//! Shared text layout in cells, painted into the final bitmap for graphics protocols or merged into half-block cells.
use crate::{
    metadata::MetadataField,
    projection::ProjectedSky,
    scene::{RenderOptions, format_star_label, pixels::planet_rgb, select_dynamically_named_stars},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Clear, Paragraph, Widget},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn compose_text(
    sky: &ProjectedSky<'_>,
    options: &RenderOptions,
    screen: Rect,
    area: Rect,
    fields: &[MetadataField],
    notice: Option<&str>,
    times: &mut crate::timing::StepTimes,
    prepared: Option<&crate::scene::prepared::PreparedScene>,
    named_candidates: Option<&[usize]>,
) -> Buffer {
    // assemble every text layer in memory before either image encoding or terminal output
    let mut buffer = times.measure("Text canvas", || Buffer::empty(screen));
    times.describe("Text canvas", || {
        format!(
            "output text grid={}x{}; cells={}",
            screen.width,
            screen.height,
            buffer.content.len()
        )
    });
    let (eligible, submitted, visited) = times.measure("Star labels", || {
        draw_star_labels(&mut buffer, sky, options, area, prepared, named_candidates)
    });
    times.describe("Star labels", || format!("input stars={}; label candidates={eligible}; skipped by label rules or missing cell={}; clipped label origins={}; submitted labels={submitted}; label threshold={}; dynamic names={}", sky.stars.len(), sky.stars.len()-eligible, eligible-submitted, options.label_threshold, options.dynamic_names));
    times.describe("Star labels", || format!("prepared named candidates={:?}; visited candidates={visited}; dynamic candidates selected separately; labels retain projected draw order", named_candidates.map(<[usize]>::len)));
    times.measure("Body labels", || draw_body_labels(&mut buffer, sky, area));
    times.describe("Body labels", || {
        format!(
            "input Sun/planets={}; input Moon=1; visible label candidates={}; labels clipped to text area",
            sky.planets.len(),
            sky.planets.iter().filter(|p| p.cell.is_some()).count() + usize::from(sky.moon.cell.is_some())
        )
    });
    times.measure("Orientation labels", || {
        draw_orientation_labels(&mut buffer, sky, options, area)
    });
    times.describe("Orientation labels", || {
        format!(
            "facing={}; grid={}; horizon label inputs={}; text clipped to area={}x{}",
            sky.facing,
            options.grid,
            sky.horizon_labels.len(),
            area.width,
            area.height
        )
    });
    times.measure("Metadata panel", || draw_metadata(&mut buffer, screen, fields));
    times.describe("Metadata panel", || {
        format!(
            "input fields={}; drawn rows={}; clipped rows={}; transparent background",
            fields.len(),
            fields.len().min(usize::from(screen.height)),
            fields.len().saturating_sub(usize::from(screen.height))
        )
    });
    times.measure("Notices", || draw_notices(&mut buffer, sky, screen, notice));
    times.describe("Notices", || {
        format!(
            "fallback notice={}; accuracy warning={}; final nonblank text cells={}",
            notice.is_some(),
            sky.outside_accuracy_range,
            buffer.content.iter().filter(|c| !c.symbol().trim().is_empty()).count()
        )
    });
    buffer
}

fn draw_star_labels(
    buffer: &mut Buffer,
    sky: &ProjectedSky<'_>,
    options: &RenderOptions,
    area: Rect,
    prepared: Option<&crate::scene::prepared::PreparedScene>,
    named_candidates: Option<&[usize]>,
) -> (usize, usize, usize) {
    let mut eligible = 0;
    let mut submitted = 0;
    let cell = |position| map_pixel_to_cell(sky, area, position);
    let named = if options.dynamic_names {
        select_dynamically_named_stars(options, sky)
    } else {
        Vec::new()
    };
    let mut candidates = named_candidates.map(|indices| indices.to_vec());
    if let Some(indices) = &mut candidates {
        indices.extend(named.iter().copied());
        indices.sort_unstable();
        indices.dedup(); // preserve the original dim-to-bright label overwrite order
    }
    let count = candidates.as_ref().map_or(sky.stars.len(), Vec::len);
    for slot in 0..count {
        let index = candidates.as_ref().map_or(slot, |indices| indices[slot]);
        let entry = &sky.stars[index];
        let Some(position) = entry.cell else {
            continue;
        };
        let label = if named.contains(&index) {
            Some(format_star_label(&entry.star, sky.names, true))
        } else if entry.star.magnitude <= options.label_threshold {
            sky.names.get(entry.star.name()).map(std::borrow::Cow::Borrowed)
        } else {
            None
        };
        if let Some(label) = label {
            let (row, col) = cell(position);
            let [r, g, b] = crate::scene::prepared::resolve_star_rgb(&entry.star, prepared);
            eligible += 1;
            submitted += usize::from(put_label(buffer, area, row - 1, col + 1, &label, Color::Rgb(r, g, b)));
        }
    }
    (eligible, submitted, count)
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

fn put_label(buffer: &mut Buffer, area: Rect, row: i32, col: i32, text: &str, color: Color) -> bool {
    if row < i32::from(area.y)
        || col < i32::from(area.x)
        || row >= i32::from(area.bottom())
        || col >= i32::from(area.right())
    {
        return false;
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
    true
}

#[cfg(test)]
mod prepared_tests {
    use super::*;
    use crate::{
        astro::Horizontal,
        catalog::{Catalog, StarNames, load_embedded_catalog},
        projection::{View, Viewport, project_sky},
        scene::cached::SceneCache,
        sky::Sky,
        timing::StepTimes,
    };

    #[test]
    fn prepared_label_candidates_preserve_order_clipping_and_dynamic_names() {
        let mut parsed = load_embedded_catalog().unwrap();
        parsed.stars.truncate(12);
        let mut names = StarNames::default();
        let name = names.insert("Named 星");
        for (i, star) in parsed.stars.iter_mut().enumerate() {
            star.name = (i % 3 == 0).then_some(name);
            star.magnitude = i as f32 * 0.25;
        }
        let mut sky = Sky::from_catalog(&Catalog::new(parsed.stars, names, vec![]));
        for star in &mut sky.stars {
            star.position = Horizontal {
                azimuth: 0.5,
                altitude: 1.1,
            }
            .to_unit_vector();
        }
        let mut cache = SceneCache::default();
        cache.prepare_catalog(sky.catalog.clone(), &mut StepTimes::default());
        let area = Rect::new(0, 0, 40, 20);
        let mut options = RenderOptions {
            unicode: true,
            braille: false,
            color: true,
            constellations: false,
            grid: false,
            magnitude_threshold: 5.0,
            label_threshold: 0.25,
            dynamic_names: true,
        };
        for phase in 0..6 {
            let mut projected = project_sky(&sky, &View::default(), Viewport { width: 160, height: 80 });
            projected.planets.iter_mut().for_each(|p| p.cell = None);
            projected.moon.cell = None;
            match phase {
                1 => options.label_threshold = 5.0,
                2 => options.dynamic_names = false,
                3 => projected.stars.reverse(),
                4 => {
                    projected.stars[0].cell = None;
                    projected.stars[1].cell = Some((0, 0));
                }
                5 => projected.stars.clear(),
                _ => {}
            }
            cache
                .draw_pixels(&projected, &options, 0.0, &mut StepTimes::default())
                .unwrap();
            let mut expected = Buffer::empty(area);
            let mut actual = Buffer::empty(area);
            let expected_counts = draw_star_labels(&mut expected, &projected, &options, area, None, None);
            let actual_counts = draw_star_labels(
                &mut actual,
                &projected,
                &options,
                area,
                cache.prepared(),
                cache.named_candidates(),
            );
            assert_eq!(actual, expected);
            assert_eq!(
                (actual_counts.0, actual_counts.1),
                (expected_counts.0, expected_counts.1)
            );
            assert!(actual_counts.2 <= expected_counts.2);
        }
    }
}
