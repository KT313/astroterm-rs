//! Resolve one stable physical prefix; completed groups remain independently reusable when N changes.
use super::{storage::{BlobStore, BlobStamp, VerifiedBlob, digest, encode, decode, invalid}, transport::{Fetch, Http, Identity}, parquet};
use crate::{catalog::datasets::DatasetDirectories, constants::{MAST_BLOCK_SECONDS, MAST_FOOTER_TAIL_BYTES, MAST_FOOTER_MAX_BYTES}, progress::{DatasetProgress, Stage, Unit}, timing::StepTimes};
use serde::{Serialize, Deserialize};
use std::{io, time::{Instant, Duration}};
use super::downloading::{plan_blocks, download_missing};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Leaf { order: u8, pixel: u64 }
impl Leaf { pub(super) fn path(&self) -> String { format!("dataset/Norder={}/Dir={}/Npix={}.parquet",self.order, self.pixel / 10000 * 10000, self.pixel) } }
#[derive(Serialize, Deserialize)]
struct Discovery { version: u8, properties: String, partitions: String }
#[derive(Serialize, Deserialize)]
pub(super) struct Footer { pub identity: Identity, pub bytes: Vec<u8> }
#[derive(Clone, Debug)]
pub(crate) struct MastBlock {
    pub key: String, pub hash: String, pub rows: usize, pub selected: usize, pub base: u64, pub stamp: BlobStamp,
    pub source: Option<MastSource>,          // production blocks retain pinned provenance; synthetic converted-only fixtures may omit it
}
#[derive(Clone, Debug)]
pub(crate) struct MastSource { pub path: String, pub footer_hash: String, pub group: usize }
pub(crate) struct MastSelection { pub store: BlobStore, pub blocks: Vec<MastBlock>, pub discovery: String, pub requested: u32, pub available: u64 }
impl MastSelection {
    pub(crate) fn check_unchanged(&self) -> io::Result<()> {
        for block in &self.blocks { self.store.check_unchanged(&block.hash, block.stamp)?; }
        Ok(())
    }
}
/// The public catalog loader validates the block limit, including zero-row selections, before calling here.
pub(crate) fn resolve_mast(count: u32, parallel_blocks: u8, directories: &DatasetDirectories, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<MastSelection> {
    let root = directories.data.as_ref().ok_or_else(|| invalid("Cannot find the data directory for MAST"))?.join("gaia-gdr3/mast-parquet-v1");
    resolve_selection(count, parallel_blocks, BlobStore::new(root), &Http::new(), progress, times)
}
fn resolve_selection(count: u32, parallel_blocks: u8, store: BlobStore, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<MastSelection> {
    if count == 0 { return Ok(MastSelection { store, blocks: Vec::new(), discovery: String::new(), requested: 0, available: 0 }); }
    progress.begin_stage(Stage::Resolve, "MAST partition list", Unit::Work, None);
    let discovery = load_discovery(&store, fetch, progress)?;                                    // pin the list before planning the requested prefix
    let leaves = parse_leaves(&discovery.partitions)?;
    let key = digest(&encode(&discovery)?);
    let mut selection = MastSelection { store, blocks: Vec::new(), discovery: key, requested: count, available: 0 };
    let mut plan = plan_blocks(&mut selection, leaves, fetch, progress, times)?;                  // locate cached blocks without opening their star payloads
    let downloaded = plan.jobs.len();
    let reused = plan.ready.len() - downloaded;
    times.measure_steps("MAST block downloads", |times| download_missing(&selection.store, &mut plan, parallel_blocks, fetch, progress, times))?;
    selection.blocks = plan.ready.into_iter().map(|block| block.ok_or_else(|| invalid("missing MAST block result"))).collect::<io::Result<_>>()?;

    if selection.available < u64::from(count) {
        writeln!(progress, "Gaia DR3: requested {count} raw rows; catalog contains only {}. Loading available rows.", selection.available)?;
    }
    writeln!(progress, "MAST Gaia DR3: raw requested={count}; locally covered={}; blocks reused={reused}; downloaded={downloaded}.", selection.available)?;
    Ok(selection)
}

pub(super) fn validate_raw_block(raw: &[u8], rows: usize) -> io::Result<()> {
    let metadata = parquet::parse_footer(raw)?;                                                  // metadata checks do not decompress cached star columns
    parquet::projected_schema(&metadata)?;
    if metadata.num_row_groups() != 1 || metadata.row_group(0).num_rows() != rows as i64 {
        return Err(invalid("invalid cached MAST block row count"));
    }
    Ok(())
}

pub(super) fn recoverable_read(store: &BlobStore, key: &str, progress: &mut dyn DatasetProgress) -> io::Result<Option<VerifiedBlob>> {
    match store.read_verified(key, progress) {
        Ok(value) => Ok(value),
        Err(error) => {
            writeln!(progress, "Ignoring invalid cached MAST data: {error}; refetching the original input.")?;
            Ok(None)
        },
    }
}

fn load_discovery(store: &BlobStore, fetch: &impl Fetch, progress: &mut dyn DatasetProgress) -> io::Result<Discovery> {
    if let Some(blob) = recoverable_read(store, "selected-discovery-v1", progress)? {
        let discovery: Discovery = decode(&blob.bytes)?;
        validate_discovery(&discovery)?;
        return Ok(discovery);
    }
    progress.begin_stage(Stage::Connect, "MAST catalog discovery", Unit::Work, None);
    let deadline = Instant::now() + Duration::from_secs(MAST_BLOCK_SECONDS);
    let properties = fetch.fetch("properties", None, None, 64 * 1024, deadline)?.bytes;
    let partitions = fetch.fetch("partition_info.csv", None, None, 1024 * 1024, deadline)?.bytes;
    if fetch.fetch("properties", None, None, 64 * 1024, deadline)?.bytes != properties {
        return Err(invalid("MAST catalog changed during discovery"));
    }
    let discovery = Discovery {
        version: 1,
        properties: String::from_utf8(properties).map_err(io::Error::other)?,
        partitions: String::from_utf8(partitions).map_err(io::Error::other)?,
    };
    validate_discovery(&discovery)?;
    decode(&store.publish("selected-discovery-v1", &encode(&discovery)?, true)?.bytes)                // repair the original hash; never replace the pinned identity
}

fn validate_discovery(discovery: &Discovery) -> io::Result<()> {
    if discovery.version != 1 || !discovery.properties.lines().any(|s| s == "obs_collection=gaia")
        || !discovery.properties.lines().any(|s| s == "dataproduct_type=object") {
        return Err(invalid("unexpected MAST catalog identity"));
    }
    parse_leaves(&discovery.partitions)?;
    Ok(())
}

fn parse_leaves(text: &str) -> io::Result<Vec<Leaf>> {
    let mut csv = csv::Reader::from_reader(text.as_bytes());
    if csv.headers().map_err(io::Error::other)?.iter().collect::<Vec<_>>() != ["Norder", "Npix"] {
        return Err(invalid("invalid MAST partition columns"));
    }
    let mut leaves = Vec::new();
    for record in csv.records() {
        let record = record.map_err(io::Error::other)?;
        let order: u8 = record[0].parse().map_err(io::Error::other)?;
        let pixel: u64 = record[1].parse().map_err(io::Error::other)?;
        if order > 29 || pixel >= 12_u64.checked_shl(2 * u32::from(order)).ok_or_else(|| invalid("invalid HEALPix order"))? {
            return Err(invalid("invalid HEALPix leaf"));
        }
        leaves.push(Leaf { order, pixel });
    }
    let max = leaves.iter().map(|leaf| leaf.order).max().ok_or_else(|| invalid("empty MAST partition list"))?;
    leaves.sort_by_key(|leaf| leaf.pixel << (2 * (max - leaf.order)));
    let mut end = 0;
    for leaf in &leaves {
        let width = 1_u64 << (2 * (max - leaf.order));
        let start = leaf.pixel * width;
        if start < end { return Err(invalid("overlapping MAST leaves")); }
        end = start + width;
    }
    Ok(leaves)
}

pub(super) fn load_footer(store: &BlobStore, discovery: &str, path: &str, fetch: &impl Fetch, progress: &mut dyn DatasetProgress) -> io::Result<Footer> {
    if let Some(footer) = read_cached_footer(store, discovery, path, progress)? { return Ok(footer); }
    fetch_footer(store, discovery, path, fetch, progress)
}

pub(super) fn read_cached_footer(store: &BlobStore, discovery: &str, path: &str, progress: &mut dyn DatasetProgress) -> io::Result<Option<Footer>> {
    let key = format!("footer-v1:{discovery}:{path}");
    let Some(blob) = recoverable_read(store, &key, progress)? else { return Ok(None); };
    let footer: Footer = decode(&blob.bytes)?;
    parquet::parse_footer(&footer.bytes)?;
    Ok(Some(footer))
}

pub(super) fn fetch_footer(store: &BlobStore, discovery: &str, path: &str, fetch: &impl Fetch, progress: &mut dyn DatasetProgress) -> io::Result<Footer> {
    let key = format!("footer-v1:{discovery}:{path}");
    progress.set_detail(&format!("Fetching file metadata: {path}")); // retain the planning stage while fetching a missing footer
    let deadline = Instant::now() + Duration::from_secs(MAST_BLOCK_SECONDS);
    let tail = fetch.fetch(path, Some(&format!("bytes=-{MAST_FOOTER_TAIL_BYTES}")), None, MAST_FOOTER_TAIL_BYTES as usize, deadline)?;
    let identity = tail.identity.ok_or_else(|| invalid("missing footer identity"))?;
    if tail.bytes.len() < 8 || !tail.bytes.ends_with(b"PAR1") { return Err(invalid("invalid Parquet magic")); }
    let length = u32::from_le_bytes(tail.bytes[tail.bytes.len() - 8..tail.bytes.len() - 4].try_into().unwrap()) as usize;
    if length > MAST_FOOTER_MAX_BYTES || length as u64 + 8 > identity.size { return Err(invalid("oversized Parquet footer")); }
    let bytes = if length + 8 <= tail.bytes.len() {
        tail.bytes[tail.bytes.len() - 8 - length..].to_vec()                                        // most footers fit inside the initial tail request
    } else {
        fetch.fetch(path, Some(&format!("bytes={}-{}", identity.size - length as u64 - 8, identity.size - 1)), Some(&identity), length + 8, deadline)?.bytes
    };
    parquet::parse_footer(&bytes)?;
    let footer = Footer { identity, bytes };
    decode(&store.publish(&key, &encode(&footer)?, true)?.bytes)
}

#[cfg(test)]
mod tests;
