//! Kitty RGB transport. Upload the back image outside synchronization, then atomically place it and delete
//! the previous image. Two IDs bound terminal storage; no Unicode placeholder cells are required.
mod compression;
mod shared_memory;
pub(crate) use shared_memory::{create_shared_image, encode_shared_upload, wait_for_shared_consumption};

use std::{
    fmt::Write as _,
    io::{self, Write as _},
};

use image::RgbImage;
use ratatui::layout::Rect;
use ratatui_image::picker::cap_parser::Parser;

pub const IMAGE_IDS: [u32; 2] = [super::IMAGE_ID, super::IMAGE_ID + 1];

pub fn other_image_id(id: u32) -> u32 {
    if id == IMAGE_IDS[0] { IMAGE_IDS[1] } else { IMAGE_IDS[0] }
}

/// Encode only an upload, without changing the visible placement. The caller supplies an already composed,
/// opaque RGB image; alpha blending belongs to rasterization, before this boundary.
pub fn encode_upload(image: &RgbImage, id: u32, compress: bool, tmux: bool) -> io::Result<Vec<u8>> {
    let mut compressed = Vec::new();
    let mut output = String::new();
    encode_upload_into(image, id, compress, tmux, &mut compressed, &mut output)?;
    Ok(output.into_bytes())
}

/// Populate application-owned compression and upload buffers after the prior upload has finished.
pub fn encode_upload_into(image: &RgbImage, id: u32, compress: bool, tmux: bool, compressed: &mut Vec<u8>, output: &mut String) -> io::Result<()> {
    encode_upload_reusing(image, id, compress, tmux, &mut None, compressed, output)
}

/// Keep the zlib engine alongside its output buffers; a reset starts a complete independent stream each time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_upload_reusing(image: &RgbImage, id: u32, compress: bool, tmux: bool, engine: &mut Option<flate2::Compress>, compressed: &mut Vec<u8>, output: &mut String) -> io::Result<()> {
    compressed.clear();
    output.clear();
    let raw = image.as_raw();
    let bytes = if compress {
        compression::compress_image(raw, engine, compressed)?;
        compressed.as_slice()
    } else { raw.as_slice() };

    // Base64 chunks may contain at most 4096 bytes, corresponding to 3072 input bytes.
    let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
    output.reserve(bytes.len().div_ceil(3) * 4 + bytes.len().div_ceil(3072) * 32 + 128);
    let chunks = bytes.chunks(3072);
    let count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        write!(output, "{start}{escape}_Gq=2,").unwrap();
        if index == 0 {
            let compression = if compress { "o=z," } else { "" };
            write!(
                output,
                "i={id},a=t,f=24,{compression}t=d,s={},v={},",
                image.width(),
                image.height()
            )
            .unwrap();
        }
        write!(output, "m={};", u8::from(index + 1 < count)).unwrap();
        base64_simd::STANDARD.encode_append(chunk, output);
        write!(output, "{escape}\\{end}").unwrap();
    }
    Ok(())
}

/// Place the completed back image before deleting the front image. The synchronization block contains no
/// image data, so uploading or decompressing a new image does not prolong the visible frame switch.
pub fn serialize_swap(id: u32, area: Rect, tmux: bool) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    serialize_swap_into(id, area, tmux, &mut output)?;
    Ok(output)
}

/// Populate the small synchronized swap command buffer; upload bytes stay outside it.
pub fn serialize_swap_into(id: u32, area: Rect, tmux: bool, output: &mut Vec<u8>) -> io::Result<()> {
    output.clear();
    crossterm::queue!(
        output,
        crossterm::terminal::BeginSynchronizedUpdate,
        crossterm::cursor::MoveTo(area.x, area.y)
    )?;
    let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
    write!(
        output,
        "{start}{escape}_Ga=p,i={id},p=1,c={},r={},C=1,q=2;{escape}\\{end}",
        area.width, area.height
    )?;
    write_deletion(output, other_image_id(id), tmux)?;
    crossterm::queue!(output, crossterm::terminal::EndSynchronizedUpdate)?;
    Ok(())
}

pub(super) fn clear_images(out: &mut impl io::Write, tmux: bool) -> io::Result<()> {
    for id in IMAGE_IDS {
        write_deletion(out, id, tmux)?;
    }
    Ok(())
}

fn write_deletion(out: &mut impl io::Write, id: u32, tmux: bool) -> io::Result<()> {
    let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
    write!(out, "{start}{escape}_Ga=d,d=I,i={id},q=2{escape}\\{end}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn owned_transport_scratch_reuses_capacity_without_stale_payload() {
        let large = RgbImage::from_fn(127, 83, |x, y| image::Rgb([x as u8, y as u8, (x * y) as u8]));
        let small = RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]));
        let mut compressed = Vec::new();
        let mut upload = String::new();
        encode_upload_into(&large, IMAGE_IDS[0], true, false, &mut compressed, &mut upload).unwrap();
        let capacities = (compressed.capacity(), upload.capacity());
        encode_upload_into(&small, IMAGE_IDS[1], false, false, &mut compressed, &mut upload).unwrap();
        assert!(compressed.is_empty());
        assert_eq!((compressed.capacity(), upload.capacity()), capacities);
        assert_eq!(upload.as_bytes(), encode_upload(&small, IMAGE_IDS[1], false, false).unwrap());
        assert!(!upload.contains("o=z,") && !upload.contains("s=127,"));
        let mut swap = Vec::new();
        serialize_swap_into(IMAGE_IDS[0], Rect::new(0, 0, 80, 40), false, &mut swap).unwrap();
        let capacity = swap.capacity();
        serialize_swap_into(IMAGE_IDS[1], Rect::new(0, 0, 2, 2), false, &mut swap).unwrap();
        assert_eq!(swap.capacity(), capacity);
        assert_eq!(swap, serialize_swap(IMAGE_IDS[1], Rect::new(0, 0, 2, 2), false).unwrap());
    }

    #[test]
    fn rgb_transfers_round_trip_with_and_without_compression_and_tmux() {
        // Incompressible-looking RGB verifies multi-chunk transfers as well as the final partial chunk.
        let image = RgbImage::from_fn(127, 83, |x, y| {
            image::Rgb([(x * 31 + y * 17) as u8, (x * y + 71) as u8, (x * 91 + y * 43) as u8])
        });
        for compress in [false, true] {
            for tmux in [false, true] {
                let output = String::from_utf8(encode_upload(&image, IMAGE_IDS[0], compress, tmux).unwrap()).unwrap();
                let output = if tmux {
                    output.replace("\x1bPtmux;", "").replace("\x1b\x1b", "\x1b")
                } else {
                    output
                };
                assert!(output.contains("a=t,f=24,"));
                assert_eq!(output.contains("o=z,"), compress);
                assert!(!output.contains("2026") && !output.contains("a=p"));
                let mut bytes = Vec::new();
                let chunks: Vec<_> = output.split("\x1b_G").skip(1).collect();
                assert!(chunks.len() > 1);
                for (index, command) in chunks.iter().enumerate() {
                    let (header, payload) = command.split_once(';').unwrap();
                    let payload = payload.split("\x1b\\").next().unwrap();
                    assert!(payload.len() <= 4096 && payload.len() % 4 == 0);
                    assert!(header.ends_with(if index + 1 == chunks.len() { "m=0" } else { "m=1" }));
                    bytes.extend(base64_simd::STANDARD.decode_to_vec(payload).unwrap());
                }
                let decoded = if compress {
                    let mut decoded = Vec::new();
                    flate2::read::ZlibDecoder::new(bytes.as_slice())
                        .read_to_end(&mut decoded)
                        .unwrap();
                    decoded
                } else {
                    bytes
                };
                assert_eq!(decoded, *image.as_raw());
            }
        }
    }

    #[test]
    fn swap_places_before_deleting_and_cleanup_covers_both_ids() {
        for tmux in [false, true] {
            for id in IMAGE_IDS {
                let swap = String::from_utf8(serialize_swap(id, Rect::new(3, 2, 80, 30), tmux).unwrap()).unwrap();
                assert!(swap.starts_with("\x1b[?2026h") && swap.ends_with("\x1b[?2026l"));
                assert!(swap.contains(&format!("a=p,i={id},p=1,c=80,r=30,C=1,q=2")));
                assert!(swap.contains(&format!("a=d,d=I,i={}", other_image_id(id))));
                assert!(swap.find("a=p").unwrap() < swap.find("a=d").unwrap());
                assert!(!swap.contains("m=") && !swap.contains('\u{10eeee}') && !swap.contains("\x1b[2J"));
                assert_eq!(swap.contains("\x1bPtmux;"), tmux);
            }
            let mut cleanup = Vec::new();
            clear_images(&mut cleanup, tmux).unwrap();
            let cleanup = String::from_utf8(cleanup).unwrap();
            for id in IMAGE_IDS {
                assert!(cleanup.contains(&format!("a=d,d=I,i={id}")));
            }
        }
    }
}
