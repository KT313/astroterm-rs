//! Diagnostics describe submitted geometry; overdraw means this is not a count of distinct lit pixels.
pub(super) mod memory;
use crate::model::{RenderOptions, ProjectedSky};
use crate::timing::StepTimes;

pub(super) fn describe_scene(sky: &ProjectedSky<'_>, options: &RenderOptions, times: &mut StepTimes) {
    times.describe("Raster stars", || {
        let bright = count_drawable_stars(sky, options);
        format!("input stars={}; rejected current magnitude > {}={}; submitted stars={bright}; later objects may overwrite these", sky.stars.len(), options.magnitude_threshold, sky.stars.len()-bright)
    });
    describe_scene_geometry(sky, options, times);
}

fn describe_scene_geometry(sky: &ProjectedSky<'_>, options: &RenderOptions, times: &mut StepTimes) {
    times.describe("Canvas initialization", || {
        format!(
            "output canvas={}x{}; elements={}; background initialized",
            sky.viewport.width,
            sky.viewport.height,
            sky.viewport.width * sky.viewport.height
        )
    });
    times.describe("Raster horizon", || {
        format!(
            "input segments={}; submitted={}",
            sky.horizon.len(),
            if sky.facing { sky.horizon.len() } else { 0 }
        )
    });
    times.describe("Raster constellations", || {
        let (eligible, submitted) = sky.constellations.iter().filter(|c| c.maximum_magnitude <= options.magnitude_threshold).fold((0, 0), |(count, submitted), figure| {
            let segments = if options.constellations { figure.arcs.iter().map(|arc| arc.points.len().saturating_sub(1)).sum() } else { 0 };
            (count + 1, submitted + segments)
        });
        format!("input figures={}; enabled={}; skipped disabled={}; then rejected figure magnitude={}; submitted line segments={submitted}", sky.constellations.len(), options.constellations, if options.constellations { 0 } else { sky.constellations.len() }, if options.constellations { sky.constellations.len()-eligible } else { 0 })
    });
    times.describe("Raster planets", || {
        let submitted = sky.planets.iter().filter(|planet| planet.cell.is_some()).count();
        format!(
            "input Sun/planets={}; rejected missing cell={}; submitted={}; no magnitude filtering",
            sky.planets.len(),
            sky.planets.len() - submitted,
            submitted
        )
    });
    times.describe("Raster moon", || {
        format!(
            "input Moon=1; submitted={}; illuminated fraction={:.8}",
            usize::from(sky.moon.cell.is_some()),
            sky.moon.illumination.illuminated_fraction
        )
    });
    times.describe("Raster grid", || {
        format!(
            "enabled={}; facing={}; submitted radial lines={}",
            options.grid,
            sky.facing,
            if options.grid && !sky.facing { 12 } else { 0 }
        )
    });
    times.describe("Orientation labels", || {
        format!(
            "facing={}; grid={}; horizon label inputs={}",
            sky.facing,
            options.grid,
            sky.horizon_labels.len()
        )
    });
    times.describe("Raster finalization", || {
        format!(
            "output RGBA={}x{}; bytes={}",
            sky.viewport.width,
            sky.viewport.height,
            sky.viewport.width * sky.viewport.height * 4
        )
    });
}

pub(super) fn describe_pixel_scene(sky: &ProjectedSky<'_>, options: &RenderOptions, pixels: usize, submitted: usize, opacities: &crate::model::StarOpacityTable, rebuilt: bool, times: &mut StepTimes) {
    describe_scene_geometry(sky, options, times);
    times.describe("Star brightness preparation", || format!("field of view={} degrees; star opacity multiplier={}; opacity table entries={}; rebuilt={rebuilt}; catalog magnitudes unchanged", sky.fov_degrees, opacities.zoom_boost, opacities.opacities.len()));
    times.describe("Star layer initialization", || format!("pixels={pixels}; element bytes={}; premultiplied f32 RGB and opacity; reusable capacity", std::mem::size_of::<crate::model::StarPixel>()));
    times.describe("Raster stars", || {
        let bright = count_drawable_stars(sky, options);
        format!("input stars={}; rejected magnitude={}; then omitted edge stars={}; submitted stars={submitted}; four pixels per star; read from the regions' drawn records", sky.stars.len(), sky.stars.len()-bright, bright-submitted)
    });
    times.describe("Star layer composition", || format!("input star pixels={pixels}; premultiplied RGB over the background; nonempty pixels raised to minimum opacity={}; output scene opacity=1", crate::constants::MIN_STAR_PIXEL_OPACITY));
}

pub(super) fn describe_coverage_notice(canvas: &crate::canvas::Canvas, sky: &crate::model::ProjectedSky<'_>, times: &mut crate::timing::StepTimes) {
    times.describe("Coverage notice", || {
        format!("date warning={}; brightness-bound warning={}; output rows={}", sky.outside_accuracy_range, sky.magnitude_clipping().any(), (usize::from(sky.outside_accuracy_range) + usize::from(sky.magnitude_clipping().any())).min(canvas.height()))
    });
}

/// Every projected star has a cell, so this is the brightness filter alone; the drawn records are read directly.
fn count_drawable_stars(sky: &ProjectedSky<'_>, options: &RenderOptions) -> usize {
    sky.stars.drawn().filter(|star| star.magnitude <= options.magnitude_threshold).count()
}
