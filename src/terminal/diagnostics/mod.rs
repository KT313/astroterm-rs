//! Rendering-boundary observations, separate from canvas assembly and terminal lifetime.
use crate::canvas::Canvas;
use crate::model::MetadataField;
use crate::scene::describe_canvas;
use crate::state::CharacterState;
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, Operation};

#[allow(clippy::ptr_arg)] // inventory needs capacity, not only live elements
#[inline]
pub(in crate::terminal) fn record_field_rebuild(times: &mut StepTimes, buffer: BufferId, before: Option<BufferShape>, fields: &Vec<MetadataField>) {
    times.record_shape(buffer, Operation::Clear, before, || { let mut shape = before.unwrap(); shape.len = Some(0); shape });
    times.record_shape(buffer, Operation::Build, before, || BufferShape::vector(fields, IndexDomain::Objects));
}

#[allow(clippy::ptr_arg)]
#[inline]
pub(in crate::terminal) fn record_metadata_append(times: &mut StepTimes, before: Option<BufferShape>, steps_before: Option<BufferShape>, fields: &Vec<MetadataField>, step_fields: &Vec<MetadataField>) {
    record_field_rebuild(times, BufferId::MetadataFields, before, fields);
    times.record_shape(BufferId::StepFields, Operation::Move, steps_before, || BufferShape::vector(step_fields, IndexDomain::Objects));
}

#[allow(clippy::ptr_arg)]
#[inline]
pub(in crate::terminal) fn record_panel(times: &mut StepTimes, fields: &Vec<MetadataField>, panel: &Canvas) {
    times.record_borrow(BufferId::MetadataFields, Access::ReadOnly, || BufferShape::vector(fields, IndexDomain::Objects));
    times.record_borrow(BufferId::PanelCanvas, Access::Writable, || describe_canvas(panel));
    times.record_unknown(BufferId::PanelCanvas, Operation::Write);
}

#[inline]
pub(in crate::terminal) fn record_character_present(times: &mut StepTimes, state: &CharacterState, had_previous: bool) {
    times.record_borrow(BufferId::CharacterFrame, Access::ReadOnly, || describe_canvas(&state.frame.sky));
    times.record_borrow(BufferId::PresenterScreen, Access::Writable, || describe_canvas(&state.presenter.screen));
    times.record_shape(BufferId::PresenterScreen, Operation::Clear, None, || describe_canvas(&state.presenter.screen));
    times.record_unknown(BufferId::PresenterScreen, Operation::Copy);
    if had_previous { times.record_unknown(BufferId::PreviousFrame, Operation::Compare); }
    times.record_shape(BufferId::PreviousFrame, Operation::Copy, None, || describe_canvas(state.presenter.previous.as_ref().expect("successful presentation saves screen")));
    times.record_unknown(BufferId::PresenterScreen, Operation::Output);
}
