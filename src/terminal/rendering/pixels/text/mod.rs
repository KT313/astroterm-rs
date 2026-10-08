//! Shared text layout in cells, painted into the final bitmap for graphics protocols or merged into half-block cells.
use crate::model::{MetadataField, ProjectedSky, RenderOptions};
use crate::scene::{format_star_label, select_pixel_star_labels, planet_rgb};
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, Operation};
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
) -> Buffer {
    // assemble every text layer in memory before either image encoding or terminal output
    let mut buffer = times.measure("Text canvas", || Buffer::empty(screen));
    times.record_shape(BufferId::TextCells, Operation::Build, None, || BufferShape::vector(&buffer.content, IndexDomain::Cells)); // new ratatui storage, symbol heap payload is not counted
    times.describe("Text canvas", || {
        format!(
            "output text grid={}x{}; cells={}",
            screen.width,
            screen.height,
            buffer.content.len()
        )
    });
    let (eligible, submitted, visited) = times.measure("Star labels", || {
        draw_star_labels(&mut buffer, sky, options, area)
    });
    times.record_borrow(BufferId::TextCells, Access::Writable, || BufferShape::vector(&buffer.content, IndexDomain::Cells));
    times.describe("Star labels", || format!("input stars={}; label candidates={eligible}; skipped by label rules or missing cell={}; clipped label origins={}; submitted labels={submitted}; dynamic names={}", sky.stars.len(), sky.stars.len()-eligible, eligible-submitted, options.dynamic_names));
    times.describe("Star labels", || format!("visited candidates={visited}; selected from the brightest end; solar-system labels are independent"));
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
    {
        times.record_borrow(BufferId::MetadataFields, Access::ReadOnly, || BufferShape::slice(fields, IndexDomain::Objects));
        times.record_borrow(BufferId::TextCells, Access::Writable, || BufferShape::vector(&buffer.content, IndexDomain::Cells));
    }
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
            "fallback notice={}; accuracy warning={}; brightness-bound warning={}; final nonblank text cells={}",
            notice.is_some(),
            sky.outside_accuracy_range,
            sky.magnitude_clipping().any(),
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
) -> (usize, usize, usize) {
    let mut eligible = 0;
    let mut submitted = 0;
    let cell = |position| map_pixel_to_cell(sky, area, position);
    let candidates = select_pixel_star_labels(options, sky);
    let count = candidates.len();
    for index in candidates {
        let entry = sky.stars.get(index);
        let Some(position) = entry.cell else { continue; };
        let label = format_star_label(&entry.star, sky.names, true);
        let (row, col) = cell(position);
        let [r, g, b] = crate::scene::star_rgb(&entry.star);
        eligible += 1;
        submitted += usize::from(put_label(buffer, area, row - 1, col + 1, &label, Color::Rgb(r, g, b)));
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
        for &(position, label) in sky.horizon_labels {
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
    let notices = [
        sky.outside_accuracy_range.then_some(crate::astro::accuracy::ACCURACY_WARNING),
        sky.magnitude_clipping().any().then_some(crate::catalog::MAGNITUDE_CLIPPING_WARNING),
    ];
    let warning_rows = notices.iter().flatten().count();
    for (offset, text) in notices.into_iter().flatten().enumerate() {
        let Some(row) = screen.height.checked_sub(offset as u16 + 1) else { break; };
        Paragraph::new(text)
            .style(Style::default().fg(Color::Yellow).bg(Color::Black))
            .render(Rect::new(screen.x, screen.y + row, screen.width, 1), buffer);
    }
    if let Some(text) = notice {
        let Some(row) = screen.height.checked_sub(((warning_rows + 1).max(2)) as u16) else { return; };
        Paragraph::new(text).style(Style::default().fg(Color::Yellow).bg(Color::Black))
            .render(Rect::new(screen.x, screen.y + row, screen.width, 1), buffer);
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
    use crate::catalog::load_embedded_catalog;
    use crate::model::{ProjectionViewport as Viewport, View};
    use crate::projection::project_sky;

    #[test]
    fn pixel_notices_keep_clipping_date_and_fallback_on_separate_rows() {
        let mut parsed = load_embedded_catalog().unwrap();
        parsed.stars.truncate(1);
        let star = &mut parsed.stars[0];
        star.magnitude = -10.0;
        star.right_ascension = 0.0;
        star.declination = 0.0;
        star.space_motion = Some(crate::catalog::SpaceMotion {
            distance_pc: 10.0, position: crate::astro::Vector3 { x: 10.0, y: 0.0, z: 0.0 },
            velocity: crate::astro::Vector3 { x: -0.0001, y: 0.0, z: 0.0 },
        });
        let mut sky = crate::sky::create_sky_from_catalog(&parsed).unwrap();
        sky.outside_accuracy_range = true;
        let data = project_sky(&sky, &View::default(), Viewport { width: 200, height: 100 });
        let projected = data.view(&sky);
        assert!(projected.magnitude_clipping().any());
        let area = Rect::new(0, 0, 140, 6);
        let mut buffer = Buffer::empty(area);
        draw_notices(&mut buffer, &projected, area, Some("Protocol fallback notice"));
        let row = |y| (0..140).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(5).starts_with(crate::astro::accuracy::ACCURACY_WARNING));
        assert!(row(4).starts_with("Stored brightness bounds clipped"));
        assert!(row(3).starts_with("Protocol fallback notice"));
        for height in [0, 1, 2] { // tiny terminals clip notices without underflow or overwriting retained rows
            let area = Rect::new(0, 0, 140, height);
            draw_notices(&mut Buffer::empty(area), &projected, area, Some("fallback"));
        }
    }

    #[test]
    fn labels_visit_only_the_brightest_tail_even_with_visible_planets() {
        use crate::astro::Vector3;
        use crate::constants::DYNAMIC_NAME_COUNT;
        let mut parsed = load_embedded_catalog().unwrap();
        parsed.stars.truncate(20);
        for (i, star) in parsed.stars.iter_mut().enumerate() { star.magnitude = i as f64 * 0.25; }
        let mut sky = crate::sky::create_sky_from_catalog(&parsed).unwrap();
        for star in &mut sky.stars { star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 }; }
        for planet in &mut sky.planets { planet.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 }; }
        sky.moon.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let area = Rect::new(0, 0, 40, 20);
        let options = RenderOptions { unicode: true, braille: false, color: true, constellations: false,
            grid: false, magnitude_threshold: 5.0, dynamic_names: true };
        for (threshold, enabled, expected) in [(5.0, true, DYNAMIC_NAME_COUNT), (0.25, true, 2),
            (-1.0, true, 0), (5.0, false, 0)] {
            let options = RenderOptions { magnitude_threshold: threshold, dynamic_names: enabled, ..options };
            let projected = project_sky(&sky, &View::default(), Viewport { width: 160, height: 80 });
            let mut buffer = Buffer::empty(area);
            let (eligible, submitted, visited) = draw_star_labels(&mut buffer, &projected.view(&sky), &options, area);
            assert_eq!((eligible, submitted, visited), (expected, expected, expected));
        }
    }

    #[test]
    fn pixel_labels_remain_current_when_the_sky_image_is_reused() {
        use crate::astro::Vector3;
        use crate::catalog::StarNames;
        use crate::state::SceneCache;
        use crate::timing::StepTimes;
        let mut cache = SceneCache::default();
        let options = RenderOptions { unicode: true, braille: false, color: true, constellations: false,
            grid: false, magnitude_threshold: 5.0, dynamic_names: true };
        let area = Rect::new(0, 0, 40, 20);
        for name in ["Before", "After"] {
            let mut parsed = load_embedded_catalog().unwrap();
            parsed.stars.truncate(1);
            parsed.names = StarNames::default();
            parsed.stars[0].name = Some(parsed.names.insert(name).unwrap());
            parsed.stars[0].magnitude = 1.0;
            let mut sky = crate::sky::create_sky_from_catalog(&parsed).unwrap();
            sky.stars[0].position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            let mut data = project_sky(&sky, &View::default(), Viewport { width: 160, height: 80 });
            let projected = data.view(&sky);
            crate::scene::draw_pixels(&mut cache, &projected, &options, 0.0, &mut StepTimes::default()).unwrap();
            let mut buffer = Buffer::empty(area);
            assert_eq!(draw_star_labels(&mut buffer, &projected, &options, area), (1, 1, 1));
            assert!(buffer.content.iter().map(|cell| cell.symbol()).collect::<String>().contains(name));
            if name == "After" { assert_eq!(cache.stats().hits, 1); }

            data.stars[0].1 = (1, 1); // a clipped text origin does not force a scan for replacement labels
            assert_eq!(draw_star_labels(&mut buffer, &data.view(&sky), &options, area), (1, 0, 1));
            data.order.clear();
            assert_eq!(draw_star_labels(&mut buffer, &data.view(&sky), &options, area), (0, 0, 0));
        }
    }
}
