//! Immutable hash-named blobs; a complete reference is the only publication point. No locks or eviction.
use serde::{Serialize, de::DeserializeOwned, Deserialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read, Write}, path::{Path, PathBuf}, time::SystemTime};
use crate::constants::MAST_DECODED_MAX_BYTES;
use crate::progress::{DatasetProgress, NoProgress, ActivityReader};

pub(crate) fn digest(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }
pub(crate) fn encode<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).map_err(io::Error::other)
}
pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    let (value, used) = bincode::serde::decode_from_slice(bytes, bincode::config::standard().with_limit::<MAST_DECODED_MAX_BYTES>()).map_err(io::Error::other)?;
    if used != bytes.len() { return Err(invalid("trailing cache data")); }
    Ok(value)
}
pub(super) fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }
fn at_path(path: &Path, error: io::Error) -> io::Error { io::Error::new(error.kind(), format!("{}: {error}", path.display())) }

#[derive(Clone)]
pub(crate) struct BlobStore {
    root: PathBuf,
    #[cfg(test)]
    reads: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>,
}
#[derive(Serialize, Deserialize)]
struct Reference { version: u8, key: String, hash: String, bytes: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlobStamp { bytes: u64, modified: SystemTime }
/// A locator only: metadata agrees with the reference, but payload bytes have not been checked.
pub(crate) struct LocatedBlob { pub hash: String, pub bytes: u64, pub stamp: BlobStamp }
pub(crate) struct VerifiedBlob { pub bytes: Vec<u8>, pub hash: String, pub stamp: BlobStamp }

impl BlobStamp {
    fn read(metadata: &fs::Metadata) -> io::Result<Self> { Ok(Self { bytes: metadata.len(), modified: metadata.modified()? }) }
}
impl BlobStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root, #[cfg(test)] reads: Default::default() }
    }
    #[cfg(test)]
    pub(crate) fn payload_reads(&self, hash: &str) -> usize { *self.reads.lock().unwrap().get(hash).unwrap_or(&0) }
    #[cfg(test)]
    fn record_read(&self, hash: &str) { *self.reads.lock().unwrap().entry(hash.into()).or_default() += 1; }
    fn reference(&self, key: &str) -> PathBuf { self.root.join("references").join(format!("{}.toml", digest(key.as_bytes()))) }
    fn object_path(&self, hash: &str) -> io::Result<PathBuf> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(invalid("invalid MAST object hash")); }
        Ok(self.root.join("objects").join(hash))
    }

    #[cfg(test)]
    pub(crate) fn read(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.read_verified(key)?.map(|blob| blob.bytes))
    }
    pub(crate) fn read_with_progress(&self, key: &str, progress: &mut dyn DatasetProgress) -> io::Result<Option<Vec<u8>>> {
        Ok(self.read_verified_with_progress(key, progress)?.map(|blob| blob.bytes))
    }
    pub(crate) fn read_verified(&self, key: &str) -> io::Result<Option<VerifiedBlob>> {
        self.read_verified_with_progress(key, &mut NoProgress(io::sink()))
    }
    pub(crate) fn read_verified_with_progress(&self, key: &str, progress: &mut dyn DatasetProgress) -> io::Result<Option<VerifiedBlob>> {
        let Some(reference) = self.read_reference(key)? else { return Ok(None); };
        let path = self.reference(key);
        let blob = self.read_verified_object(&reference.hash, progress).map_err(|error| at_path(&path, error))?;
        if reference.bytes != blob.bytes.len() as u64 { return Err(at_path(&path, invalid("MAST cache size mismatch"))); }
        Ok(Some(blob))
    }

    /// Locate a raw object without opening or hashing its payload. A later consumer must verify it.
    pub(crate) fn locate(&self, key: &str) -> io::Result<Option<LocatedBlob>> {
        let Some(reference) = self.read_reference(key)? else { return Ok(None); };
        let path = self.object_path(&reference.hash)?;
        let metadata = fs::metadata(&path).map_err(|error| at_path(&path, error))?;
        if !metadata.is_file() || metadata.len() != reference.bytes { return Err(at_path(&path, invalid("MAST cache size/type mismatch"))); }
        let stamp = BlobStamp::read(&metadata).map_err(|error| at_path(&path, error))?;
        Ok(Some(LocatedBlob { hash: reference.hash, bytes: reference.bytes, stamp }))
    }

    fn read_reference(&self, key: &str) -> io::Result<Option<Reference>> {
        let path = self.reference(key);
        let text = match read_limited(&path, 16 * 1024) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let reference: Reference = (|| {
            let text = std::str::from_utf8(&text).map_err(io::Error::other)?;
            toml::from_str(text).map_err(io::Error::other)
        })().map_err(|error| at_path(&path, error))?;
        if reference.version != 1 || reference.key != key || reference.bytes > MAST_DECODED_MAX_BYTES as u64 {
            return Err(at_path(&path, invalid("invalid MAST cache reference")));
        }
        self.object_path(&reference.hash).map_err(|error| at_path(&path, error))?;
        Ok(Some(reference))
    }
    /// Read the captured object even if another publisher later replaces its reference.
    pub(crate) fn read_object(&self, hash: &str) -> io::Result<Vec<u8>> { self.read_object_with_progress(hash, &mut NoProgress(io::sink())) }

    pub(crate) fn read_object_with_progress(&self, hash: &str, progress: &mut dyn DatasetProgress) -> io::Result<Vec<u8>> {
        Ok(self.read_verified_object(hash, progress)?.bytes)
    }
    fn read_verified_object(&self, hash: &str, progress: &mut dyn DatasetProgress) -> io::Result<VerifiedBlob> {
        let path = self.object_path(hash)?;
        #[cfg(test)]
        self.record_read(hash);
        let (bytes, stamp) = read_stamped(&path, MAST_DECODED_MAX_BYTES, progress)?;
        progress.set_detail("Verifying cached checksum");
        if digest(&bytes) != hash { return Err(at_path(&path, invalid("MAST cache checksum mismatch"))); }
        Ok(VerifiedBlob { bytes, hash: hash.into(), stamp })
    }
    /// The final preparation check compares metadata, without rereading and hashing every source blob.
    pub(crate) fn check_unchanged(&self, hash: &str, expected: BlobStamp) -> io::Result<()> {
        let path = self.object_path(hash)?;
        let current = fs::metadata(&path).and_then(|metadata| BlobStamp::read(&metadata)).map_err(|error| at_path(&path, error))?;
        if current != expected { return Err(at_path(&path, invalid("MAST input changed during preparation"))); }
        Ok(())
    }
    /// Pins use no-clobber references. Repair may restore the original bytes but never repin a new identity.
    pub(crate) fn publish(&self, key: &str, bytes: &[u8], pin: bool) -> io::Result<VerifiedBlob> {
        if bytes.len() > MAST_DECODED_MAX_BYTES { return Err(invalid("MAST cache exceeds byte limit")); }
        let hash = digest(bytes);
        let object = self.object_path(&hash)?;
        fs::create_dir_all(object.parent().unwrap())?;
        #[cfg(test)]
        self.record_read(&hash);
        if !read_limited(&object, MAST_DECODED_MAX_BYTES).is_ok_and(|old| digest(&old) == hash) { atomic_write(&object, bytes, false)?; }
        let stamp = fs::metadata(&object).and_then(|m| BlobStamp::read(&m)).map_err(|error| at_path(&object, error))?;
        let reference = Reference { version: 1, key: key.into(), hash: hash.clone(), bytes: bytes.len() as u64 };
        let path = self.reference(key);
        fs::create_dir_all(path.parent().unwrap())?;
        let installed = atomic_write(&path, toml::to_string(&reference).map_err(io::Error::other)?.as_bytes(), pin)?;
        if !installed {
            let winner = self.read_verified(key)?.ok_or_else(|| at_path(&path, invalid("missing pinned MAST reference")))?;
            if winner.bytes != bytes { return Err(at_path(&path, invalid("MAST catalog identity changed; refusing to mix revisions"))); }
            return Ok(winner);
        }
        Ok(VerifiedBlob { bytes: bytes.to_vec(), hash, stamp })
    }
}
fn atomic_write(path: &Path, bytes: &[u8], no_clobber: bool) -> io::Result<bool> {
    let result = (|| {
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        if no_clobber {
            match temporary.persist_noclobber(path) {
                Ok(_) => {},
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
                Err(error) => return Err(error.error),
            }
        } else { temporary.persist(path).map_err(|error| error.error)?; }
        #[cfg(unix)]
        fs::File::open(path.parent().unwrap())?.sync_all()?;
        Ok(true)
    })();
    result.map_err(|error| at_path(path, error))
}
pub(super) fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> { Ok(read_stamped(path, limit, &mut NoProgress(io::sink()))?.0) }
fn read_stamped(path: &Path, limit: usize, progress: &mut dyn DatasetProgress) -> io::Result<(Vec<u8>, BlobStamp)> {
    let result = (|| {
        let mut file = fs::File::open(path)?;
        let before = BlobStamp::read(&file.metadata()?)?;
        if before.bytes > limit as u64 { return Err(invalid("MAST cache exceeds byte limit")); }
        let capacity = usize::try_from(before.bytes).ok().and_then(|bytes| bytes.checked_add(1)).ok_or_else(|| invalid("MAST cache capacity overflow"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(io::Error::other)?; // reserve once, including room to check for data past the expected end
        ActivityReader::new(Read::by_ref(&mut file).take(limit as u64 + 1), progress, before.bytes).read_to_end(&mut bytes)?;
        let after = BlobStamp::read(&file.metadata()?)?;
        if bytes.len() as u64 != before.bytes || before != after { return Err(invalid("MAST cache changed during read")); }
        Ok((bytes, after))
    })();
    result.map_err(|error| at_path(path, error))
}

#[cfg(test)]
mod tests;
