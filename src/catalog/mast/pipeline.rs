//! Resolve one stable physical prefix; completed groups remain independently reusable when N changes.
use super::{storage::{BlobStore, digest, encode, decode, invalid}, transport::{Fetch, Http, Identity}, parquet::{self, Ranges}};
use crate::{catalog::datasets::DatasetDirectories, constants::*, progress::{DatasetProgress, Stage, Unit, Outcome}, timing::StepTimes};
use serde::{Serialize, Deserialize};
use std::{io, time::{Instant, Duration}};
use bytes::Bytes;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Leaf { order: u8, pixel: u64 }
impl Leaf { fn path(&self) -> String { format!("dataset/Norder={}/Dir={}/Npix={}.parquet",self.order,self.pixel/10000*10000,self.pixel) } }
#[derive(Serialize, Deserialize)]
struct Discovery { version: u8, properties: String, partitions: String }
#[derive(Serialize, Deserialize)]
struct Footer { identity: Identity, bytes: Vec<u8> }
#[derive(Clone, Debug)]
pub(crate) struct MastBlock { pub key: String, pub hash: String, pub rows: usize, pub selected: usize, pub base: u64 }
pub(crate) struct MastSelection { pub store: BlobStore, pub blocks: Vec<MastBlock>, pub discovery: String, pub requested: u32, pub available: u64 }
impl MastSelection {
    pub(crate) fn read(&self, block: &MastBlock) -> io::Result<Vec<u8>> {
        self.store.read_object(&block.hash)
    }
}
pub(crate) fn resolve_mast(count: u32, directories: &DatasetDirectories, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<MastSelection> {
    let root = directories.data.as_ref().ok_or_else(|| invalid("Cannot find the data directory for MAST"))?.join("gaia-gdr3/mast-parquet-v1");
    resolve_selection(count, BlobStore::new(root), &Http::new(), progress, times)
}
fn resolve_selection(count: u32, store: BlobStore, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<MastSelection> {
    progress.begin_stage(Stage::Resolve, "MAST partition list", Unit::Work, None);
    let discovery = load_discovery(&store, fetch, progress)?;                                    // reuse the fixed list of sky partitions
    let leaves = parse_leaves(&discovery.partitions)?;
    let key = digest(&encode(&discovery)?);
    let mut selection = MastSelection { store, blocks: Vec::new(), discovery: key, requested: count, available: 0 };
    let (mut reused, mut downloaded) = (0, 0);

    for leaf in leaves {
        if selection.available >= u64::from(count) { break; }
        let path = leaf.path();
        let footer = times.measure("MAST file metadata", || load_footer(&selection.store, &selection.discovery, &path, fetch, progress))?;
        let metadata = parquet::parse_footer(&footer.bytes)?;
        for group in 0..metadata.num_row_groups() {
            if selection.available >= u64::from(count) { break; }
            let rows = usize::try_from(metadata.row_group(group).num_rows()).map_err(io::Error::other)?;
            if rows == 0 { continue; }
            parquet::selected_ranges(&metadata, group, footer.identity.size)?;                   // check the whole group's byte budget before fetching any column
            let raw_key = format!("raw-v1:{}:{}:{}:{}:{}", selection.discovery, path, footer.identity.etag, footer.identity.size, group);
            let (raw, was_reused) = load_raw_block(&selection.store, &raw_key, &path, &footer, group, fetch, progress, times)?;
            validate_raw_block(&raw, rows)?;
            if was_reused { reused += 1; } else { downloaded += 1; }
            let selected = rows.min((u64::from(count) - selection.available) as usize);
            selection.blocks.push(MastBlock { key: raw_key, hash: digest(&raw), rows, selected, base: selection.available });
            selection.available = selection.available.checked_add(rows as u64).ok_or_else(|| invalid("MAST row count overflow"))?;
        }
    }

    if selection.available < u64::from(count) {
        writeln!(progress, "Gaia DR3: requested {count} raw rows; catalog contains only {}. Loading available rows.", selection.available)?;
    }
    writeln!(progress, "MAST Gaia DR3: raw requested={count}; locally covered={}; blocks reused={reused}; downloaded={downloaded}.", selection.available)?;
    Ok(selection)
}

#[allow(clippy::too_many_arguments)]
fn load_raw_block(store: &BlobStore, key: &str, path: &str, footer: &Footer, group: usize, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<(Vec<u8>, bool)> {
    if let Some(bytes) = recoverable_read(store, key, progress)? { return Ok((bytes, true)); }
    let raw = times.measure("MAST column download", || download_block(path, footer, group, fetch, progress))?;
    Ok((store.publish(key, &raw, false)?, false))
}

fn validate_raw_block(raw: &[u8], rows: usize) -> io::Result<()> {
    let metadata = parquet::parse_footer(raw)?;                                                  // metadata checks do not decompress cached star columns
    parquet::projected_schema(&metadata)?;
    if metadata.num_row_groups() != 1 || metadata.row_group(0).num_rows() != rows as i64 {
        return Err(invalid("invalid cached MAST block row count"));
    }
    Ok(())
}

fn recoverable_read(store: &BlobStore,key: &str,progress:&mut dyn DatasetProgress)->io::Result<Option<Vec<u8>>> {
    match store.read(key) {Ok(value)=>Ok(value),Err(e)=>{writeln!(progress,"Ignoring invalid MAST block: {e}; fetching that block again.")?;Ok(None)}}
}
fn load_discovery(store:&BlobStore,fetch:&impl Fetch,progress:&mut dyn DatasetProgress)->io::Result<Discovery> {
    if let Some(bytes)=store.read("selected-discovery-v1")? {let d:Discovery=decode(&bytes)?;validate_discovery(&d)?;return Ok(d);}
    progress.begin_stage(Stage::Connect,"MAST catalog discovery",Unit::Work,None);
    let deadline=Instant::now()+Duration::from_secs(MAST_BLOCK_SECONDS);
    let properties=fetch.fetch("properties",None,None,64*1024,deadline)?.bytes;
    let partitions=fetch.fetch("partition_info.csv",None,None,1024*1024,deadline)?.bytes;
    if fetch.fetch("properties",None,None,64*1024,deadline)?.bytes!=properties {return Err(invalid("MAST catalog changed during discovery"));}
    let d=Discovery {version:1,properties:String::from_utf8(properties).map_err(io::Error::other)?,partitions:String::from_utf8(partitions).map_err(io::Error::other)?};
    validate_discovery(&d)?;
    decode(&store.publish("selected-discovery-v1",&encode(&d)?,true)?)
}
fn validate_discovery(d:&Discovery)->io::Result<()> {
    if d.version!=1 || !d.properties.lines().any(|s|s=="obs_collection=gaia") || !d.properties.lines().any(|s|s=="dataproduct_type=object") {return Err(invalid("unexpected MAST catalog identity"));}
    parse_leaves(&d.partitions)?;Ok(())
}
fn parse_leaves(text:&str)->io::Result<Vec<Leaf>> {
    let mut csv=csv::Reader::from_reader(text.as_bytes());
    if csv.headers().map_err(io::Error::other)?.iter().collect::<Vec<_>>()!=["Norder","Npix"] {return Err(invalid("invalid MAST partition columns"));}
    let mut leaves=Vec::new();
    for record in csv.records() {
        let r=record.map_err(io::Error::other)?;
        let order:u8=r[0].parse().map_err(io::Error::other)?;
        let pixel:u64=r[1].parse().map_err(io::Error::other)?;
        if order>29 || pixel>=12_u64.checked_shl(2*u32::from(order)).ok_or_else(||invalid("invalid HEALPix order"))? {return Err(invalid("invalid HEALPix leaf"));}
        leaves.push(Leaf{order,pixel});
    }
    let max=leaves.iter().map(|l|l.order).max().ok_or_else(||invalid("empty MAST partition list"))?;
    leaves.sort_by_key(|l|l.pixel << (2*(max-l.order)));
    let mut end=0;
    for leaf in &leaves {let width=1_u64 << (2*(max-leaf.order));let start=leaf.pixel*width;if start<end{return Err(invalid("overlapping MAST leaves"));}end=start+width;}
    Ok(leaves)
}
fn load_footer(store:&BlobStore,discovery:&str,path:&str,fetch:&impl Fetch,progress:&mut dyn DatasetProgress)->io::Result<Footer> {
    let key=format!("footer-v1:{discovery}:{path}");
    if let Some(bytes)=store.read(&key)? {let f:Footer=decode(&bytes)?;parquet::parse_footer(&f.bytes)?;return Ok(f);}
    progress.begin_stage(Stage::Connect,path,Unit::Work,None);
    let deadline=Instant::now()+Duration::from_secs(MAST_BLOCK_SECONDS);
    let tail=fetch.fetch(path,Some(&format!("bytes=-{MAST_FOOTER_TAIL_BYTES}")),None,MAST_FOOTER_TAIL_BYTES as usize,deadline)?;
    let id=tail.identity.ok_or_else(||invalid("missing footer identity"))?;
    if tail.bytes.len()<8 || !tail.bytes.ends_with(b"PAR1") {return Err(invalid("invalid Parquet magic"));}
    let n=u32::from_le_bytes(tail.bytes[tail.bytes.len()-8..tail.bytes.len()-4].try_into().unwrap()) as usize;
    if n>MAST_FOOTER_MAX_BYTES || n as u64+8>id.size {return Err(invalid("oversized Parquet footer"));}
    let bytes=if n+8<=tail.bytes.len() {tail.bytes[tail.bytes.len()-8-n..].to_vec()} else {
        fetch.fetch(path,Some(&format!("bytes={}-{}",id.size-n as u64-8,id.size-1)),Some(&id),n+8,deadline)?.bytes
    };
    parquet::parse_footer(&bytes)?;
    let footer=Footer{identity:id,bytes};
    decode(&store.publish(&key,&encode(&footer)?,true)?)
}
fn download_block(path:&str,footer:&Footer,group:usize,fetch:&impl Fetch,progress:&mut dyn DatasetProgress)->io::Result<Vec<u8>> {
    let metadata=parquet::parse_footer(&footer.bytes)?;
    let ranges=parquet::selected_ranges(&metadata,group,footer.identity.size)?;
    let total=ranges.iter().map(|(_,n)|*n as u64).sum();
    progress.begin_stage(Stage::Download,&format!("{path} group {group}"),Unit::Bytes,Some(total));
    let deadline=Instant::now()+Duration::from_secs(MAST_BLOCK_SECONDS);
    let mut input=Ranges{size:footer.identity.size,parts:vec![(footer.identity.size-footer.bytes.len() as u64,Bytes::copy_from_slice(&footer.bytes))]};
    let mut completed=0;
    for batch in ranges.chunks(MAST_RANGE_REQUESTS.max(1)) {
        let fetched=std::thread::scope(|scope| {
            let handles:Vec<_>=batch.iter().map(|&(start,n)|scope.spawn(move|| {
                let r=fetch.fetch(path,Some(&format!("bytes={start}-{}",start+n as u64-1)),Some(&footer.identity),n,deadline)?;
                if r.start!=start || r.bytes.len()!=n {return Err(invalid("invalid selected range length"));}
                Ok::<_,io::Error>((start,Bytes::from(r.bytes)))
            })).collect();
            handles.into_iter().map(|h|h.join().map_err(|_|io::Error::other("range worker panicked"))?).collect::<io::Result<Vec<_>>>()
        })?;
        for part in fetched {completed+=part.1.len() as u64;input.parts.push(part);}
        progress.set_position(completed);
    }
    progress.begin_stage(Stage::Parse,"MAST selected columns",Unit::Rows,Some(metadata.row_group(group).num_rows() as u64));
    let rows=parquet::read_group(input,group)?;
    let raw=parquet::write_block(&rows,parquet::projected_schema(&metadata)?)?;
    progress.set_position(rows.len() as u64);progress.finish_stage(Outcome::Complete);
    Ok(raw)
}

#[cfg(test)]
mod tests;
