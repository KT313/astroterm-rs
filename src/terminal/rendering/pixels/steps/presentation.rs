//! Keep the completed Kitty image displayed until its pixels or placement change.
use std::io::{self, Write};
use crate::{cache::Group, model::{KittyDisplayKey, RenderOutcome}, state::{PixelState, CompressionSupport},
    timing::{BufferId, BufferShape, IndexDomain, Operation, StepTimes}};
use crate::terminal::transport::graphics::{kitty, present_frame};
use super::{encode_kitty_upload, serialize_kitty_swap};
use super::composition::completed_frame;
use super::shared_memory::{prepare_shared_upload, upload_kitty_pixels};

#[cfg(all(test, unix))]
#[path = "shared_tests.rs"]
mod shared_tests;

pub(in crate::terminal) fn present_kitty_image(state: &mut PixelState, out: &mut impl Write, times: &mut StepTimes) -> io::Result<RenderOutcome> {
    let Some((frame_version, frame)) = completed_frame(state) else { return Err(io::Error::other("cannot present incomplete Kitty pixels")); };
    let mut key = KittyDisplayKey { shared_memory: state.shared_memory, frame_version, dimensions: frame.dimensions(),
        screen: [state.screen.x, state.screen.y, state.screen.width, state.screen.height], tmux: state.tmux,
        compression: match state.compression { CompressionSupport::Supported => 1, CompressionSupport::Unsupported => 2, CompressionSupport::Unknown => 0 } };
    let reuse = times.measure("Presentation decision", || state.display_valid && state.displayed_key == Some(key)
        && state.reuse_assets && state.scene_cache.config.allows(Group::Raster) && state.scene_cache.config.allows(Group::RasterAssets));
    times.describe("Presentation decision", || format!("existing terminal image reused={reuse}; frame revision={frame_version}"));
    if reuse { return Ok(RenderOutcome::ReusedDisplayedFrame); }                      // no encoding, writes, flushes, swap or image-ID advance

    if !prepare_shared_upload(state, times) { encode_kitty_upload(state, times)?; }
    serialize_kitty_swap(state, times)?;
    state.display_valid = false;                                                     // even a partially failed write makes terminal contents uncertain
    let result = times.measure_steps("Present", |times| -> io::Result<usize> {
        let uploaded = upload_kitty_pixels(state, out, times)?;
        times.measure("Image swap", || present_frame(out, &state.serialized))?;
        times.record_shape(BufferId::SerializedBytes, Operation::Output, None, || BufferShape::vector(&state.serialized, IndexDomain::Bytes));
        times.describe("Image swap", || format!("bytes written/flushed={}; place completed image then delete prior image; frames=1", state.serialized.len()));
        Ok(uploaded + state.serialized.len())
    });
    state.shared_upload = None; // cleanup also runs when an upload or swap write failed
    let written = result?;
    key.shared_memory = state.shared_memory; // consumption failure may have selected the streaming fallback
    state.displayed_key = Some(key);                                                 // commit only after both writes and flushes succeed
    state.display_valid = true;
    state.kitty_image_id = kitty::other_image_id(state.kitty_image_id);
    times.describe("Present", || format!("Kitty total protocol bytes={}; frames=1", written));
    Ok(RenderOutcome::Presented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use ratatui_image::picker::ProtocolType;

    #[derive(Default)]
    struct Writer { bytes: Vec<u8>, writes: usize, flushes: usize, fail_write: Option<usize>, fail_flush: Option<usize> }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.fail_write == Some(self.writes) { return Err(io::Error::other("injected write failure")); }
            self.bytes.extend_from_slice(bytes); Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.fail_flush == Some(self.flushes) { return Err(io::Error::other("injected flush failure")); }
            Ok(())
        }
    }
    fn pixels() -> PixelState {
        let mut state = super::super::lifetime_tests::pixels(ProtocolType::Kitty);
        state.frame_image = Some(image::RgbaImage::from_pixel(16, 8, image::Rgba([21, 42, 84, 255])));
        state.frame_version.publish(true);
        state
    }
    fn present(state: &mut PixelState, writer: &mut Writer) -> io::Result<RenderOutcome> {
        present_kitty_image(state, writer, &mut StepTimes::with_trace(true))
    }

    #[test]
    fn unchanged_display_skips_encoding_writes_flushes_and_id_changes() {
        let mut state = pixels(); let mut out = Writer::default();
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        let before = (out.bytes.len(), out.writes, out.flushes, state.kitty_image_id);
        for _ in 0..3 {
            let mut times = StepTimes::with_trace(true);
            assert_eq!(present_kitty_image(&mut state, &mut out, &mut times).unwrap(), RenderOutcome::ReusedDisplayedFrame);
            assert!(times.trace().unwrap().steps.iter().all(|step| !["Image encoding", "Image upload", "Image swap", "Present"].contains(&step.name)));
            assert_eq!((out.bytes.len(), out.writes, out.flushes, state.kitty_image_id), before);
        }
        state.frame_version.publish(true);
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        assert_eq!(out.flushes, before.2 + 2);
    }

    #[test]
    fn display_invalidation_placement_transport_and_bypass_force_output() {
        let mut state = pixels(); let mut out = Writer::default();
        present(&mut state, &mut out).unwrap();
        state.display_valid = false;
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        state.screen.x += 1;
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        state.tmux = true;
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        state.compression = CompressionSupport::Unsupported;
        assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented);
        state.scene_cache.configure(&CacheConfig::disabled());
        for _ in 0..2 { assert_eq!(present(&mut state, &mut out).unwrap(), RenderOutcome::Presented); }
        state.frame_version.invalidate();
        let bytes = out.bytes.len();
        assert!(present(&mut state, &mut out).is_err());
        assert_eq!(out.bytes.len(), bytes);
    }

    #[test]
    fn failed_upload_or_swap_preserves_last_success_and_id_but_invalidates_display() {
        for (fail_write, fail_flush) in [(Some(1), None), (Some(2), None), (None, Some(1)), (None, Some(2))] {
            let mut state = pixels(); present(&mut state, &mut Writer::default()).unwrap();
            let old = state.displayed_key; let id = state.kitty_image_id;
            state.frame_version.publish(true);
            let mut out = Writer { fail_write, fail_flush, ..Default::default() };
            assert!(present(&mut state, &mut out).is_err());
            assert_eq!(state.displayed_key, old); assert_eq!(state.kitty_image_id, id); assert!(!state.display_valid);
            let mut retry = Writer::default();
            assert_eq!(present(&mut state, &mut retry).unwrap(), RenderOutcome::Presented);
            assert!(state.display_valid && !retry.bytes.is_empty()); assert_ne!(state.kitty_image_id, id);
        }
    }
}
