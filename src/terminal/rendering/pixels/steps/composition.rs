//! Reuse the complete opaque Kitty frame and its transport bytes only after their inputs have been published.
use std::io;
use crate::cache::Group;
use crate::model::{KittyEncodingKey, PixelFrameKey};
use crate::state::{CompressionSupport, PixelState};
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, Operation, StepTimes};
use crate::terminal::transport::graphics::kitty;
use super::{compose_pixel_sky, initialize_pixel_canvas, paint_pixel_text, prepare_pixel_glyphs, describe_upload};
use super::super::layout::validate_frame_size;

/// Text and sky belong to this same PixelState, so their owner-local versions cannot alias another renderer.
/// Incomplete inputs cannot publish an image or allow two missing versions to match.
pub(in crate::terminal) fn prepare_kitty_pixels(state: &mut PixelState, text_cell: (u16, u16), times: &mut StepTimes) -> io::Result<()> {
    let dimensions = match validate_frame_size(state.screen, state.font) {
        Ok(value) => value,
        Err(error) => { invalidate_pixels(state); return Err(error); }
    };
    let Some(sky_version) = state.scene_cache.ready_pixel_generation() else {
        invalidate_pixels(state);
        return Err(io::Error::other("cannot compose incomplete sky pixels"));
    };
    let Some(text_version) = state.text_version.current() else {
        invalidate_pixels(state);
        return Err(io::Error::other("cannot compose incomplete text layout"));
    };
    let key = PixelFrameKey {
        sky_version, text_version, dimensions,
        screen: [state.screen.x, state.screen.y, state.screen.width, state.screen.height],
        sky_area: [state.area.x, state.area.y, state.area.width, state.area.height],
        font: (state.font.width, state.font.height), text_cell,
        background: crate::constants::PIXEL_BACKGROUND_RGBA,
    };
    let reuse = times.measure("Full image cache decision", || {
        allow_image_reuse(state) && state.frame_key == Some(key) && state.frame_version.current().is_some() && state.frame_image.is_some()
    });
    if reuse {
        times.record_shape(BufferId::FrameImage, Operation::Reuse, None, || BufferShape::vector(state.frame_image.as_ref().unwrap().as_raw(), IndexDomain::Bytes));
        times.describe("Full image cache decision", || "reused completed frame; composition and text painting skipped".into());
        return Ok(());
    }

    invalidate_pixels(state);                                                       // no failed or partially rebuilt image can be published
    initialize_pixel_canvas(state, times)?;                                         // size the reusable RGBA storage for this frame
    compose_pixel_sky(state, times);                                                // copy the completed sky and paint the background around it
    prepare_pixel_glyphs(state, times);                                             // prepare letter shapes only when painting is required
    paint_pixel_text(state, text_cell, times);                                       // paint onto the freshly reset image
    state.frame_version.publish(true);                                              // the opaque frame is complete; Kitty uploads these bytes directly
    state.frame_key = Some(key);
    Ok(())
}

fn invalidate_pixels(state: &mut PixelState) {
    state.frame_version.invalidate();
    state.frame_key = None;
    state.encoding_key = None;
}

/// The frame whose version is published; both are owned by the same PixelState, so a ready version implies pixels.
pub(super) fn completed_frame(state: &PixelState) -> Option<(u64, &image::RgbaImage)> {
    state.frame_version.current().zip(state.frame_image.as_ref())
}

/// Drop the constant alpha into the RGB scratch; both Kitty transports send a quarter fewer bytes that way.
/// Runs only when an upload is actually prepared, so unchanged frames never convert.
pub(super) fn convert_kitty_pixels(state: &mut PixelState, times: &mut StepTimes) {
    let before = times.inspect_memory(|| BufferShape::vector(&state.rgb, IndexDomain::Bytes));
    let frame = state.frame_image.as_ref().expect("completed frame");
    times.measure("Pixel conversion", || kitty::strip_alpha(frame, &mut state.rgb));
    times.record_borrow(BufferId::FrameImage, Access::ReadOnly, || BufferShape::vector(frame.as_raw(), IndexDomain::Bytes));
    times.record_shape(BufferId::RgbImage, Operation::Build, before, || BufferShape::vector(&state.rgb, IndexDomain::Bytes));
    times.describe("Pixel conversion", || format!("input RGBA bytes={}; output RGB bytes={}; alpha dropped without blending; storage retained for reuse", frame.len(), state.rgb.len()));
}

fn allow_image_reuse(state: &PixelState) -> bool {
    state.reuse_assets && state.scene_cache.config.allows(Group::Raster) && state.scene_cache.config.allows(Group::RasterAssets)
}

/// Changed images are encoded for their target ID; compression working memory survives across images.
pub(in crate::terminal) fn encode_kitty_upload(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    encode_kitty_upload_with(state, times, |state| kitty::encode_upload_reusing(&state.rgb, state.frame_image.as_ref().expect("completed frame").dimensions(), state.kitty_image_id,
        state.compression == CompressionSupport::Supported, state.tmux, &mut state.compressor, &mut state.compressed, &mut state.upload))
}

fn encode_kitty_upload_with(state: &mut PixelState, times: &mut StepTimes, encode: impl FnOnce(&mut PixelState) -> io::Result<()>) -> io::Result<()> {
    let Some((frame_version, frame)) = completed_frame(state) else {
        state.encoding_key = None;
        return Err(io::Error::other("cannot encode incomplete Kitty pixels"));
    };
    let key = KittyEncodingKey {
        frame_version, dimensions: frame.dimensions(), tmux: state.tmux, image_id: state.kitty_image_id,
        compression: match state.compression { CompressionSupport::Supported => 1, CompressionSupport::Unsupported => 2, CompressionSupport::Unknown => 0 },
    };
    let reuse = times.measure("Image encoding cache decision", || allow_image_reuse(state) && state.encoding_key == Some(key));
    if reuse {
        times.record_shape(BufferId::UploadBytes, Operation::Reuse, None, || describe_upload(state));
        times.describe("Image encoding cache decision", || format!("reused {} upload bytes for image {}", state.upload.len(), state.kitty_image_id));
        return Ok(());
    }

    state.encoding_key = None;                                                      // partial compression or commands must not be reused on retry
    convert_kitty_pixels(state, times);                                             // the streamed payload is RGB; the shared-memory path converts before its copy
    let before = times.inspect_memory(|| (describe_upload(state), BufferShape::vector(&state.compressed, IndexDomain::Bytes)));
    let reused_engine = state.compressor.is_some();
    let result = times.measure("Image encoding", || encode(state));
    if state.compression == CompressionSupport::Supported && state.compressor.is_some() {
        times.record_unknown(BufferId::CompressionEngine, if reused_engine { Operation::Reuse } else { Operation::Build });
    }
    times.record_borrow(BufferId::FrameImage, Access::ReadOnly, || BufferShape::vector(state.frame_image.as_ref().unwrap().as_raw(), IndexDomain::Bytes));
    times.record_shape(BufferId::UploadBytes, Operation::Clear, before.map(|s| s.0), || { let mut shape = before.unwrap().0; shape.len = Some(0); shape });
    times.record_shape(BufferId::CompressedBytes, Operation::Clear, before.map(|s| s.1), || { let mut shape = before.unwrap().1; shape.len = Some(0); shape });
    times.record_shape(BufferId::UploadBytes, Operation::Build, before.map(|s| s.0), || describe_upload(state));
    if state.compression == CompressionSupport::Supported {
        times.record_shape(BufferId::CompressedBytes, Operation::Build, before.map(|s| s.1), || BufferShape::vector(&state.compressed, IndexDomain::Bytes));
    }
    result?;
    state.encoding_key = Some(key);
    times.describe("Image encoding", || format!("input RGBA bytes={}; compression={:?}; output protocol bytes={}; image ID={}; tmux={}",
        state.frame_image.as_ref().map_or(0, |frame| frame.len()), state.compression, state.upload.len(), state.kitty_image_id, state.tmux));
    Ok(())
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
