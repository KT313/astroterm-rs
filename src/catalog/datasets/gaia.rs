//! A fixed Gaia prefix with verified, independently reusable source files. No terminal is active during I/O.
use super::{DatasetDirectories, GAIA_DR3_FILE_COUNT};
use crate::constants::GAIA_DOWNLOAD_MAX_BYTES;
use crate::timing::StepTimes;
use crate::progress::{DatasetProgress, Stage, Unit, Outcome};
use md5::{Digest, Md5};
use std::{fs::{self, File}, io::{self, Read, Write}, path::{Path, PathBuf}, time::{Duration, SystemTime}};

const SOURCE_URL: &str = "https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source/";
const MANIFEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/gaia-dr3-source-md5.txt"));

#[derive(Clone, Debug)]
pub(crate) struct GaiaEntry {
    pub name: String,
    pub checksum: [u8; 16],
    pub first_cell: u32,
    pub last_cell: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct GaiaFile {
    pub entry: GaiaEntry,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}
impl GaiaFile {
    #[cfg(test)]
    pub(crate) fn for_test(path: PathBuf, first_cell: u32, last_cell: u32) -> Self {
        let bytes = fs::read(&path).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        Self { entry: GaiaEntry { name: path.file_name().unwrap().to_str().unwrap().into(), checksum: Md5::digest(bytes).into(), first_cell, last_cell },
            path, size: metadata.len(), modified: metadata.modified().unwrap() }
    }
    pub fn is_unchanged(&self) -> io::Result<bool> {
        let metadata = fs::metadata(&self.path)?;
        Ok(metadata.len() == self.size && metadata.modified()? == self.modified)
    }
}

pub(crate) fn resolve_gaia_files(count: u16, directories: &DatasetDirectories, notices: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Vec<GaiaFile>> {
    if count > GAIA_DR3_FILE_COUNT { return Err(invalid("--dataset-files must be between 0 and 3386")); }
    if count == 0 { return Ok(Vec::new()); }
    let entries = parse_manifest(MANIFEST, GAIA_DR3_FILE_COUNT as usize)?;
    let directory = directories.data.as_ref().ok_or_else(|| invalid("Cannot find the per-user data directory for Gaia files"))?.join("gaia-gdr3/gaia_source");
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(1800)))
        .timeout_connect(Some(Duration::from_secs(30))).timeout_recv_response(Some(Duration::from_secs(30))).build().new_agent();
    resolve_entries(&entries[..count as usize], &directory, notices, times, |entry| {
        let response = agent.get(format!("{SOURCE_URL}{}", entry.name)).header("Accept-Encoding", "identity").call().map_err(io::Error::other)?;
        if response.status() != 200 { return Err(invalid("expected a complete HTTP 200 Gaia file")); }
        let length = response.headers().get("Content-Length").map(|value| {
            value.to_str().map_err(io::Error::other)?.parse::<u64>().map_err(io::Error::other)
        }).transpose()?;
        Ok((Box::new(response.into_body().into_reader()), length))
    })
}

fn resolve_entries(entries: &[GaiaEntry], directory: &Path, notices: &mut dyn DatasetProgress, times: &mut StepTimes,
    mut fetch: impl FnMut(&GaiaEntry) -> io::Result<(Box<dyn Read>, Option<u64>)>) -> io::Result<Vec<GaiaFile>> {
    fs::create_dir_all(directory)?;
    writeln!(notices, "Gaia DR3: verifying {} selected files (~230 MB each if downloading); files cover only their sky regions.", entries.len())?;
    let mut files = Vec::with_capacity(entries.len());
    let mut reused = 0;
    notices.set_files(0, entries.len() as u64);
    for (index, entry) in entries.iter().enumerate() {
        let path = directory.join(&entry.name);
        let result = (|| {
            let (size, modified) = if let Some(stamp) = times.measure("Gaia file verification", || verify_file(&path, entry.checksum, notices))? {
                reused += 1;
                stamp
            } else {
                times.measure("Gaia file download", || {
                    notices.begin_stage(Stage::Connect, &entry.name, Unit::Work, None); // animate while connecting and waiting for response headers
                    let (reader, length) = fetch(entry)?;
                    install_verified(reader, &path, entry.checksum, length, notices)
                })?
            };
            let file = GaiaFile { entry: entry.clone(), path: path.clone(), size, modified };
            if !file.is_unchanged()? { return Err(invalid("source changed after verification")); }
            Ok(file)
        })();
        files.push(result.map_err(|error: io::Error| io::Error::new(error.kind(), format!("Gaia file {}/{} ({}): {error}. A verified manual copy can be placed at {}. Completed files remain reusable.", index + 1, entries.len(), entry.name, path.display())))?);
        notices.set_files((index + 1) as u64, entries.len() as u64);
    }
    writeln!(notices, "Gaia DR3: requested={} reused={} downloaded={}", entries.len(), reused, entries.len() - reused)?;
    times.describe("Dataset resolution", || format!("Gaia files requested={}; reused={reused}; downloaded={}", entries.len(), entries.len() - reused));
    Ok(files)
}

fn verify_file(path: &Path, expected: [u8; 16], progress: &mut dyn DatasetProgress) -> io::Result<Option<(u64, SystemTime)>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let before = file.metadata()?;
    if !before.is_file() { return Err(invalid("Gaia source is not a regular file")); }
    progress.begin_stage(Stage::Verify, &path.file_name().unwrap_or_default().to_string_lossy(), Unit::Bytes, Some(before.len()));
    let mut completed = 0;
    let mut hash = Md5::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 { break; }
        hash.update(&buffer[..read]);
        completed += read as u64;
        progress.set_position(completed);
    }
    let after = file.metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? { return Err(invalid("source changed during verification")); }
    let valid = <[u8; 16]>::from(hash.finalize()) == expected;
    progress.finish_stage(if valid { Outcome::Complete } else { Outcome::Failed });
    Ok(valid.then_some((after.len(), after.modified()?)))
}

fn install_verified(mut reader: impl Read, path: &Path, expected: [u8; 16], length: Option<u64>, notices: &mut dyn DatasetProgress) -> io::Result<(u64, SystemTime)> {
    if length.is_some_and(|length| length > GAIA_DOWNLOAD_MAX_BYTES) { return Err(invalid("Gaia download exceeds the per-file limit")); }
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().ok_or_else(|| invalid("missing parent directory"))?)?;
    let mut hash = Md5::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 64 * 1024];
    notices.begin_stage(Stage::Download, &path.file_name().unwrap_or_default().to_string_lossy(), Unit::Bytes, length);
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 { break; }
        bytes += read as u64;
        if bytes > GAIA_DOWNLOAD_MAX_BYTES || length.is_some_and(|length| bytes > length) { return Err(invalid("Gaia download larger than expected")); }
        hash.update(&buffer[..read]);
        temporary.write_all(&buffer[..read])?;
        notices.set_position(bytes);
    }
    if length.is_some_and(|length| bytes != length) || <[u8; 16]>::from(hash.finalize()) != expected {
        return Err(invalid("Gaia download size or MD5 mismatch"));
    }
    notices.begin_stage(Stage::Install, "Gaia", Unit::Work, None);
    temporary.as_file().sync_all()?;
    if let Some(stamp) = verify_file(path, expected, notices)? { return Ok(stamp); } // another invocation may already have installed the same valid file
    let metadata = temporary.as_file().metadata()?;
    let stamp = (metadata.len(), metadata.modified()?);
    temporary.persist(path).map_err(|error| error.error)?;
    notices.finish_stage(Outcome::Complete);
    Ok(stamp)
}

fn parse_manifest(text: &str, expected_count: usize) -> io::Result<Vec<GaiaEntry>> {
    let mut entries = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() != 2 { return Err(invalid("invalid Gaia manifest line")); }
        let hash = parts[0];
        if hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(invalid("invalid Gaia MD5")); }
        if parts[1] == "_MD5SUM.txt" { continue; } // publisher's checksum list also lists itself
        let name = parts[1];
        let range = name.strip_prefix("GaiaSource_").and_then(|s| s.strip_suffix(".csv.gz")).ok_or_else(|| invalid("unexpected Gaia manifest filename"))?;
        let (first, last) = range.split_once('-').ok_or_else(|| invalid("invalid Gaia sky range"))?;
        if [first, last].iter().any(|s| s.len() != 6 || !s.bytes().all(|b| b.is_ascii_digit())) { return Err(invalid("invalid Gaia sky range")); }
        let first_cell = first.parse().map_err(io::Error::other)?;
        let last_cell = last.parse().map_err(io::Error::other)?;
        if first_cell > last_cell || last_cell >= 786432 { return Err(invalid("Gaia sky range out of bounds")); }
        let mut checksum = [0; 16];
        for (i, byte) in checksum.iter_mut().enumerate() { *byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16).map_err(io::Error::other)?; }
        entries.push(GaiaEntry { name: name.into(), checksum, first_cell, last_cell });
    }
    entries.sort_unstable_by_key(|entry| (entry.first_cell, entry.last_cell));
    if entries.len() != expected_count || entries.windows(2).any(|p| p[0].last_cell >= p[1].first_cell) {
        return Err(invalid("Gaia manifest count or disjoint sky ranges mismatch"));
    }
    Ok(entries)
}
fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }

#[cfg(test)]
mod tests;
