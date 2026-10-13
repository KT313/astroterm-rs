//! Step 7 of the frame: `present`.
//!
//! Inputs: the finished frame (star layer and overlay) and the terminal. Output: the frame on the terminal.
//! Runs every frame: the star layer is tone mapped to 8 bits and the overlay composited over it, then the bytes are
//! compared with the previous frame; an unchanged frame sends nothing, a changed one is encoded for the terminal's
//! graphics protocol (Kitty upload and swap, Sixel, iTerm2, or half-block cells) through the keep-list transport
//! and written. A Kitty terminal that passed the shared-memory probe gets the RGB bytes as a named POSIX object
//! it reads and unlinks itself; if it does not consume the object in time, or the object cannot be made, the
//! session falls back to streaming for good. Nothing here reads catalog data. `display` opens the terminal and
//! sizes the frame.
mod display;
pub use display::{Display, DisplayInfo};
pub use crate::terminal::transport::graphics::detection::{CompressionSupport, describe_compression}; // the detection result the panel and the benchmark report describe

/// The notice step 6 shows when a Kitty terminal streams: the old transport notice, unchanged.
pub const SHARED_MEMORY_NOTICE: &str = "Shared memory unavailable; using streaming.";

/// The transport notice for the display as it is now: none unless a Kitty terminal is streaming.
pub fn transport_notice(info: &DisplayInfo) -> Option<&'static str> {
    (info.protocol == ProtocolType::Kitty && !info.shared_memory).then_some(SHARED_MEMORY_NOTICE)
}

use crate::timing::StepTimes;
use crate::constants::{MIN_STAR_PIXEL_OPACITY, PIXEL_BACKGROUND_RGBA};
use crate::projection::Frame;
use crate::terminal::transport::graphics::{compose_image, encode_image, kitty, present_frame, serialize_frame_into};
use image::{DynamicImage, RgbaImage};
use ratatui::{buffer::Buffer, layout::Rect};
use ratatui_image::picker::ProtocolType;
use std::io::{self, Write};

/// The terminal a frame is presented on: the output, its graphics protocol and the scratch the encoders reuse.
pub struct Terminal<W: Write> {
    out: W,
    protocol: ProtocolType,
    tmux: bool,
    screen: Rect,                           // the terminal in cells; the frame covers it
    compress: bool,                         // zlib for streamed Kitty uploads
    shared_memory: bool,                    // Kitty reads the RGB from a shared-memory object; cleared for the session on the first failed transfer
    kitty_id: u32,
    previous: Vec<u8>,                      // the RGBA bytes last sent; empty before the first frame
    image: Vec<u8>,                         // the composed RGBA bytes of this frame
    rgb: Vec<u8>,
    compressed: Vec<u8>,
    compressor: Option<flate2::Compress>,
    upload: String,
    swap: Vec<u8>,
    blank: Buffer,
    serialized: Vec<u8>,
}

/// What `present` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    Sent(usize),                            // bytes written to the terminal
    Unchanged,                              // the frame equals the previous one: nothing sent
}

impl<W: Write> Terminal<W> {
    pub fn new(out: W, protocol: ProtocolType, tmux: bool, screen: Rect, compress: bool, shared_memory: bool) -> Self {
        Self {
            out, protocol, tmux, screen, compress, shared_memory,
            kitty_id: kitty::IMAGE_IDS[0],
            previous: Vec::new(), image: Vec::new(), rgb: Vec::new(), compressed: Vec::new(), compressor: None,
            upload: String::new(), swap: Vec::new(), blank: Buffer::empty(Rect::default()), serialized: Vec::new(),
        }
    }

    pub fn protocol(&self) -> ProtocolType { self.protocol }
    pub fn screen(&self) -> Rect { self.screen }
    pub fn output(&mut self) -> &mut W { &mut self.out }
    pub fn kitty_image_id(&self) -> u32 { self.kitty_id }
    /// Whether Kitty uploads currently go through shared memory rather than the base64 stream.
    pub fn shared_memory(&self) -> bool { self.shared_memory }

    /// Adopt a new terminal size; the next frame is sent whole.
    pub fn resize(&mut self, screen: Rect) {
        self.screen = screen;
        self.previous.clear();
    }
}

/// Show the frame: tone map, compare, encode, send. Returns whether anything was sent.
pub fn present<W: Write>(frame: &Frame, terminal: &mut Terminal<W>, times: &mut StepTimes) -> io::Result<Presented> {
    times.measure_detail("Frame composition", || compose(frame, &mut terminal.image));
    if times.measure_detail("Frame comparison", || terminal.image == terminal.previous || terminal.image.is_empty()) { return Ok(Presented::Unchanged); } // a zero-size frame has nothing to send
    let (width, height) = (frame.width() as u32, frame.height() as u32);
    let written = match terminal.protocol {
        ProtocolType::Kitty => {
            times.measure_detail("RGB conversion", || strip_alpha(&terminal.image, &mut terminal.rgb));
            let mut written = upload_shared(terminal, (width, height), times)?;                                  // the shared object, if the terminal takes it
            if !terminal.shared_memory {                                                                  // otherwise, or after a failed transfer, stream the same image
                times.measure_detail("Image encoding", || kitty::encode_upload_reusing(&terminal.rgb, (width, height), terminal.kitty_id, terminal.compress, terminal.tmux, &mut terminal.compressor, &mut terminal.compressed, &mut terminal.upload))?;
                times.measure_detail("Image upload", || present_frame(&mut terminal.out, terminal.upload.as_bytes()))?;
                written += terminal.upload.len();
            }
            times.measure_detail("Image swap", || {
                kitty::serialize_swap_into(terminal.kitty_id, terminal.screen, terminal.tmux, &mut terminal.swap)?;
                present_frame(&mut terminal.out, &terminal.swap)
            })?;
            terminal.kitty_id = kitty::other_image_id(terminal.kitty_id);
            written + terminal.swap.len()
        }
        protocol => {
            let image = times.measure_detail("RGBA image copy", || RgbaImage::from_raw(width, height, terminal.image.clone()).expect("the composed frame is width × height RGBA"));
            let encoded = times.measure_detail("Image encoding", || encode_image(DynamicImage::ImageRgba8(image), terminal.screen, protocol, terminal.tmux))?;
            let cells = times.measure_detail("Terminal image layout", || compose_image(&encoded, terminal.screen, terminal.screen));
            times.measure_detail("Frame serialization", || serialize_frame_into(&cells, &mut terminal.blank, &mut terminal.serialized))?;
            times.measure_detail("Image upload", || present_frame(&mut terminal.out, &terminal.serialized))?;
            terminal.serialized.len()
        }
    };
    std::mem::swap(&mut terminal.previous, &mut terminal.image);
    Ok(Presented::Sent(written))
}

/// Hand the RGB bytes to a Kitty terminal through a shared-memory object and wait for it to read and unlink them.
/// Returns the command bytes written. Does nothing when the transport is off, and switches it off for the rest of
/// the session when the object cannot be made or the terminal does not consume it in time; the caller then streams
/// the same image under the same id, so nothing is placed before a complete upload. The object is unlinked here if
/// the terminal did not, also when the write fails.
fn upload_shared<W: Write>(terminal: &mut Terminal<W>, dimensions: (u32, u32), times: &mut StepTimes) -> io::Result<usize> {
    if !terminal.shared_memory { return Ok(0); }
    let object = match times.measure_detail("Shared memory preparation", || kitty::create_shared_image(&terminal.rgb)) {
        Ok(object) => object,
        Err(_) => { terminal.shared_memory = false; return Ok(0); }                                      // no shared memory on this system: stream from now on
    };
    times.measure_detail("Shared memory command", || kitty::encode_shared_upload(&object, dimensions, terminal.kitty_id, false, terminal.tmux, &mut terminal.upload));
    times.measure_detail("Image upload", || present_frame(&mut terminal.out, terminal.upload.as_bytes()))?; // on error the object drops and unlinks its name
    let consumed = times.measure_detail("Shared memory consumption", || kitty::wait_for_shared_consumption(&object));
    if !matches!(consumed, Ok(true)) { terminal.shared_memory = false; }                                  // timed out or unreadable: the streamed copy follows
    Ok(terminal.upload.len())
}

/// Tone map the star layer over the background and composite the overlay, into opaque 8-bit RGBA. A star pixel
/// darkens the background by its opacity and adds its colour share; a non-empty pixel fainter than the floor is
/// raised to it without changing its colour; summed light above full opacity saturates to the star's colour.
pub fn compose(frame: &Frame, out: &mut Vec<u8>) {
    let background = PIXEL_BACKGROUND_RGBA.map(f32::from);
    out.clear();
    out.reserve(frame.width() * frame.height() * 4);
    for (texel, overlay) in frame.stars().iter().zip(frame.overlay().chunks_exact(4)) {
        #[cfg(feature = "pixel-debug")]
        {
            texel.check("compose star input", f64::INFINITY);
            crate::pixel_debug::record_accumulated([texel.rgb[0] as f64, texel.rgb[1] as f64, texel.rgb[2] as f64, texel.opacity as f64]);
            crate::pixel_debug::check_overlay("compose overlay input", overlay);
        }
        let mut pixel = [background[0], background[1], background[2]];
        if texel.opacity > 0.0 {
            let (scale, floor) = if texel.opacity > 1.0 { (255.0 / texel.opacity, 1.0) }                // saturated: the straight colour
                else if texel.opacity < MIN_STAR_PIXEL_OPACITY { (255.0 * MIN_STAR_PIXEL_OPACITY / texel.opacity, MIN_STAR_PIXEL_OPACITY) } // raised to the floor, colour intact
                else { (255.0, texel.opacity) };
            for (index, (channel, color)) in pixel.iter_mut().zip(texel.rgb).enumerate() {
                let value = color * scale + *channel * (1.0 - floor);
                #[cfg(feature = "pixel-debug")]
                crate::pixel_debug::check_channel("compose tone mapping before clamp", index, value as f64, 255.0);
                let _ = index;
                *channel = value.min(255.0);
            }
        }
        let alpha = f32::from(overlay[3]) / 255.0;
        for (channel, &drawn) in pixel.iter_mut().zip(&overlay[..3]) { *channel = f32::from(drawn) + *channel * (1.0 - alpha); } // the overlay is premultiplied
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("compose before u8 conversion", [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64, 255.0], 255.0);
        out.extend(pixel.map(|channel| (channel + 0.5) as u8));
        out.push(255);
    }
}

fn strip_alpha(rgba: &[u8], rgb: &mut Vec<u8>) {
    #[cfg(feature = "pixel-debug")]
    {
        crate::pixel_debug::check_overlay("strip_alpha input", rgba);
        assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 255), "pixel-debug: strip_alpha requires opaque pixels");
    }
    rgb.clear();
    rgb.reserve(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) { rgb.extend_from_slice(&pixel[..3]); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::SkyArea;
    use crate::regions::Texel;

    fn frame(width: usize, height: usize) -> Frame { Frame::new(width, height, SkyArea::centred(width, height), (10, 20)) }
    fn pixel(out: &[u8], frame: &Frame, row: usize, column: usize) -> [u8; 4] { out[(row * frame.width() + column) * 4..][..4].try_into().unwrap() }
    fn premultiplied(rgb: [f32; 3], opacity: f32) -> Texel { Texel { rgb: rgb.map(|c| c / 255.0 * opacity), opacity } }

    #[cfg(feature = "pixel-debug")]
    #[test]
    fn pixel_wrappers_reject_hidden_color_overflow_and_nonopaque_rgb_conversion() {
        let mut frame = frame(1, 1);
        frame.stars_mut()[0] = Texel { rgb: [1.1, 0.0, 0.0], opacity: 1.0 };
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compose(&frame, &mut Vec::new()))).unwrap_err();
        assert!(failure.downcast_ref::<String>().unwrap().contains("compose star input")); // caught before min(255) could hide the bad colour
        assert!(std::panic::catch_unwind(|| strip_alpha(&[20, 10, 5, 128], &mut Vec::new())).is_err());
    }

    #[test]
    fn tone_mapping_matches_the_old_star_composition_and_handles_floor_and_saturation() {
        let mut frame = frame(6, 1);
        let stars = frame.stars_mut();
        stars[1] = premultiplied([255.0, 231.0, 176.0], 1.0);
        stars[2] = premultiplied([255.0, 231.0, 176.0], 0.2);
        stars[3] = premultiplied([255.0, 231.0, 176.0], 1e-24);                             // raised to the 5 % floor
        stars[4] = premultiplied([255.0, 231.0, 176.0], MIN_STAR_PIXEL_OPACITY * 0.5);
        stars[5] = Texel { rgb: [2.0, 0.0, 0.0], opacity: 2.0 };                            // two full red stars added: saturates to red
        let mut out = Vec::new();
        compose(&frame, &mut out);
        let [r, g, b, _] = PIXEL_BACKGROUND_RGBA;
        assert_eq!(pixel(&out, &frame, 0, 0), [r, g, b, 255]);                              // empty: the background
        assert_eq!(pixel(&out, &frame, 0, 1), [255, 231, 176, 255]);
        let expected = |color: f32, background: u8| (color * 0.2 + f32::from(background) * 0.8 + 0.5) as u8;
        assert_eq!(pixel(&out, &frame, 0, 2), [expected(255.0, r), expected(231.0, g), expected(176.0, b), 255]);
        let floor = |color: f32, background: u8| (color * MIN_STAR_PIXEL_OPACITY + f32::from(background) * (1.0 - MIN_STAR_PIXEL_OPACITY) + 0.5) as u8;
        assert_eq!(pixel(&out, &frame, 0, 3), [floor(255.0, r), floor(231.0, g), floor(176.0, b), 255]);
        assert_eq!(pixel(&out, &frame, 0, 4), pixel(&out, &frame, 0, 3));                    // every pixel below the floor is lifted to it
        assert_eq!(pixel(&out, &frame, 0, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn the_overlay_is_composited_over_the_stars_by_its_alpha() {
        let mut frame = frame(3, 1);
        frame.stars_mut()[1] = premultiplied([255.0, 255.0, 255.0], 1.0);
        frame.stars_mut()[2] = premultiplied([255.0, 255.0, 255.0], 1.0);
        {
            let (overlay, _) = frame.overlay_and_text();
            overlay[4..8].copy_from_slice(&[100, 0, 0, 255]);                                   // opaque red over a white star
            overlay[8..12].copy_from_slice(&[0, 0, 128, 128]);                                 // half-transparent blue over a white star
        }
        let mut out = Vec::new();
        compose(&frame, &mut out);
        assert_eq!(pixel(&out, &frame, 0, 1), [100, 0, 0, 255]);
        let half = pixel(&out, &frame, 0, 2);
        assert!(half[0] > 125 && half[0] < 130 && half[2] == 255, "{half:?}");              // 255 × (1 − 0.5) red, 128 + 127 blue
    }

    #[test]
    fn an_unchanged_frame_sends_nothing_and_a_changed_one_is_sent() {
        let mut frame = frame(40, 20);
        let mut terminal = Terminal::new(Vec::new(), ProtocolType::Sixel, false, Rect::new(0, 0, 4, 1), false, false);
        let first = present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap();
        let Presented::Sent(bytes) = first else { panic!("{first:?}") };
        assert!(bytes > 0 && terminal.output().len() == bytes);
        assert_eq!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Unchanged);
        assert_eq!(terminal.output().len(), bytes);                                          // nothing written, nothing flushed to the output
        frame.stars_mut()[205] = Texel { rgb: [1.0; 3], opacity: 1.0 };
        assert!(matches!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Sent(_)));
        assert!(terminal.output().len() > bytes);
        terminal.resize(Rect::new(0, 0, 4, 1));
        assert!(matches!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Sent(_)));      // a resize resends the same frame
    }

    /// Release-mode cost of composing and encoding a full HD frame; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_frame_output_at_full_hd() {
        let mut frame = frame(1920, 1100);
        for (index, texel) in frame.stars_mut().iter_mut().enumerate() { if index % 97 == 0 { *texel = Texel { rgb: [0.5, 0.5, 0.4], opacity: 0.6 }; } }
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel] {
            let mut terminal = Terminal::new(std::io::sink(), protocol, false, Rect::new(0, 0, 192, 55), protocol == ProtocolType::Kitty, false);
            let started = std::time::Instant::now();
            let mut composed = Vec::new();
            compose(&frame, &mut composed);
            let compose_time = started.elapsed();
            let started = std::time::Instant::now();
            let sent = present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap();
            let first = started.elapsed();
            let started = std::time::Instant::now();
            let again = present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap();
            let unchanged = started.elapsed();
            eprintln!("{protocol:?}: compose {compose_time:?}; present {first:?} ({sent:?}); unchanged frame {unchanged:?} ({again:?})");
        }
    }

    #[test]
    fn the_transport_notice_is_limited_to_streaming_kitty_displays() {
        let info = |protocol, shared_memory| DisplayInfo { protocol, tmux: false, compression: CompressionSupport::Unknown, shared_memory, screen: Rect::default(), font: (10, 20), image: (100, 50), text_cell: (9, 17) };
        assert_eq!(transport_notice(&info(ProtocolType::Kitty, false)), Some(SHARED_MEMORY_NOTICE));
        assert_eq!(transport_notice(&info(ProtocolType::Kitty, true)), None);
        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2, ProtocolType::Halfblocks] { assert_eq!(transport_notice(&info(protocol, false)), None); }
    }

    #[test]
    fn kitty_frames_upload_then_swap_and_alternate_the_image_id() {
        let frame = frame(8, 4);
        let mut terminal = Terminal::new(Vec::new(), ProtocolType::Kitty, false, Rect::new(0, 0, 8, 2), false, false);
        let first_id = terminal.kitty_image_id();
        let Presented::Sent(bytes) = present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap() else { panic!() };
        let text = String::from_utf8_lossy(terminal.output()).into_owned();
        assert!(text.contains(&format!("i={first_id}")) && text.contains("f=24"), "{text}");  // an RGB upload for the first id
        assert_eq!(terminal.output().len(), bytes);
        assert_ne!(terminal.kitty_image_id(), first_id);
        assert_eq!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Unchanged);
        assert_eq!(terminal.kitty_image_id(), kitty::other_image_id(first_id));
    }
}

#[cfg(all(test, unix))]
mod shared_tests {
    use super::*;
    use crate::projection::SkyArea;
    use crate::regions::Texel;
    use std::{ffi::CString, io::Read};

    /// A Kitty terminal stand-in: decodes shared-memory uploads and either consumes (reads and unlinks) them or not.
    struct Receiver { consume: bool, fail_upload: bool, names: Vec<CString>, pixels: Vec<Vec<u8>>, output: Vec<u8> }
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

    fn frame() -> Frame {
        let mut frame = Frame::new(16, 8, SkyArea::centred(16, 8), (4, 8));
        frame.stars_mut()[20] = Texel { rgb: [1.0, 0.5, 0.25], opacity: 1.0 };
        frame
    }
    fn rgb(frame: &Frame) -> Vec<u8> { let mut rgba = Vec::new(); compose(frame, &mut rgba); rgba.chunks_exact(4).flat_map(|pixel| pixel[..3].to_vec()).collect() }
    fn missing(name: &CString) -> bool {
        rustix::shm::open(name.as_c_str(), rustix::shm::OFlags::RDONLY, rustix::shm::Mode::empty()).is_err_and(|error| error == rustix::io::Errno::NOENT)
    }
    fn terminal(consume: bool, tmux: bool) -> Terminal<Receiver> { Terminal::new(Receiver::new(consume), ProtocolType::Kitty, tmux, Rect::new(0, 0, 4, 1), true, true) }

    #[test]
    fn shared_transfer_carries_the_rgb_without_a_stream_and_the_next_frame_is_unchanged() {
        for tmux in [false, true] {
            let frame = frame();
            let mut terminal = terminal(true, tmux);
            let Presented::Sent(bytes) = present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap() else { panic!() };
            assert_eq!(terminal.output().pixels, [rgb(&frame)]);                                          // the object carries RGB, not the RGBA frame
            assert!(terminal.shared_memory() && terminal.output().names.iter().all(missing));            // consumed: the name is gone and the transport stays
            let text = String::from_utf8(terminal.output().output.clone()).unwrap();
            assert!(text.contains("t=s,") && !text.contains("t=d,") && !text.contains("o=z,"), "{text}"); // no streamed copy, no compression
            assert!(text.find("t=s,").unwrap() < text.find("a=p,").unwrap());
            assert_eq!(text.len(), bytes);
            assert_eq!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Unchanged);
            assert_eq!(terminal.output().names.len(), 1);
        }
    }

    #[test]
    fn an_unconsumed_object_falls_back_to_a_complete_stream_before_the_swap_for_the_rest_of_the_session() {
        let frame = frame();
        let mut terminal = terminal(false, false);
        assert!(matches!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Sent(_)));
        assert!(!terminal.shared_memory() && terminal.output().names.iter().all(missing));              // unlinked here, transport switched off
        let text = String::from_utf8(terminal.output().output.clone()).unwrap();
        assert!(text.find("t=s,").unwrap() < text.find("t=d,").unwrap() && text.find("t=d,").unwrap() < text.find("a=p,").unwrap());
        let mut compressed = Vec::new();
        for command in text.split("\x1b_G").skip(1) {
            let Some((header, payload)) = command.split_once(';') else { continue; };
            if header.contains("m=") { compressed.extend(base64_simd::STANDARD.decode_to_vec(payload.split('\x1b').next().unwrap()).unwrap()); }
        }
        let mut decoded = Vec::new(); flate2::read::ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, rgb(&frame));                                                                 // the streamed copy is the whole image
        let mut changed = frame; changed.stars_mut()[30] = Texel { rgb: [0.2, 0.2, 1.0], opacity: 1.0 };
        let before = terminal.output().names.len();
        assert!(matches!(present(&changed, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Sent(_)));
        assert_eq!(terminal.output().names.len(), before);                                                // no further shared attempt this session
    }

    #[test]
    fn a_failed_shared_write_unlinks_the_object_and_places_nothing() {
        let frame = frame();
        let mut terminal = terminal(false, false);
        terminal.output().fail_upload = true;
        let id = terminal.kitty_image_id();
        assert!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).is_err());
        assert_eq!(terminal.kitty_image_id(), id);
        assert!(terminal.output().names.iter().all(missing));
        assert!(!String::from_utf8(terminal.output().output.clone()).unwrap().contains("a=p,"));
        terminal.output().fail_upload = false;
        assert!(matches!(present(&frame, &mut terminal, &mut crate::timing::StepTimes::default()).unwrap(), Presented::Sent(_)));                  // the next frame is sent whole
    }
}
