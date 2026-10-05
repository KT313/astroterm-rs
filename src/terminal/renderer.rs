//! The terminal renderer: draws the sky as a grid of characters and shows it in the terminal.

use std::io;

use crate::astro::{Observer, SimulationClock};
use crate::metadata::{fill_metadata_fields, append_step_time_fields};
use crate::model::projection::{ProjectedSky, View};
use crate::model::projection::ProjectionViewport as Viewport;
use crate::model::rendering::{RenderOptions};
use crate::scene::{draw_metadata_panel};
use crate::timing::StepTimes;
use crate::timing::memory::{BufferId, BufferShape, IndexDomain};
use super::memory::{record_field_rebuild, record_metadata_append, record_panel, record_character_present};

use super::session::{TerminalSession, open_terminal_session};

use crate::model::config::TerminalSettings;

use crate::state::rendering::{CharacterState, Presenter};

/// Take over the terminal and size the canvases to it.
pub fn open_terminal_renderer(options: RenderOptions, settings: TerminalSettings) -> io::Result<(TerminalSession, CharacterState)> {
    let mut session = open_terminal_session()?;
    let mut presenter = Presenter::default();
    let frame = session.fit_frame(&mut presenter, settings.aspect_ratio, settings.metadata_panel)?;
    Ok((session, CharacterState {
        scene_cache: Default::default(),
        cache_diagnostics: Default::default(),
        frame,
        options,
        settings,
        time_zone: None,
        startup_notice: None,
        presenter,
        fields: Vec::new(),
        step_fields: Vec::new(),
    }))
}

pub fn character_viewport(state: &CharacterState) -> Viewport {
    Viewport {
        height: state.frame.sky.height(),
        width: state.frame.sky.width(),
    }
}

/// Resize the canvases to the terminal, after it was resized. The next frame is drawn in full.
pub fn fit_character_terminal(state: &mut CharacterState, session: &mut TerminalSession) -> io::Result<()> {
    state.scene_cache.invalidate();
    state.frame = session.fit_frame(&mut state.presenter, state.settings.aspect_ratio, state.settings.metadata_panel)?;
    Ok(())
}

/// Draw the sky as seen in `view`, with the metadata panel on top, and show it. `julian_date_utc` is the simulation
/// time the sky's positions were computed for. The durations of drawing and presenting are added to `step_times`;
/// the panel shows them as of the previous frame.
#[allow(clippy::too_many_arguments)]
pub fn render_character_frame(
    state: &mut CharacterState,
    session: &mut TerminalSession,
    sky: &ProjectedSky<'_>,
    view: &View,
    julian_date_utc: f64,
    clock: &SimulationClock,
    observer: &Observer,
    step_times: &mut StepTimes,
) -> io::Result<()> {
    // resolve geographic zone rules only when a panel needs them and the observer changes
    if state.frame.panel.is_some() && state.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
        state.time_zone = Some((*observer, crate::metadata::resolve_observer_timezone(observer)));
    }

    // the step durations so far, read before this frame's drawing is measured
    let step_fields_before = step_times.inspect_memory(|| BufferShape::vector(&state.step_fields, IndexDomain::Objects));
    step_times.measure_memory_scope("Timing fields", |step_times| {
        state.step_fields.clear();
        if state.settings.frame_times { append_step_time_fields(&mut state.step_fields, step_times.steps()); }
    });
    record_field_rebuild(step_times, BufferId::StepFields, step_fields_before, &state.step_fields);

    // draw the sky and the panel, then write the changes to the terminal
    step_times.measure_steps("Draw", |step_times| {
        step_times.measure_steps("Raster", |step_times| {
            crate::scene::cached::draw_characters_with_times(&mut state.scene_cache, &mut state.frame.sky,
                sky,
                &state.options,
                crate::model::simulation::FrameTime::from_utc(julian_date_utc).tt,
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
        if let Some(notice) = &state.startup_notice {
            let row = state.frame.sky.height().saturating_sub(2) as i32;
            state.frame
                .sky
                .put_str_truncated(row, 0, notice, Some(crate::canvas::Color::Yellow));
        }
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
                    state.fields.push(crate::model::metadata::MetadataField {
                        label: "Obs cache".into(),
                        value: state.cache_diagnostics[0].clone(),
                    });
                    state.fields.push(crate::model::metadata::MetadataField {
                        label: "Proj cache".into(),
                        value: state.cache_diagnostics[1].clone(),
                    });
                    state.fields.push(crate::model::metadata::MetadataField {
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
                        state.fields.push(crate::model::metadata::MetadataField {
                            label: label.into(),
                            value: count.to_string(),
                        });
                    }
                    if sky.selection.brute_force {
                        state.fields.push(crate::model::metadata::MetadataField {
                            label: "Selection".into(),
                            value: "outside interval: all stars".into(),
                        });
                    }
                    state.fields.push(crate::model::metadata::MetadataField {
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
    });
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
