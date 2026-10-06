//! Individual character-frame stages and their existing diagnostic boundaries.
use std::io;

use crate::astro::{Observer, SimulationClock};
use crate::metadata::{fill_metadata_fields, append_step_time_fields};
use crate::model::{ProjectedSky, View};
use crate::scene::{draw_metadata_panel};
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain};
use crate::terminal::diagnostics::{record_field_rebuild, record_metadata_append, record_panel, record_character_present};

use crate::terminal::TerminalSession;


use crate::state::CharacterState;

pub(in crate::terminal) fn prepare_character_timezone(state: &mut CharacterState, observer: &Observer) {
    if state.frame.panel.is_some() && state.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
        state.time_zone = Some((*observer, crate::metadata::resolve_observer_timezone(observer)));
    }
}

pub(in crate::terminal) fn prepare_character_timing_fields(state: &mut CharacterState, step_times: &mut StepTimes) {
    let step_fields_before = step_times.inspect_memory(|| BufferShape::vector(&state.step_fields, IndexDomain::Objects));
    step_times.measure_memory_scope("Timing fields", |step_times| {
        state.step_fields.clear();
        if state.settings.frame_times { append_step_time_fields(&mut state.step_fields, step_times.steps()); }
    });
    record_field_rebuild(step_times, BufferId::StepFields, step_fields_before, &state.step_fields);
}

pub(in crate::terminal) fn rasterize_character_sky(state: &mut CharacterState, sky: &ProjectedSky<'_>, julian_date_utc: f64, step_times: &mut StepTimes) {
    step_times.measure_steps("Raster", |step_times| {
        crate::scene::draw_characters_with_times(&mut state.scene_cache, &mut state.frame.sky,
            sky,
            &state.options,
            crate::model::FrameTime::from_utc(julian_date_utc).tt,
            step_times)
    });
    step_times.describe("Raster", || {
        format!(
            "canvas={}x{} cells; cache={:?}",
            state.frame.sky.width(),
            state.frame.sky.height(),
            state.scene_cache.stats()
        )
    });
}

pub(in crate::terminal) fn draw_character_notice(state: &mut CharacterState) {
    if let Some(notice) = &state.startup_notice {
        let row = state.frame.sky.height().saturating_sub(2) as i32;
        state.frame
            .sky
            .put_str_truncated(row, 0, notice, Some(crate::canvas::Color::Yellow));
    }
}

pub(in crate::terminal) fn draw_character_panel(state: &mut CharacterState, sky: &ProjectedSky<'_>, view: &View, julian_date_utc: f64, clock: &SimulationClock, observer: &Observer, step_times: &mut StepTimes) {
    if let Some(panel) = &mut state.frame.panel {
        let fields_before = step_times.inspect_memory(|| BufferShape::vector(&state.fields, IndexDomain::Objects));
        let step_fields_filled = step_times.inspect_memory(|| BufferShape::vector(&state.step_fields, IndexDomain::Objects));
        step_times.measure_memory_scope("Metadata fields", |_| {
            fill_metadata_fields(
                &mut state.fields,
                julian_date_utc,
                clock,
                sky.moon.phase,
                observer,
                view,
                state.options.unicode,
                &state.time_zone.as_ref().expect("panel zone initialized").1,
            );
            if state.settings.frame_times {
                state.fields.push(crate::model::MetadataField {
                    label: "Obs cache".into(),
                    value: state.cache_diagnostics[0].clone(),
                });
                state.fields.push(crate::model::MetadataField {
                    label: "Proj cache".into(),
                    value: state.cache_diagnostics[1].clone(),
                });
                state.fields.push(crate::model::MetadataField {
                    label: "Raster cache".into(),
                    value: crate::cache::format_stats(state.scene_cache.stats()),
                });
                for (label, count) in [
                    ("Candidate cells", sky.selection.cells),
                    ("Candidate stars", sky.selection.candidates),
                    ("Evaluated stars", sky.evaluated_stars),
                    ("Correction skips", sky.correction_stats.skipped),
                    ("Endpoint only", sky.correction_stats.endpoint_only),
                ] {
                    state.fields.push(crate::model::MetadataField {
                        label: label.into(),
                        value: count.to_string(),
                    });
                }
                if sky.selection.brute_force {
                    state.fields.push(crate::model::MetadataField {
                        label: "Selection".into(),
                        value: "outside interval: all stars".into(),
                    });
                }
                state.fields.push(crate::model::MetadataField {
                    label: "Star fallbacks".into(),
                    value: format!(
                        "{} catalog, {} frame",
                        sky.catalog_singular_count, sky.runtime_singular_count
                    ),
                });
            }
            state.fields.append(&mut state.step_fields);
        });
        record_metadata_append(step_times, fields_before, step_fields_filled, &state.fields, &state.step_fields);
        step_times.measure("Metadata panel", || draw_metadata_panel(panel, &state.fields));
        record_panel(step_times, &state.fields, panel);
        step_times.describe("Metadata panel", || {
            format!(
                "input fields={}; panel={}x{} cells; clipped to panel",
                state.fields.len(),
                panel.width(),
                panel.height()
            )
        });
    }
}

pub(in crate::terminal) fn present_character_frame(state: &mut CharacterState, session: &mut TerminalSession, step_times: &mut StepTimes) -> io::Result<()> {
    let had_previous = step_times.inspect_memory(|| state.presenter.previous.is_some()).unwrap_or(false);
    step_times.measure("Present", || session.present(&mut state.presenter, &state.frame))?;
    record_character_present(step_times, state, had_previous);
    step_times.describe("Present", || {
        format!(
            "one character frame submitted; sky={}x{} cells; panel={}; character diff and flush included",
            state.frame.sky.width(),
            state.frame.sky.height(),
            state.frame.panel.is_some()
        )
    });
    Ok(())
}
