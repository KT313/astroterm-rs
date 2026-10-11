//! One shared RGB object at a time. A failed local transfer switches this session back to streaming.
use std::io::{self, Write};
use crate::{state::PixelState, timing::{StepTimes, BufferId, BufferShape, IndexDomain, Operation}};
use crate::terminal::transport::graphics::{kitty, present_frame};
use super::{encode_kitty_upload, describe_upload};
use super::composition::convert_kitty_pixels;

pub(super) fn prepare_shared_upload(state: &mut PixelState, times: &mut StepTimes) -> bool {
    prepare_shared_upload_with(state, times, kitty::create_shared_image)
}

fn prepare_shared_upload_with(state: &mut PixelState, times: &mut StepTimes, create: impl FnOnce(&[u8]) -> io::Result<crate::state::SharedMemoryImage>) -> bool {
    if !state.shared_memory { return false; }
    state.shared_upload = None;
    state.encoding_key = None; // upload bytes will hold a name, so an older streamed payload must not be reused
    state.compressed.clear();
    convert_kitty_pixels(state, times);                                             // the terminal reads a quarter fewer bytes than the RGBA frame
    let frame = state.frame_image.as_ref().expect("completed frame");
    let result = times.measure("Shared memory preparation", || -> io::Result<()> {
        let object = create(&state.rgb)?;
        kitty::encode_shared_upload(&object, frame.dimensions(), state.kitty_image_id, false, state.tmux, &mut state.upload);
        state.shared_upload = Some(object);
        Ok(())
    });
    if let Err(error) = result {
        state.shared_memory = false;
        times.describe("Shared memory preparation", || format!("streaming fallback: {error}"));
        return false;
    }
    times.record_shape(BufferId::SharedImage, Operation::Copy, None, || BufferShape::slice(&state.rgb, IndexDomain::Bytes));
    times.describe("Shared memory preparation", || format!("RGB bytes copied={}; command bytes={}; no compression or pixel base64; one pending object", state.rgb.len(), state.upload.len()));
    true
}

pub(super) fn upload_kitty_pixels(state: &mut PixelState, out: &mut impl Write, times: &mut StepTimes) -> io::Result<usize> {
    let mut written = write_upload(state, out, times)?;
    if let Some(object) = &state.shared_upload {
        let consumed = times.measure("Shared memory consumption", || kitty::wait_for_shared_consumption(object));
        state.shared_upload = None; // unlink failures/timeouts too; no shared-image placement has been sent yet
        times.record_unknown(BufferId::SharedImage, Operation::Release);
        if !matches!(consumed, Ok(true)) {
            state.shared_memory = false;
            times.describe("Shared memory consumption", || match consumed {
                Ok(false) => "streaming fallback: terminal did not consume the object before timeout".into(),
                Err(error) => format!("streaming fallback: cannot check shared-memory consumption: {error}"),
                Ok(true) => unreachable!(),
            });
            encode_kitty_upload(state, times)?; // replace the same back image before the normal swap
            written += write_upload(state, out, times)?;
        }
    }
    Ok(written)
}

fn write_upload(state: &PixelState, out: &mut impl Write, times: &mut StepTimes) -> io::Result<usize> {
    times.measure("Image upload", || present_frame(out, state.upload.as_bytes()))?;
    times.record_shape(BufferId::UploadBytes, Operation::Output, None, || describe_upload(state));
    times.describe("Image upload", || format!("command bytes written/flushed={}; shared memory={}; outside synchronized output", state.upload.len(), state.shared_upload.is_some()));
    Ok(state.upload.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_failure_selects_streaming_without_publishing_a_name() {
        let mut state = super::super::lifetime_tests::pixels(ratatui_image::picker::ProtocolType::Kitty);
        state.shared_memory = true;
        state.frame_image = Some(image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])));
        state.frame_version.publish(true);
        assert!(!prepare_shared_upload_with(&mut state, &mut StepTimes::default(), |_| Err(io::Error::other("injected allocation failure"))));
        assert!(!state.shared_memory && state.shared_upload.is_none());
        encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
        assert!(state.upload.contains("t=d,"));
    }
}
