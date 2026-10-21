//! Version 9: a bounded directory, independent compressed chunks, then a CRC32 trailer.
use super::{invalid, supported};
use crate::{constants::CACHE_CHUNK_BYTES, progress::{DatasetProgress, Stage, Unit, Outcome}};
use crate::time::StepTimes;
use std::{fs::File, io::{self, Read, Write}, path::Path};

const MAGIC: &[u8; 8] = b"ASTROCAT";
const VERSION: u32 = 9;
const PREFIX: usize = 56;                              // magic, version, count, fingerprint, uncompressed chunk size
const MAX_SECTIONS: usize = 64;

pub(crate) struct PreparedCatalogReader {
    file: File,
    lengths: Vec<usize>,
    next: usize,
    checksum: crc32fast::Hasher,
    consumed: u64,
    file_size: u64,
    modified: Option<std::time::SystemTime>,
    compressed: Vec<u8>,
    decoder: zstd::bulk::Decompressor<'static>,
}
impl PreparedCatalogReader {
    pub(crate) fn open(path: &Path, fingerprint: &[u8; 32], progress: &mut dyn DatasetProgress) -> io::Result<Self> {
        if !supported() { return Err(invalid("prepared cache requires a 64-bit little-endian target")); }
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        let mut header = [0u8; PREFIX];
        file.read_exact(&mut header)?;
        if &header[..8] != MAGIC || u32_at(&header, 8) != VERSION || &header[16..48] != fingerprint {
            return Err(invalid("cache magic/version/fingerprint mismatch"));
        }
        let count = u32_at(&header, 12) as usize;
        if count > MAX_SECTIONS || u64_at(&header, 48) != CACHE_CHUNK_BYTES as u64 { return Err(invalid("invalid cache directory/chunk size")); }
        let mut directory = vec![0u8; count * 16];
        file.read_exact(&mut directory)?;
        let mut checksum = crc32fast::Hasher::new();
        checksum.update(&header);
        checksum.update(&directory);
        let consumed = (PREFIX + directory.len()) as u64;
        let mut minimum = consumed.checked_add(4).ok_or_else(|| invalid("cache size overflow"))?;
        let mut lengths = Vec::with_capacity(count);
        for (index, entry) in directory.chunks_exact(16).enumerate() {
            if u64_at(entry, 0) != index as u64 { return Err(invalid("invalid cache section identity")); }
            let length = usize::try_from(u64_at(entry, 8)).map_err(|_| invalid("section size overflow"))?;
            let chunks = length.div_ceil(CACHE_CHUNK_BYTES) as u64;
            minimum = minimum.checked_add(chunks.checked_mul(9).ok_or_else(|| invalid("chunk count overflow"))?).ok_or_else(|| invalid("cache size overflow"))?;
            lengths.push(length);
        }
        if minimum > metadata.len() { return Err(invalid("impossible cache lengths")); }
        progress.begin_stage(Stage::ReadCache, "Compressed prepared catalog", Unit::Bytes, Some(metadata.len()));
        progress.set_position(consumed);
        let mut decoder = zstd::bulk::Decompressor::new()?;
        decoder.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(CACHE_CHUNK_BYTES.ilog2()))?;
        Ok(Self { file, lengths, next: 0, checksum, consumed, file_size: metadata.len(), modified: metadata.modified().ok(),
            compressed: Vec::new(), decoder })
    }

    pub(crate) fn lengths(&self) -> &[usize] { &self.lengths }

    /// Decompress directly into safely initialized final column storage; no whole-file byte snapshot.
    pub(crate) fn decode<T: bytemuck::Pod>(&mut self, index: usize, progress: &mut dyn DatasetProgress) -> io::Result<Vec<T>> {
        if index != self.next { return Err(invalid("cache sections must be read in order")); }
        let length = *self.lengths.get(index).ok_or_else(|| invalid("missing cache section"))?;
        let width = std::mem::size_of::<T>();
        if width == 0 || !length.is_multiple_of(width) { return Err(invalid("section element-size mismatch")); }
        let mut output = Vec::new();
        output.try_reserve_exact(length / width).map_err(io::Error::other)?;
        let mut offset = 0;
        while offset < length {
            let end = offset + (length - offset).min(CACHE_CHUNK_BYTES);
            output.resize(end.div_ceil(width), T::zeroed());                                // touch only one chunk before its compressed input is checked
            self.read_chunk(&mut bytemuck::cast_slice_mut::<T, u8>(&mut output)[offset..end], progress)?;
            offset = end;
        }
        self.next += 1;
        Ok(output)
    }

    fn read_chunk(&mut self, output: &mut [u8], progress: &mut dyn DatasetProgress) -> io::Result<()> {
        let mut encoded = [0u8; 8];
        self.read_hashed(&mut encoded)?;
        let length = usize::try_from(u64::from_le_bytes(encoded)).map_err(|_| invalid("compressed length overflow"))?;
        if length == 0 || length > zstd::zstd_safe::compress_bound(output.len())
            || length as u64 > self.file_size.saturating_sub(self.consumed).saturating_sub(4) {
            return Err(invalid("invalid compressed chunk length"));
        }
        self.compressed.resize(length, 0);
        self.file.read_exact(&mut self.compressed)?;
        self.checksum.update(&self.compressed);
        self.consumed += length as u64;
        let frame_length = zstd::zstd_safe::find_frame_compressed_size(&self.compressed).map_err(|_| invalid("invalid compressed frame"))?;
        if frame_length != length { return Err(invalid("extra compressed frame/trailing chunk bytes")); }
        if self.decoder.decompress_to_buffer(&self.compressed, output)? != output.len() { return Err(invalid("decompressed chunk length mismatch")); }
        progress.set_position(self.consumed);
        Ok(())
    }

    fn read_hashed(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.file.read_exact(bytes)?;
        self.checksum.update(bytes);
        self.consumed += bytes.len() as u64;
        Ok(())
    }

    pub(crate) fn finish(mut self, progress: &mut dyn DatasetProgress) -> io::Result<()> {
        if self.next != self.lengths.len() { return Err(invalid("unread cache sections")); }
        let mut trailer = [0u8; 4];
        self.file.read_exact(&mut trailer)?;
        if u32::from_le_bytes(trailer) != self.checksum.finalize() { return Err(invalid("cache checksum mismatch")); }
        let mut extra = [0u8; 1];
        if self.consumed + 4 != self.file_size || self.file.read(&mut extra)? != 0 { return Err(invalid("trailing cache bytes")); }
        let metadata = self.file.metadata()?;
        if metadata.len() != self.file_size || metadata.modified().ok() != self.modified { return Err(invalid("cache changed during reading")); }
        progress.set_position(self.file_size);
        progress.finish_stage(Outcome::Complete);
        progress.begin_stage(Stage::ValidateCache, "Checking prepared catalog", Unit::Work, None);
        Ok(())
    }
}

/// Publish only a complete, synced compressed file. CRC covers the exact stream before its trailer.
pub(crate) fn write_sections(path: &Path, fingerprint: &[u8; 32], sections: &[&[u8]], progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    if !supported() || sections.len() > MAX_SECTIONS { return Err(invalid("unsupported cache layout")); }
    let directory = path.parent().ok_or_else(|| invalid("missing cache directory"))?;
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut header = Vec::with_capacity(PREFIX + sections.len() * 16);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&(sections.len() as u32).to_le_bytes());
    header.extend_from_slice(fingerprint);
    header.extend_from_slice(&(CACHE_CHUNK_BYTES as u64).to_le_bytes());
    let mut total = 0u64;
    for (index, section) in sections.iter().enumerate() {
        header.extend_from_slice(&(index as u64).to_le_bytes());
        header.extend_from_slice(&(section.len() as u64).to_le_bytes());
        total = total.checked_add(section.len() as u64).ok_or_else(|| invalid("cache size overflow"))?;
    }
    progress.begin_stage(Stage::WriteCache, "Uncompressed bytes processed", Unit::Bytes, Some(total));
    let mut checksum = crc32fast::Hasher::new();
    times.measure("Dataset loading.Prepared cache.Prepared catalog write.Prepared section writing");
    {
        write_hashed(&mut temporary, &mut checksum, &header)?;
        let mut compressor = zstd::bulk::Compressor::new(1)?;
        let mut compressed = vec![0u8; zstd::zstd_safe::compress_bound(CACHE_CHUNK_BYTES)];
        let mut processed = 0u64;
        for section in sections {
            for chunk in section.chunks(CACHE_CHUNK_BYTES) {
                let length = compressor.compress_to_buffer(chunk, compressed.as_mut_slice())?;
                write_hashed(&mut temporary, &mut checksum, &(length as u64).to_le_bytes())?;
                write_hashed(&mut temporary, &mut checksum, &compressed[..length])?;
                processed += chunk.len() as u64;
                progress.set_position(processed);
            }
        }
    };
    progress.finish_stage(Outcome::Complete);
    progress.begin_stage(Stage::FinalizeCache, "Finalizing prepared catalog", Unit::Work, None);
    times.measure("Dataset loading.Prepared cache.Prepared catalog write.Prepared catalog finalization");
    {
        temporary.write_all(&checksum.finalize().to_le_bytes())?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
    };
    progress.finish_stage(Outcome::Complete);
    Ok(())
}
fn write_hashed(out: &mut impl Write, checksum: &mut crc32fast::Hasher, bytes: &[u8]) -> io::Result<()> {
    out.write_all(bytes)?;
    checksum.update(bytes);
    Ok(())
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 { u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) }
fn u64_at(bytes: &[u8], offset: usize) -> u64 { u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) }

#[cfg(test)]
mod tests;
