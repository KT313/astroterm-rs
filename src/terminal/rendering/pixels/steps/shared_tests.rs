use super::*;
use std::{ffi::CString, io::Read};
use ratatui_image::picker::ProtocolType;

struct Receiver {
    consume: bool,
    fail_upload: bool,
    names: Vec<CString>,
    pixels: Vec<Vec<u8>>,
    output: Vec<u8>,
}
impl Receiver {
    fn new(consume: bool) -> Self { Self { consume, fail_upload: false, names: Vec::new(), pixels: Vec::new(), output: Vec::new() } }
}
impl Write for Receiver {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(bytes);
        let text = std::str::from_utf8(bytes).unwrap();
        if text.contains("t=s,") {
            let command = &text[text.find("_G").unwrap() + 2..];
            let payload = command.split_once(';').unwrap().1.split('\x1b').next().unwrap();
            let name = CString::new(base64_simd::STANDARD.decode_to_vec(payload).unwrap()).unwrap();
            self.names.push(name.clone());
            if self.fail_upload { return Err(io::Error::other("injected shared upload failure")); }
            if self.consume {
                let mut input = std::fs::File::from(rustix::shm::open(name.as_c_str(), rustix::shm::OFlags::RDONLY, rustix::shm::Mode::empty())?);
                let mut rgb = Vec::new(); input.read_to_end(&mut rgb)?;
                self.pixels.push(rgb);
                rustix::shm::unlink(name.as_c_str())?;
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
fn pixels() -> PixelState {
    let mut state = super::super::lifetime_tests::pixels(ProtocolType::Kitty);
    state.shared_memory = true;
    state.rgb = image::RgbImage::from_pixel(16, 8, image::Rgb([11, 22, 33]));
    state.rgb_version.publish(true);
    state
}
fn missing(name: &CString) -> bool {
    rustix::shm::open(name.as_c_str(), rustix::shm::OFlags::RDONLY, rustix::shm::Mode::empty()).is_err_and(|error| error == rustix::io::Errno::NOENT)
}

#[test]
fn shared_transfer_keeps_rgb_and_bypasses_compression_then_reuses_the_display() {
    for tmux in [false, true] {
        let mut state = pixels(); state.tmux = tmux;
        let mut out = Receiver::new(true); let pointer = state.rgb.as_ptr();
        let mut times = StepTimes::with_trace(true);
        assert_eq!(present_kitty_image(&mut state, &mut out, &mut times).unwrap(), RenderOutcome::Presented);
        assert_eq!(out.pixels, [state.rgb.as_raw().clone()]);
        assert_eq!(state.rgb.as_ptr(), pointer); assert!(state.compressor.is_none());
        assert!(state.shared_upload.is_none() && state.shared_memory);
        assert!(out.names.iter().all(missing));
        assert!(!times.trace().unwrap().steps.iter().any(|step| step.name == "Image encoding"));
        let count = out.output.len();
        assert_eq!(present_kitty_image(&mut state, &mut out, &mut times).unwrap(), RenderOutcome::ReusedDisplayedFrame);
        assert_eq!(out.output.len(), count); assert_eq!(out.names.len(), 1);
    }
}

#[test]
fn an_unconsumed_object_falls_back_to_a_complete_stream_before_the_swap() {
    let mut state = pixels(); let mut out = Receiver::new(false);
    assert_eq!(present_kitty_image(&mut state, &mut out, &mut StepTimes::default()).unwrap(), RenderOutcome::Presented);
    assert!(!state.shared_memory && state.shared_upload.is_none());
    assert!(out.names.iter().all(missing));
    let text = String::from_utf8(out.output).unwrap();
    assert!(text.find("t=s,").unwrap() < text.find("t=d,").unwrap());
    assert!(text.find("t=d,").unwrap() < text.find("a=p,").unwrap());
    let mut compressed = Vec::new();
    let mut started = false;
    for command in text.split("\x1b_G").skip(1) {
        let Some((header, payload)) = command.split_once(';') else { continue; };
        if header.contains("t=d,") { started = true; }
        if started && header.contains("m=") { compressed.extend(base64_simd::STANDARD.decode_to_vec(payload.split('\x1b').next().unwrap()).unwrap()); }
    }
    let mut decoded = Vec::new(); flate2::read::ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, *state.rgb.as_raw());
    assert!(state.display_valid && !state.displayed_key.unwrap().shared_memory);
}

#[test]
fn shared_write_failure_unlinks_the_object_and_does_not_publish_or_swap() {
    let mut state = pixels(); let mut out = Receiver::new(false); out.fail_upload = true;
    let id = state.kitty_image_id;
    assert!(present_kitty_image(&mut state, &mut out, &mut StepTimes::default()).is_err());
    assert_eq!(state.kitty_image_id, id); assert!(!state.display_valid);
    assert!(state.displayed_key.is_none() && state.shared_upload.is_none());
    assert!(out.names.iter().all(missing));
    assert!(!String::from_utf8(out.output).unwrap().contains("a=p,"));
}
