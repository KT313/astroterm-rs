//! Assemble every sky layer and label before publishing the completed frame.
use std::io;
use crate::astro::{Observer, SimulationClock};
use crate::model::{ProjectedSky, View};
use crate::state::{CharacterState, PixelState};
use crate::timing::StepTimes;
use ratatui_image::picker::ProtocolType;
use super::TerminalSession;
use super::rendering::{
    prepare_character_timezone, prepare_character_timing_fields, rasterize_character_sky, draw_character_notice,
    draw_character_panel, present_character_frame, prepare_pixel_timezone, initialize_pixel_canvas, rasterize_pixel_sky,
    compose_pixel_sky, prepare_pixel_fields, layout_pixel_text, prepare_pixel_glyphs, paint_pixel_text, encode_pixel_cells,
    serialize_pixel_cells, present_pixel_cells, prepare_kitty_pixels,
    present_kitty_image,
};

/// Draw the projected sky and metadata as characters, then publish the changed terminal cells.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_character_frame(state: &mut CharacterState, session: &mut TerminalSession, sky: &ProjectedSky<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<()> {
    prepare_character_timezone(state, observer);                                      // look up local time rules when the panel needs them
    prepare_character_timing_fields(state, times);                                    // retain timings from before this frame is drawn
    times.measure_steps("Draw", |times| draw_character_layers(state, sky, view, date, clock, observer, times)); // assemble the sky and optional panel
    present_character_frame(state, session, times)                                    // write changed cells and flush terminal output
}

#[allow(clippy::too_many_arguments)]
fn draw_character_layers(state: &mut CharacterState, sky: &ProjectedSky<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) {
    rasterize_character_sky(state, sky, date, times);                                  // draw objects and constellation lines onto the sky canvas
    draw_character_notice(state, sky);                                                    // explain any fallback from pixel rendering
    draw_character_panel(state, sky, view, date, clock, observer, times);              // add local time, location and requested diagnostics
}

/// Blend sky and text before encoding a graphics image or composing native half-block cells.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_pixel_frame(state: &mut PixelState, session: &mut TerminalSession, sky: &ProjectedSky<'_>, prepared: Option<&crate::model::RenderProjection<'_>>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<crate::model::RenderOutcome> {
    prepare_pixel_timezone(state, observer);                                          // look up local time rules when the panel needs them
    rasterize_pixel_sky(state, sky, prepared, date, times)?;                           // draw objects and constellation lines in the sky area

    prepare_pixel_fields(state, sky, view, date, clock, observer, times);              // collect metadata and the available stage timings
    let text_cell = layout_pixel_text(state, sky, prepared, times);                   // position object names, metadata and notices
    if state.protocol == ProtocolType::Kitty {
        prepare_kitty_pixels(state, text_cell, times)?;                               // reuse completed RGB or compose changed sky and text
        return present_kitty_image(state, session.output(), times);                           // upload Kitty images before swapping them
    }

    initialize_pixel_canvas(state, times)?;                                           // other protocols consume their completed image during encoding
    compose_pixel_sky(state, times);                                                  // place the sky into the full terminal image
    prepare_pixel_glyphs(state, times);                                               // discard saved letter shapes when reuse is disabled
    paint_pixel_text(state, text_cell, times);                                        // blend letters into the graphics image when present

    encode_pixel_cells(state, times)?;                                               // encode the image or merge text with half-block cells
    serialize_pixel_cells(state, times)?;                                            // assemble the complete terminal command sequence
    present_pixel_cells(state, session, times)?;                                       // write and flush the completed frame
    Ok(crate::model::RenderOutcome::Presented)
}
