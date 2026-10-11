//! Character renderer creation and terminal resizing.
use std::io;
use crate::model::{TerminalSettings, RenderOptions, ProjectionViewport as Viewport};
use crate::state::{CharacterState, Presenter};
use crate::terminal::{TerminalSession, open_terminal_session};

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

