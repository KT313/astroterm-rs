//! Diagnostics describe submitted geometry; overdraw means this is not a count of distinct lit pixels.
pub(super) mod memory;
use crate::model::{RenderOptions, ProjectedSky};
use crate::timing::StepTimes;

pub(super) fn describe_scene(sky: &ProjectedSky<'_>, options: &RenderOptions, times: &mut StepTimes) {
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
    times.describe("Raster stars", || {
        let bright = sky.stars.iter().filter(|s| s.star.magnitude <= options.magnitude_threshold).count();
        let placed = sky.stars.iter().filter(|s| s.star.magnitude <= options.magnitude_threshold && s.cell.is_some()).count();
        format!("input stars={}; rejected current magnitude > {}={}; then rejected missing cell={}; submitted stars={placed}; later objects may overwrite these", sky.stars.len(), options.magnitude_threshold, sky.stars.len()-bright, bright-placed)
    });
    times.describe("Raster constellations", || {
        let eligible: Vec<_> = sky.constellations.iter().filter(|c| c.maximum_magnitude <= options.magnitude_threshold).collect();
        let submitted = if options.constellations { eligible.iter().flat_map(|c| &c.arcs).map(|a| a.points.len().saturating_sub(1)).sum() } else { 0 };
        format!("input figures={}; enabled={}; skipped disabled={}; then rejected figure magnitude={}; submitted line segments={submitted}", sky.constellations.len(), options.constellations, if options.constellations { 0 } else { sky.constellations.len() }, if options.constellations { sky.constellations.len()-eligible.len() } else { 0 })
    });
    times.describe("Raster planets", || {
        format!(
            "input Sun/planets={}; rejected missing cell={}; submitted={}; no magnitude filtering",
            sky.planets.len(),
            sky.planets.iter().filter(|p| p.cell.is_none()).count(),
            sky.planets.iter().filter(|p| p.cell.is_some()).count()
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

pub(super) fn describe_minimum_stars(sky: &crate::model::ProjectedSky<'_>, options: &crate::model::RenderOptions, fast_stars: bool, times: &mut crate::timing::StepTimes) {
    times.describe("Raster stars", || {
        let tiny = sky
            .stars
            .iter()
            .filter(|s| {
                s.cell.is_some()
                    && s.star.magnitude <= options.magnitude_threshold
                    && (2.8 - 0.32 * s.star.magnitude).clamp(0.55, 4.0) as f32 == super::raster::pixels::MINIMUM_STAR_RADIUS
            })
            .count();
        format!(
            "minimum-radius stars={tiny}; fast path enabled={}; pixel bounds <=4096={}",
            fast_stars,
            sky.viewport.width <= 4096 && sky.viewport.height <= 4096
        )
    });
}

pub(super) fn describe_coverage_notice(canvas: &crate::canvas::Canvas, sky: &crate::model::ProjectedSky<'_>, times: &mut crate::timing::StepTimes) {
    times.describe("Coverage notice", || {
        format!("date warning={}; brightness-bound warning={}; output rows={}", sky.outside_accuracy_range, sky.magnitude_clipping().any(), (usize::from(sky.outside_accuracy_range) + usize::from(sky.magnitude_clipping().any())).min(canvas.height()))
    });
}
