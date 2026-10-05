//! Value-only observations at completed rendering steps. Shapes describe direct storage, not nested payloads.
use crate::{canvas::{Canvas, Cell}, model::rendering::{SceneKey, StarKeys}, timing::memory::{BufferShape, IndexDomain}};
use crate::cache::Quality;

pub(crate) fn describe_canvas(canvas: &Canvas) -> BufferShape {
    BufferShape { len: canvas.width().checked_mul(canvas.height()), capacity: None, element_bytes: Some(std::mem::size_of::<Cell>()), domain: IndexDomain::Cells, quality: Quality::ExactPayload }
}

pub(crate) fn describe_candidate(key: &SceneKey) -> BufferShape {
    match &key.stars {
        StarKeys::Pixels(values) => BufferShape::vector(values, IndexDomain::DrawOrder),
        StarKeys::Characters { glyphs: values, .. } => BufferShape::vector(values, IndexDomain::DrawOrder),
    }
}


#[allow(clippy::ptr_arg)] // describe retained capacity without traversing candidates
#[inline]
pub(crate) fn record_scene_candidate(times: &mut crate::timing::StepTimes, buffer: crate::timing::memory::BufferId, before: Option<BufferShape>, key: &SceneKey, names: Option<&Vec<usize>>) {
    use crate::timing::memory::{Access, BufferId, Operation};
    times.record_shape(buffer, Operation::Clear, before, || {
        let mut shape = before.unwrap_or(BufferShape::unknown(IndexDomain::DrawOrder)); shape.len = Some(0); shape
    });
    times.record_borrow(BufferId::ProjectedView, Access::ReadOnly, || BufferShape::unknown(IndexDomain::DrawOrder));
    times.record_borrow(buffer, Access::Writable, || describe_candidate(key));
    if let Some(names) = names { times.record_shape(BufferId::NamedCandidates, Operation::Build, None, || BufferShape::vector(names, IndexDomain::DrawOrder)); }
    times.record_shape(buffer, Operation::Build, None, || describe_candidate(key));
    times.record_unknown(buffer, Operation::Copy); // nested arc/label payload is not rescanned
}

#[inline]
pub(crate) fn record_scene_commit(times: &mut crate::timing::StepTimes, candidate: crate::timing::memory::BufferId, output: crate::timing::memory::BufferId, outcome: crate::cache::StoreOutcome) {
    times.record_store(output, outcome);
    times.record_unknown(candidate, crate::timing::memory::Operation::Move);
}
