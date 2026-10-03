//! Dataset names, per-user locations and verified streaming downloads. No terminal state is active here.
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const ATHYG_URL: &str = "https://codeberg.org/astronexus/athyg/media/branch/main/data/athyg_40.csv.gz";
pub const ATHYG_SHA256: &str = "69ad04dd33d7c7bb4f5e1b4682798075811547ea9fb8d0e802e5b319c46818a6";
pub const ATHYG_BYTES: u64 = 199_688_001;
const ATHYG_FILE: &str = "athyg_40.csv.gz";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dataset {
    Athyg,
    Path(PathBuf),
}
impl Dataset {
    /// Existing files win over names. A separator explicitly selects a path, including a missing path.
    pub fn parse(value: &OsStr) -> Result<Self, String> {
        let path = Path::new(value);
        let has_separator = value.as_encoded_bytes().iter().any(|&b| b == b'/' || b == b'\\');
        if path.is_file() || has_separator {
            return Ok(Self::Path(path.to_owned()));
        }
        if value == "athyg" {
            return Ok(Self::Athyg);
        }
        Err(format!(
            "Unknown dataset '{}'. Known names: athyg. For a file path use ./<file>.",
            path.display()
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct DatasetDirectories {
    pub data: Option<PathBuf>,
    pub cache: Option<PathBuf>,
}
impl DatasetDirectories {
    pub fn for_user() -> Self {
        Self {
            data: dirs::data_dir().map(|p| p.join("astroterm")),
            cache: dirs::cache_dir().map(|p| p.join("astroterm")),
        }
    }
}

pub fn resolve_dataset(
    dataset: &Dataset,
    directories: &DatasetDirectories,
    notices: &mut impl Write,
) -> io::Result<PathBuf> {
    match dataset {
        Dataset::Path(path) => Ok(path.clone()),
        Dataset::Athyg => {
            let directory = directories.data.as_ref().ok_or_else(|| {
                io::Error::other(format!(
                    "Cannot find the per-user data directory. Download {ATHYG_URL} manually and use --dataset <path>."
                ))
            })?;
            let target = directory.join(ATHYG_FILE);
            if target.is_file() {
                return Ok(target);
            }
            writeln!(
                notices,
                "Downloading AT-HYG v4.0 ({ATHYG_BYTES} bytes, about 200 MB) to {}",
                target.display()
            )?;
            let result = (|| {
                fs::create_dir_all(directory)?;
                let agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(1800)))
                    .timeout_connect(Some(Duration::from_secs(30)))
                    .timeout_recv_response(Some(Duration::from_secs(30)))
                    .build()
                    .new_agent();
                let mut response = agent
                    .get(ATHYG_URL)
                    .header("Accept-Encoding", "identity")
                    .call()
                    .map_err(io::Error::other)?;
                install_verified(
                    response.body_mut().as_reader(),
                    &target,
                    ATHYG_BYTES,
                    ATHYG_SHA256,
                    notices,
                )
            })();
            result.map_err(|error|io::Error::other(format!(
                "AT-HYG download failed: {error}. Download {ATHYG_URL} manually to {} or use --dataset <path>. No partial download was installed.",target.display())))?;
            Ok(target)
        }
    }
}

/// Verify the exact compressed bytes, bounded by the pinned size. RAII removes the temporary on every error.
fn install_verified(
    mut source: impl Read,
    target: &Path,
    expected_size: u64,
    expected_hash: &str,
    notices: &mut impl Write,
) -> io::Result<()> {
    let directory = target
        .parent()
        .ok_or_else(|| io::Error::other("missing data directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut last = Instant::now();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > expected_size {
            return Err(io::Error::other("download is larger than the pinned dataset"));
        }
        hash.update(&buffer[..read]);
        temporary.write_all(&buffer[..read])?;
        if last.elapsed() >= Duration::from_secs(1) {
            writeln!(
                notices,
                "Downloaded {count}/{expected_size} bytes ({:.0}%)",
                100.0 * count as f64 / expected_size as f64
            )?;
            last = Instant::now();
        }
    }
    if count != expected_size || hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>() != expected_hash
    {
        return Err(io::Error::other("dataset size or SHA-256 mismatch"));
    }
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    writeln!(notices, "Downloaded and verified {count} bytes.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_explicit_or_existing_paths_resolve() {
        assert_eq!(Dataset::parse(OsStr::new("athyg")).unwrap(), Dataset::Athyg);
        assert_eq!(
            Dataset::parse(OsStr::new("./athyg")).unwrap(),
            Dataset::Path("./athyg".into())
        );
        assert!(
            Dataset::parse(OsStr::new("unknown"))
                .unwrap_err()
                .contains("Known names: athyg")
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athyg");
        fs::write(&path, b"local").unwrap();
        assert_eq!(Dataset::parse(path.as_os_str()).unwrap(), Dataset::Path(path));
        assert!(matches!(
            Dataset::parse(OsStr::new("some\\file.csv")),
            Ok(Dataset::Path(_))
        ));
    }
    #[test]
    fn verified_local_stream_installs_only_complete_matching_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("data.gz");
        let source = dir.path().join("source");
        fs::write(&source, b"catalog bytes").unwrap();
        let hash = Sha256::digest(b"catalog bytes")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        install_verified(fs::File::open(&source).unwrap(), &target, 13, &hash, &mut Vec::new()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"catalog bytes");
        for (bytes, size, hash) in [
            (&b"wrong"[..], 5, "bad"),
            (&b"short"[..], 13, &hash),
            (&b"long"[..], 1, &hash),
        ] {
            assert!(install_verified(bytes, &target, size, hash, &mut Vec::new()).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"catalog bytes");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }
    #[test]
    fn interrupted_reader_removes_its_temporary_file() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("offline"))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(install_verified(Broken, &dir.path().join("data"), 10, ATHYG_SHA256, &mut Vec::new()).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn offline_copy_is_reused_and_missing_data_location_gives_manual_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ATHYG_FILE);
        fs::write(&path, b"existing").unwrap();
        let directories = DatasetDirectories {
            data: Some(dir.path().to_owned()),
            cache: None,
        };
        assert_eq!(
            resolve_dataset(&Dataset::Athyg, &directories, &mut Vec::new()).unwrap(),
            path
        );
        let error = resolve_dataset(&Dataset::Athyg, &DatasetDirectories::default(), &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains(ATHYG_URL) && error.to_string().contains("--dataset <path>"));
    }
}
