//! Aligned, checksummed cache container. The higher sky layer owns the catalog schema and semantic validation.
mod array;
pub(crate) mod encoding;
pub use array::CatalogArray;
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    ops::Range,
    path::Path,
};

const MAGIC: &[u8; 8] = b"ASTROCAT";
const VERSION: u32 = 3; // 3: preparation bounds separate; fast movers represented by the grid tail
const PREFIX: usize = 64;
const MAX_SECTIONS: usize = 64;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub(crate) fn supported() -> bool {
    cfg!(target_endian = "little") && usize::BITS == 64
}

/// Temporary owned snapshot of a prepared cache file. Typed columns are copied out before publication.
#[derive(Debug)]
pub struct PreparedCatalogBytes {
    bytes: Vec<u8>,
    sections: Vec<Range<usize>>,
}
impl PreparedCatalogBytes {
    pub(crate) fn open(path: &Path, fingerprint: &[u8; 32]) -> io::Result<Self> {
        if !supported() {
            return Err(invalid("prepared cache requires a 64-bit little-endian target"));
        }
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if bytes.len() < PREFIX { return Err(invalid("truncated cache header")); }
        if &bytes[..8] != MAGIC || read_u32(&bytes, 8)? != VERSION || &bytes[16..48] != fingerprint {
            return Err(invalid("cache magic/version/fingerprint mismatch"));
        }
        let count = read_u32(&bytes, 12)? as usize;
        if count > MAX_SECTIONS || read_u64(&bytes, 48)? != bytes.len() as u64 {
            return Err(invalid("cache length mismatch"));
        }
        let header_end = PREFIX + count * 16;
        if header_end > bytes.len() {
            return Err(invalid("truncated section table"));
        }
        let expected = read_u32(&bytes, 56)?;
        let mut checksum = crc32fast::Hasher::new();
        checksum.update(&bytes[..56]);
        checksum.update(&bytes[60..]);
        if checksum.finalize() != expected {
            return Err(invalid("cache checksum mismatch"));
        }
        let mut previous = header_end;
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let start =
                usize::try_from(read_u64(&bytes, PREFIX + i * 16)?).map_err(|_| invalid("offset overflow"))?;
            let length =
                usize::try_from(read_u64(&bytes, PREFIX + i * 16 + 8)?).map_err(|_| invalid("length overflow"))?;
            let end = start.checked_add(length).ok_or_else(|| invalid("section overflow"))?;
            if start < previous || start % 8 != 0 || end > bytes.len() {
                return Err(invalid("invalid section range"));
            }
            sections.push(start..end);
            previous = end;
        }
        if previous != bytes.len() {
            return Err(invalid("trailing cache bytes"));
        }
        Ok(Self { bytes, sections })
    }
    pub(crate) fn section_count(&self) -> usize {
        self.sections.len()
    }
    pub(crate) fn section(&self, index: usize) -> io::Result<&[u8]> {
        let range = self.sections.get(index).ok_or_else(|| invalid("missing section"))?;
        Ok(&self.bytes[range.clone()])
    }
    /// Decode into aligned owned elements; the byte snapshot itself need not have T's alignment.
    pub(crate) fn decode<T: bytemuck::Pod>(&self, index: usize) -> io::Result<Vec<T>> {
        decode_elements(self.section(index)?)
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

/// Preserve packed bits without assuming alignment of the input byte buffer.
fn decode_elements<T: bytemuck::Pod>(bytes: &[u8]) -> io::Result<Vec<T>> {
    let width = std::mem::size_of::<T>();
    if width == 0 || !bytes.len().is_multiple_of(width) { return Err(invalid("section element-size mismatch")); }
    Ok(bytes.chunks_exact(width).map(bytemuck::pod_read_unaligned).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decode_accepts_unaligned_bytes_and_rejects_partial_elements() {
        let words = [0x0123456789abcdef_u64, u64::MAX];
        let mut bytes = vec![0; 1 + std::mem::size_of_val(&words)];
        bytes[1..].copy_from_slice(bytemuck::cast_slice(&words));
        assert_eq!(decode_elements::<u64>(&bytes[1..]).unwrap(), words);
        assert!(decode_elements::<u64>(&bytes[2..]).is_err());
        assert!(decode_elements::<()>(&[]).is_err());
    }
}
