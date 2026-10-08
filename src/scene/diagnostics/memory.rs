//! Value-only observations at completed rendering steps. Shapes describe direct storage, not nested payloads.
use crate::canvas::{Canvas, Cell};
use crate::model::{SceneKey, StarKeys};
use crate::timing::{BufferShape, IndexDomain};
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


#[inline]
pub(crate) fn record_scene_candidate(times: &mut crate::timing::StepTimes, buffer: crate::timing::BufferId, before: Option<BufferShape>, key: &SceneKey) {
    use crate::timing::{Access, BufferId, Operation};
    times.record_shape(buffer, Operation::Clear, before, || {
        let mut shape = before.unwrap_or(BufferShape::unknown(IndexDomain::DrawOrder)); shape.len = Some(0); shape
    });
    times.record_borrow(BufferId::ProjectedView, Access::ReadOnly, || BufferShape::unknown(IndexDomain::DrawOrder));
    times.record_borrow(buffer, Access::Writable, || describe_candidate(key));
    times.record_shape(buffer, Operation::Build, None, || describe_candidate(key));
    times.record_unknown(buffer, Operation::Copy); // nested arc/label payload is not rescanned
}

#[inline]
pub(crate) fn record_scene_commit(times: &mut crate::timing::StepTimes, candidate: crate::timing::BufferId, output: crate::timing::BufferId, outcome: crate::cache::StoreOutcome) {
    times.record_store(output, outcome);
    times.record_unknown(candidate, crate::timing::Operation::Move);
}

#[inline]
pub(in crate::scene) fn record_character_initialization(times: &mut crate::timing::StepTimes, canvas: &Canvas) {
    use crate::timing::{Access, BufferId, Operation};
    times.record_borrow(BufferId::CharacterFrame, Access::Writable, || describe_canvas(canvas));
    times.record_shape(BufferId::CharacterFrame, Operation::Clear, None, || describe_canvas(canvas)); // fills existing cells with blanks; length is unchanged
}

#[inline]
pub(in crate::scene) fn record_character_stars(times: &mut crate::timing::StepTimes, canvas: &Canvas) {
    use crate::timing::{Access, BufferId};
    times.record_borrow(BufferId::ProjectedView, Access::ReadOnly, || BufferShape::unknown(IndexDomain::DrawOrder));
    times.record_borrow(BufferId::CatalogStars, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
    times.record_borrow(BufferId::CharacterFrame, Access::Writable, || describe_canvas(canvas));
}

#[inline]
pub(in crate::scene) fn record_pixel_horizon(times: &mut crate::timing::StepTimes, canvas: &tiny_skia::Pixmap, sky: &crate::model::ProjectedSky<'_>) {
    use crate::timing::{Access, BufferId};
    times.record_borrow(BufferId::ProjectedHorizon, Access::ReadOnly, || BufferShape::slice(sky.horizon, IndexDomain::Cells));
    times.record_borrow(BufferId::PixelScene, Access::Writable, || BufferShape::slice(canvas.data(), IndexDomain::Bytes));
}

#[inline]
#[allow(clippy::ptr_arg)] // report retained capacity as well as live pixels
pub(in crate::scene) fn record_star_layer(times: &mut crate::timing::StepTimes, layer: &Vec<crate::model::StarPixel>, initialized: bool) {
    use crate::timing::{Access, BufferId, Operation};
    times.record_borrow(BufferId::StarLayer, Access::Writable, || BufferShape::vector(layer, IndexDomain::Pixels));
    if initialized { times.record_shape(BufferId::StarLayer, Operation::Build, None, || BufferShape::vector(layer, IndexDomain::Pixels)); }
}

#[inline]
pub(in crate::scene) fn record_star_composition(times: &mut crate::timing::StepTimes, layer: &[crate::model::StarPixel], canvas: &tiny_skia::Pixmap) {
    use crate::timing::{Access, BufferId};
    times.record_borrow(BufferId::StarLayer, Access::ReadOnly, || BufferShape::slice(layer, IndexDomain::Pixels));
    times.record_borrow(BufferId::PixelScene, Access::Writable, || BufferShape::slice(canvas.data(), IndexDomain::Bytes));
}

#[inline]
pub(in crate::scene) fn record_pixel_constellations(times: &mut crate::timing::StepTimes, canvas: &tiny_skia::Pixmap, sky: &crate::model::ProjectedSky<'_>) {
    use crate::timing::{Access, BufferId};
    times.record_borrow(BufferId::ProjectedFigures, Access::ReadOnly, || BufferShape::slice(sky.constellations, IndexDomain::Objects));
    times.record_borrow(BufferId::PixelScene, Access::Writable, || BufferShape::slice(canvas.data(), IndexDomain::Bytes));
}

#[inline]
pub(in crate::scene) fn record_pixel_planets(times: &mut crate::timing::StepTimes, canvas: &tiny_skia::Pixmap, sky: &crate::model::ProjectedSky<'_>) {
    use crate::timing::{Access, BufferId};
    times.record_borrow(BufferId::ProjectedBodies, Access::ReadOnly, || BufferShape::slice(sky.planets, IndexDomain::Objects));
    times.record_borrow(BufferId::PixelScene, Access::Writable, || BufferShape::slice(canvas.data(), IndexDomain::Bytes));
}

#[inline]
pub(in crate::scene) fn record_pixel_initialization(times: &mut crate::timing::StepTimes, canvas: &tiny_skia::Pixmap) {
    use crate::timing::{BufferId, Operation};
    times.record_shape(BufferId::PixelScene, Operation::Build, None, || BufferShape::slice(canvas.data(), IndexDomain::Bytes)); // new pixmap, not retained-capacity reuse
}

#[inline]
pub(in crate::scene) fn record_pixel_finalization(times: &mut crate::timing::StepTimes, image: Option<&image::RgbaImage>) {
    use crate::timing::{BufferId, Operation};
    if let Some(image) = image { times.record_shape(BufferId::PixelScene, Operation::Move, None, || BufferShape::vector(image.as_raw(), IndexDomain::Bytes)); }
}
