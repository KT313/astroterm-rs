//! Reuse complete Kitty pixels and transport bytes only after their inputs have been published.
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
        allow_image_reuse(state) && state.frame_key == Some(key) && state.rgb_version.current().is_some()
    });
    if reuse {
        times.record_shape(BufferId::RgbImage, Operation::Reuse, None, || BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes));
        times.describe("Full image cache decision", || "reused completed RGB; composition, text painting and conversion skipped".into());
        return Ok(());
    }

    invalidate_pixels(state);                                                       // no failed or partially rebuilt image can be published
    initialize_pixel_canvas(state, times)?;                                         // reset reusable RGBA storage to the opaque background
    compose_pixel_sky(state, times);                                                // copy the completed sky without changing its cache
    prepare_pixel_glyphs(state, times);                                             // prepare letter shapes only when painting is required
    paint_pixel_text(state, text_cell, times);                                       // paint onto the freshly reset image
    convert_kitty_pixels(state, times)?;                                            // refill RGB storage; publish only after conversion succeeds
    state.frame_key = Some(key);
    Ok(())
}

fn invalidate_pixels(state: &mut PixelState) {
    state.rgb_version.invalidate();
    state.frame_key = None;
    state.encoding_key = None;
}

pub(super) fn convert_kitty_pixels(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    invalidate_pixels(state);
    let before = times.inspect_memory(|| BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes));
    times.measure("Pixel conversion", || {
        let frame = state.frame_image.as_ref().expect("graphics frame initialized");
        let mut bytes = std::mem::take(&mut state.rgb).into_raw();
        let length = frame.width() as usize * frame.height() as usize * 3;
        bytes.try_reserve(length.saturating_sub(bytes.len())).map_err(io::Error::other)?;
        bytes.resize(length, 0);
        for (source, target) in frame.as_raw().chunks_exact(4).zip(bytes.chunks_exact_mut(3)) {
            target.copy_from_slice(&source[..3]);                                    // alpha has already been blended onto the opaque background
        }
        state.rgb = image::RgbImage::from_raw(frame.width(), frame.height(), bytes).expect("validated RGB length");
        Ok::<_, io::Error>(())
    })?;
    state.rgb_version.publish(true);
    times.record_borrow(BufferId::FrameImage, Access::ReadOnly, || BufferShape::vector(state.frame_image.as_ref().unwrap().as_raw(), IndexDomain::Bytes));
    times.record_shape(BufferId::RgbImage, Operation::Build, before, || BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes));
    times.describe("Pixel conversion", || format!("input RGBA bytes={}; output RGB bytes={}; dimensions={}x{}; storage retained for reuse",
        state.rgb.width() as usize * state.rgb.height() as usize * 4, state.rgb.len(), state.rgb.width(), state.rgb.height()));
    Ok(())
}

fn allow_image_reuse(state: &PixelState) -> bool {
    state.reuse_assets && state.scene_cache.config.allows(Group::Raster) && state.scene_cache.config.allows(Group::RasterAssets)
}

/// Changed images are encoded for their target ID; compression working memory survives across images.
pub(in crate::terminal) fn encode_kitty_upload(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    encode_kitty_upload_with(state, times, |state| kitty::encode_upload_reusing(&state.rgb, state.kitty_image_id,
        state.compression == CompressionSupport::Supported, state.tmux, &mut state.compressor, &mut state.compressed, &mut state.upload))
}

fn encode_kitty_upload_with(state: &mut PixelState, times: &mut StepTimes, encode: impl FnOnce(&mut PixelState) -> io::Result<()>) -> io::Result<()> {
    let Some(rgb_version) = state.rgb_version.current() else {
        state.encoding_key = None;
        return Err(io::Error::other("cannot encode incomplete Kitty pixels"));
    };
    let key = KittyEncodingKey {
        rgb_version, dimensions: state.rgb.dimensions(), tmux: state.tmux, image_id: state.kitty_image_id,
        compression: match state.compression { CompressionSupport::Supported => 1, CompressionSupport::Unsupported => 2, CompressionSupport::Unknown => 0 },
    };
    let reuse = times.measure("Image encoding cache decision", || allow_image_reuse(state) && state.encoding_key == Some(key));
    if reuse {
        times.record_shape(BufferId::UploadBytes, Operation::Reuse, None, || describe_upload(state));
        times.describe("Image encoding cache decision", || format!("reused {} upload bytes for image {}", state.upload.len(), state.kitty_image_id));
        return Ok(());
    }

    state.encoding_key = None;                                                      // partial compression or commands must not be reused on retry
    let before = times.inspect_memory(|| (describe_upload(state), BufferShape::vector(&state.compressed, IndexDomain::Bytes)));
    let reused_engine = state.compressor.is_some();
    let result = times.measure("Image encoding", || encode(state));
    if state.compression == CompressionSupport::Supported && state.compressor.is_some() {
        times.record_unknown(BufferId::CompressionEngine, if reused_engine { Operation::Reuse } else { Operation::Build });
    }
    times.record_borrow(BufferId::RgbImage, Access::ReadOnly, || BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes));
    times.record_shape(BufferId::UploadBytes, Operation::Clear, before.map(|s| s.0), || { let mut shape = before.unwrap().0; shape.len = Some(0); shape });
    times.record_shape(BufferId::CompressedBytes, Operation::Clear, before.map(|s| s.1), || { let mut shape = before.unwrap().1; shape.len = Some(0); shape });
    times.record_shape(BufferId::UploadBytes, Operation::Build, before.map(|s| s.0), || describe_upload(state));
    if state.compression == CompressionSupport::Supported {
        times.record_shape(BufferId::CompressedBytes, Operation::Build, before.map(|s| s.1), || BufferShape::vector(&state.compressed, IndexDomain::Bytes));
    }
    result?;
    state.encoding_key = Some(key);
    times.describe("Image encoding", || format!("input RGB bytes={}; compression={:?}; output protocol bytes={}; image ID={}; tmux={}",
        state.rgb.len(), state.compression, state.upload.len(), state.kitty_image_id, state.tmux));
    Ok(())
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
