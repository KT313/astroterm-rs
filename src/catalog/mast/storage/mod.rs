//! Immutable hash-named blobs; a complete reference is the only publication point. No locks or eviction.
use serde::{Serialize, de::DeserializeOwned, Deserialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read, Write}, path::{Path, PathBuf}};
use crate::constants::MAST_DECODED_MAX_BYTES;

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

#[derive(Clone)]
pub(crate) struct BlobStore { root: PathBuf }
#[derive(Serialize, Deserialize)]
struct Reference { version: u8, key: String, hash: String, bytes: u64 }
impl BlobStore {
    pub(crate) fn new(root: PathBuf) -> Self { Self { root } }
    fn reference(&self, key: &str) -> PathBuf { self.root.join("references").join(format!("{}.toml", digest(key.as_bytes()))) }
    pub(crate) fn read(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        let text = match read_limited(&self.reference(key), 16 * 1024) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let reference: Reference = toml::from_str(std::str::from_utf8(&text).map_err(io::Error::other)?).map_err(io::Error::other)?;
        if reference.version != 1 || reference.key != key || reference.hash.len() != 64 || !reference.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid MAST cache reference"));
        }
        let bytes = self.read_object(&reference.hash)?;
        if reference.bytes != bytes.len() as u64 { return Err(invalid("MAST cache size mismatch")); }
        Ok(Some(bytes))
    }
    /// Read the resolved immutable object even if another publisher subsequently replaces its reference.
    pub(crate) fn read_object(&self, hash: &str) -> io::Result<Vec<u8>> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(invalid("invalid MAST object hash")); }
        let bytes = read_limited(&self.root.join("objects").join(hash), MAST_DECODED_MAX_BYTES)?;
        if digest(&bytes) != hash { return Err(invalid("MAST cache checksum mismatch")); }
        Ok(bytes)
    }
    /// Pins use no-clobber references; ordinary derived results may atomically replace their reference.
    pub(crate) fn publish(&self, key: &str, bytes: &[u8], pin: bool) -> io::Result<Vec<u8>> {
        if bytes.len() > MAST_DECODED_MAX_BYTES { return Err(invalid("MAST cache exceeds byte limit")); }
        let hash = digest(bytes);
        let object = self.root.join("objects").join(&hash);
        fs::create_dir_all(object.parent().unwrap())?;
        if !read_limited(&object, MAST_DECODED_MAX_BYTES).is_ok_and(|old| digest(&old) == hash) { atomic_write(&object, bytes, false)?; }
        let reference = Reference { version: 1, key: key.into(), hash, bytes: bytes.len() as u64 };
        let path = self.reference(key);
        fs::create_dir_all(path.parent().unwrap())?;
        let installed = atomic_write(&path, toml::to_string(&reference).map_err(io::Error::other)?.as_bytes(), pin)?;
        if !installed {
            let winner = self.read(key)?.ok_or_else(|| invalid("missing pinned MAST reference"))?;
            if winner != bytes { return Err(invalid("MAST catalog identity changed; refusing to mix revisions")); }
            return Ok(winner);
        }
        Ok(bytes.to_vec())
    }
}
fn atomic_write(path: &Path, bytes: &[u8], no_clobber: bool) -> io::Result<bool> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if no_clobber {
        match temporary.persist_noclobber(path) {
            Ok(_) => {},
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(e.error),
        }
    } else { temporary.persist(path).map_err(|e| e.error)?; }
    #[cfg(unix)]
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(true)
}
pub(super) fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let before = file.metadata()?;
    if before.len() > limit as u64 { return Err(invalid("MAST cache exceeds byte limit")); }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len() || before.modified()? != after.modified()? || before.len() != after.len() {
        return Err(invalid("MAST cache changed during read"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_blobs_survive_reference_replacement_and_corruption_is_detected() {
        let dir=tempfile::tempdir().unwrap();let store=BlobStore::new(dir.path().into());
        store.publish("block",b"old",false).unwrap();let old=dir.path().join("objects").join(digest(b"old"));
        store.publish("block",b"new",false).unwrap();assert_eq!(fs::read(old).unwrap(),b"old");
        assert_eq!(store.read_object(&digest(b"old")).unwrap(),b"old");
        assert_eq!(store.read("block").unwrap().unwrap(),b"new");
        fs::write(dir.path().join("objects").join(digest(b"new")),b"bad").unwrap();assert!(store.read("block").is_err());
        store.publish("block",b"new",false).unwrap();assert_eq!(store.read("block").unwrap().unwrap(),b"new");
        store.publish("pin",b"identity",true).unwrap();assert!(store.publish("pin",b"changed",true).is_err());
        assert_eq!(store.read("pin").unwrap().unwrap(),b"identity");
    }
    #[test]
    fn concurrent_publication_always_points_at_one_complete_blob() {
        let dir=tempfile::tempdir().unwrap();let store=BlobStore::new(dir.path().into());
        std::thread::scope(|scope| {
            for byte in [1,2] {let store=&store;scope.spawn(move|| {for _ in 0..8 {store.publish("result",&vec![byte;4096],false).unwrap();}});}
        });
        let bytes=store.read("result").unwrap().unwrap();assert_eq!(bytes.len(),4096);assert!(bytes.iter().all(|b|*b==bytes[0]));
    }
}
