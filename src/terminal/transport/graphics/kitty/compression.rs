//! Independent zlib streams written directly into retained output capacity, without a per-image writer buffer.
use std::io;
use flate2::{Compress, Compression, FlushCompress, Status};
use crate::constants::KITTY_COMPRESSION_GROWTH_BYTES;

pub(super) fn compress_image(input: &[u8], engine: &mut Option<Compress>, output: &mut Vec<u8>) -> io::Result<()> {
    match engine {
        Some(engine) => engine.reset(),                                               // clear dictionary/checksum/history without reallocating the engine
        None => *engine = Some(Compress::new(Compression::fast(), true)),              // create lazily; unsupported terminals need no compressor
    }
    let engine = engine.as_mut().expect("compression engine initialized");
    output.clear();
    loop {
        if output.len() == output.capacity() { output.try_reserve(KITTY_COMPRESSION_GROWTH_BYTES.max(1)).map_err(io::Error::other)?; }
        let before = (engine.total_in(), engine.total_out());
        let remaining = &input[engine.total_in() as usize..];
        let flush = if remaining.is_empty() { FlushCompress::Finish } else { FlushCompress::None };
        let status = engine.compress_vec(remaining, output, flush).map_err(io::Error::other)?;
        if status == Status::StreamEnd {
            if engine.total_in() as usize != input.len() { return Err(io::Error::other("zlib finished before consuming the image")); }
            return Ok(());
        }
        if before == (engine.total_in(), engine.total_out()) { return Err(io::Error::other("zlib made no progress while encoding the image")); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn resets_produce_independent_streams_matching_the_previous_encoder() {
        let mut seed = 17u32;
        let noise: Vec<_> = (0..200_000).map(|_| { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 24) as u8 }).collect();
        let flat = vec![31; 300_000];
        let mut engine = None; let mut output = Vec::with_capacity(1);                  // force output growth and partial compression calls
        for input in [noise.as_slice(), &flat, &[], b"tiny", noise.as_slice(), &flat] {
            compress_image(input, &mut engine, &mut output).unwrap();
            let mut decoded = Vec::new();
            flate2::read::ZlibDecoder::new(output.as_slice()).read_to_end(&mut decoded).unwrap();
            assert_eq!(decoded, input);                                               // fresh decoder knows nothing about previous images
            let mut old = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
            old.write_all(input).unwrap();
            assert_eq!(output, old.finish().unwrap());                                // same backend/level and stream bytes as the prior path
            assert_eq!(engine.as_ref().unwrap().total_in(), input.len() as u64);
        }
        let capacity = output.capacity(); let pointer = output.as_ptr();
        compress_image(&flat, &mut engine, &mut output).unwrap();
        assert_eq!(output.capacity(), capacity); assert_eq!(output.as_ptr(), pointer);
    }

    #[test]
    fn reset_discards_an_unfinished_previous_image() {
        let mut unfinished = Compress::new(Compression::fast(), true);
        let mut partial = [0; 64];
        unfinished.compress(b"discard this interrupted input", &mut partial, FlushCompress::None).unwrap();
        assert!(unfinished.total_in() > 0);
        let mut engine = Some(unfinished); let mut output = vec![0xff; 50];
        compress_image(b"complete replacement", &mut engine, &mut output).unwrap();
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(output.as_slice()).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, b"complete replacement");
    }
}
