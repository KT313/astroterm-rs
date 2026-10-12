//! Step 7 of the frame: `present`.
//!
//! Inputs: the finished frame (star layer and overlay) and the terminal. Output: the frame on the terminal.
//! Runs every frame: the star layer is tone mapped to 8 bits and the overlay composited over it, then the bytes are
//! compared with the previous frame; an unchanged frame sends nothing, a changed one is encoded for the terminal's
//! graphics protocol (Kitty upload and swap, Sixel, iTerm2, or half-block cells) through the keep-list transport
//! and written. Nothing here reads catalog data. `display` opens the terminal and sizes the frame.
mod display;
pub use display::{Display, DisplayInfo};
pub use crate::terminal::transport::graphics::detection::{CompressionSupport, describe_compression}; // the detection result the panel and the benchmark report describe

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
    compress: bool,                         // zlib for Kitty uploads
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
    pub fn new(out: W, protocol: ProtocolType, tmux: bool, screen: Rect, compress: bool) -> Self {
        Self {
            out, protocol, tmux, screen, compress,
            kitty_id: kitty::IMAGE_IDS[0],
            previous: Vec::new(), image: Vec::new(), rgb: Vec::new(), compressed: Vec::new(), compressor: None,
            upload: String::new(), swap: Vec::new(), blank: Buffer::empty(Rect::default()), serialized: Vec::new(),
        }
    }

    pub fn protocol(&self) -> ProtocolType { self.protocol }
    pub fn screen(&self) -> Rect { self.screen }
    pub fn output(&mut self) -> &mut W { &mut self.out }
    pub fn kitty_image_id(&self) -> u32 { self.kitty_id }

    /// Adopt a new terminal size; the next frame is sent whole.
    pub fn resize(&mut self, screen: Rect) {
        self.screen = screen;
        self.previous.clear();
    }
}

/// Show the frame: tone map, compare, encode, send. Returns whether anything was sent.
pub fn present<W: Write>(frame: &Frame, terminal: &mut Terminal<W>) -> io::Result<Presented> {
    compose(frame, &mut terminal.image);
    if terminal.image == terminal.previous || terminal.image.is_empty() { return Ok(Presented::Unchanged); } // a zero-size frame has nothing to send
    let (width, height) = (frame.width() as u32, frame.height() as u32);
    let written = match terminal.protocol {
        ProtocolType::Kitty => {
            strip_alpha(&terminal.image, &mut terminal.rgb);
            kitty::encode_upload_reusing(&terminal.rgb, (width, height), terminal.kitty_id, terminal.compress, terminal.tmux, &mut terminal.compressor, &mut terminal.compressed, &mut terminal.upload)?;
            kitty::serialize_swap_into(terminal.kitty_id, terminal.screen, terminal.tmux, &mut terminal.swap)?;
            present_frame(&mut terminal.out, terminal.upload.as_bytes())?;
            present_frame(&mut terminal.out, &terminal.swap)?;
            terminal.kitty_id = kitty::other_image_id(terminal.kitty_id);
            terminal.upload.len() + terminal.swap.len()
        }
        protocol => {
            let image = RgbaImage::from_raw(width, height, terminal.image.clone()).expect("the composed frame is width × height RGBA");
            let encoded = encode_image(DynamicImage::ImageRgba8(image), terminal.screen, protocol, terminal.tmux)?;
            let cells = compose_image(&encoded, terminal.screen, terminal.screen);
            serialize_frame_into(&cells, &mut terminal.blank, &mut terminal.serialized)?;
            present_frame(&mut terminal.out, &terminal.serialized)?;
            terminal.serialized.len()
        }
    };
    std::mem::swap(&mut terminal.previous, &mut terminal.image);
    Ok(Presented::Sent(written))
}

/// Tone map the star layer over the background and composite the overlay, into opaque 8-bit RGBA. A star pixel
/// darkens the background by its opacity and adds its colour share; a non-empty pixel fainter than the floor is
/// raised to it without changing its colour; summed light above full opacity saturates to the star's colour.
pub fn compose(frame: &Frame, out: &mut Vec<u8>) {
    let background = PIXEL_BACKGROUND_RGBA.map(f32::from);
    out.clear();
    out.reserve(frame.width() * frame.height() * 4);
    for (texel, overlay) in frame.stars().iter().zip(frame.overlay().chunks_exact(4)) {
        let mut pixel = [background[0], background[1], background[2]];
        if texel.opacity > 0.0 {
            let (scale, floor) = if texel.opacity > 1.0 { (255.0 / texel.opacity, 1.0) }                // saturated: the straight colour
                else if texel.opacity < MIN_STAR_PIXEL_OPACITY { (255.0 * MIN_STAR_PIXEL_OPACITY / texel.opacity, MIN_STAR_PIXEL_OPACITY) } // raised to the floor, colour intact
                else { (255.0, texel.opacity) };
            for (channel, color) in pixel.iter_mut().zip(texel.rgb) { *channel = (color * scale + *channel * (1.0 - floor)).min(255.0); }
        }
        let alpha = f32::from(overlay[3]) / 255.0;
        for (channel, &drawn) in pixel.iter_mut().zip(&overlay[..3]) { *channel = f32::from(drawn) + *channel * (1.0 - alpha); } // the overlay is premultiplied
        out.extend(pixel.map(|channel| (channel + 0.5) as u8));
        out.push(255);
    }
}

fn strip_alpha(rgba: &[u8], rgb: &mut Vec<u8>) {
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
        let mut terminal = Terminal::new(Vec::new(), ProtocolType::Sixel, false, Rect::new(0, 0, 4, 1), false);
        let first = present(&frame, &mut terminal).unwrap();
        let Presented::Sent(bytes) = first else { panic!("{first:?}") };
        assert!(bytes > 0 && terminal.output().len() == bytes);
        assert_eq!(present(&frame, &mut terminal).unwrap(), Presented::Unchanged);
        assert_eq!(terminal.output().len(), bytes);                                          // nothing written, nothing flushed to the output
        frame.stars_mut()[205] = Texel { rgb: [1.0; 3], opacity: 1.0 };
        assert!(matches!(present(&frame, &mut terminal).unwrap(), Presented::Sent(_)));
        assert!(terminal.output().len() > bytes);
        terminal.resize(Rect::new(0, 0, 4, 1));
        assert!(matches!(present(&frame, &mut terminal).unwrap(), Presented::Sent(_)));      // a resize resends the same frame
    }

    /// Release-mode cost of composing and encoding a full HD frame; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing only"]
    fn time_the_frame_output_at_full_hd() {
        let mut frame = frame(1920, 1100);
        for (index, texel) in frame.stars_mut().iter_mut().enumerate() { if index % 97 == 0 { *texel = Texel { rgb: [0.5, 0.5, 0.4], opacity: 0.6 }; } }
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel] {
            let mut terminal = Terminal::new(std::io::sink(), protocol, false, Rect::new(0, 0, 192, 55), protocol == ProtocolType::Kitty);
            let started = std::time::Instant::now();
            let mut composed = Vec::new();
            compose(&frame, &mut composed);
            let compose_time = started.elapsed();
            let started = std::time::Instant::now();
            let sent = present(&frame, &mut terminal).unwrap();
            let first = started.elapsed();
            let started = std::time::Instant::now();
            let again = present(&frame, &mut terminal).unwrap();
            let unchanged = started.elapsed();
            eprintln!("{protocol:?}: compose {compose_time:?}; present {first:?} ({sent:?}); unchanged frame {unchanged:?} ({again:?})");
        }
    }

    #[test]
    fn kitty_frames_upload_then_swap_and_alternate_the_image_id() {
        let frame = frame(8, 4);
        let mut terminal = Terminal::new(Vec::new(), ProtocolType::Kitty, false, Rect::new(0, 0, 8, 2), false);
        let first_id = terminal.kitty_image_id();
        let Presented::Sent(bytes) = present(&frame, &mut terminal).unwrap() else { panic!() };
        let text = String::from_utf8_lossy(terminal.output()).into_owned();
        assert!(text.contains(&format!("i={first_id}")) && text.contains("f=24"), "{text}");  // an RGB upload for the first id
        assert_eq!(terminal.output().len(), bytes);
        assert_ne!(terminal.kitty_image_id(), first_id);
        assert_eq!(present(&frame, &mut terminal).unwrap(), Presented::Unchanged);
        assert_eq!(terminal.kitty_image_id(), kitty::other_image_id(first_id));
    }
}
