//! Aligned, checksummed cache container. The higher sky layer owns the catalog schema and semantic validation.
mod array;
pub(crate) mod encoding;
pub use array::CatalogArray;
use memmap2::{Mmap, MmapOptions};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    ops::Range,
    path::Path,
    sync::Arc,
};

const MAGIC: &[u8; 8] = b"ASTROCAT";
const VERSION: u32 = 1;
const PREFIX: usize = 64;
const MAX_SECTIONS: usize = 64;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub(crate) fn supported() -> bool {
    cfg!(target_endian = "little") && usize::BITS == 64
}

/// Owns a read-only mapping and validated byte ranges. Cache writers only atomically replace files; they never
/// truncate or modify an existing mapped inode. Other software must respect that same file-lifetime contract.
#[derive(Debug)]
pub struct MappedCatalog {
    mapping: Mmap,
    sections: Vec<Range<usize>>,
}
impl MappedCatalog {
    pub(crate) fn open(path: &Path, fingerprint: &[u8; 32]) -> io::Result<Arc<Self>> {
        if !supported() {
            return Err(invalid("mapped cache requires a 64-bit little-endian target"));
        }
        let file = File::open(path)?;
        if file.metadata()?.len() < PREFIX as u64 {
            return Err(invalid("truncated cache header"));
        }
        // SAFETY: only application-managed cache files are mapped. Writers use atomic replacement, so concurrent
        // rebuilds cannot alter/truncate this inode. All ranges/types/semantics are validated before publication.
        let mapping = unsafe { MmapOptions::new().map(&file)? };
        if &mapping[..8] != MAGIC || read_u32(&mapping, 8)? != VERSION || &mapping[16..48] != fingerprint {
            return Err(invalid("cache magic/version/fingerprint mismatch"));
        }
        let count = read_u32(&mapping, 12)? as usize;
        if count > MAX_SECTIONS || read_u64(&mapping, 48)? != mapping.len() as u64 {
            return Err(invalid("cache length mismatch"));
        }
        let header_end = PREFIX + count * 16;
        if header_end > mapping.len() {
            return Err(invalid("truncated section table"));
        }
        let expected = read_u32(&mapping, 56)?;
        let mut checksum = crc32fast::Hasher::new();
        checksum.update(&mapping[..56]);
        checksum.update(&mapping[60..]);
        if checksum.finalize() != expected {
            return Err(invalid("cache checksum mismatch"));
        }
        let mut previous = header_end;
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let start =
                usize::try_from(read_u64(&mapping, PREFIX + i * 16)?).map_err(|_| invalid("offset overflow"))?;
            let length =
                usize::try_from(read_u64(&mapping, PREFIX + i * 16 + 8)?).map_err(|_| invalid("length overflow"))?;
            let end = start.checked_add(length).ok_or_else(|| invalid("section overflow"))?;
            if start < previous || start % 8 != 0 || end > mapping.len() {
                return Err(invalid("invalid section range"));
            }
            sections.push(start..end);
            previous = end;
        }
        if previous != mapping.len() {
            return Err(invalid("trailing cache bytes"));
        }
        Ok(Arc::new(Self { mapping, sections }))
    }
    pub(crate) fn section_count(&self) -> usize {
        self.sections.len()
    }
    pub(crate) fn slice<T: bytemuck::Pod>(&self, index: usize) -> io::Result<&[T]> {
        let range = self.sections.get(index).ok_or_else(|| invalid("missing section"))?;
        bytemuck::try_cast_slice(&self.mapping[range.clone()]).map_err(|_| invalid("section type/alignment mismatch"))
    }
}
fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated integer"))?
            .try_into()
            .unwrap(),
    ))
}
fn read_u64(bytes: &[u8], offset: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| invalid("truncated integer"))?
            .try_into()
            .unwrap(),
    ))
}

/// Write in the destination folder and atomically replace the final name only after the complete file is synced.
pub(crate) fn write_sections(path: &Path, fingerprint: &[u8; 32], sections: &[&[u8]]) -> io::Result<()> {
    if !supported() || sections.len() > MAX_SECTIONS {
        return Err(invalid("unsupported cache layout"));
    }
    let directory = path.parent().ok_or_else(|| invalid("missing cache directory"))?;
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut header = vec![0_u8; PREFIX + 16 * sections.len()];
    header[..8].copy_from_slice(MAGIC);
    header[8..12].copy_from_slice(&VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    header[16..48].copy_from_slice(fingerprint);
    let mut offset = header.len();
    for (i, section) in sections.iter().enumerate() {
        offset = (offset + 7) & !7;
        header[PREFIX + i * 16..PREFIX + i * 16 + 8].copy_from_slice(&(offset as u64).to_le_bytes());
        header[PREFIX + i * 16 + 8..PREFIX + i * 16 + 16].copy_from_slice(&(section.len() as u64).to_le_bytes());
        offset += section.len();
    }
    header[48..56].copy_from_slice(&(offset as u64).to_le_bytes());
    temporary.write_all(&header)?;
    let mut position = header.len();
    for section in sections {
        let padding = (8 - position % 8) % 8;
        temporary.write_all(&[0; 8][..padding])?;
        temporary.write_all(section)?;
        position += padding + section.len();
    }
    temporary.rewind()?;
    let mut checksum = crc32fast::Hasher::new();
    checksum.update(&header[..56]);
    checksum.update(&header[60..]);
    temporary.seek(io::SeekFrom::Start(header.len() as u64))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = temporary.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        checksum.update(&buffer[..count]);
    }
    temporary.seek(io::SeekFrom::Start(56))?;
    temporary.write_all(&checksum.finalize().to_le_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
