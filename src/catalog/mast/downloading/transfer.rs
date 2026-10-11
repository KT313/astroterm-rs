//! Workers only fetch compressed columns; decoding and publication stay on the calling thread.
use super::{Job, Footer, parquet};
use crate::{catalog::mast::{storage::{BlobStore, decode, invalid}, transport::{Fetch, TransferControl}}, constants::MAST_BLOCK_SECONDS};
use bytes::Bytes;
use std::{io, time::{Instant, Duration}};

pub(super) fn download_ranges(store: &BlobStore, job: &Job, fetch: &impl Fetch, control: &TransferControl, admitted: Instant) -> io::Result<parquet::Ranges> {
    control.check()?;
    let footer: Footer = decode(&store.read_object(&job.footer_hash)?)?;
    let mut input = parquet::Ranges {
        size: footer.identity.size,
        parts: vec![(footer.identity.size - footer.bytes.len() as u64, Bytes::from(footer.bytes))],
    };
    let deadline = admitted + Duration::from_secs(MAST_BLOCK_SECONDS);
    let fetched = std::thread::scope(|scope| {
        let handles: Vec<_> = job.ranges.iter().map(|&(start, length)| {
            let identity = &footer.identity;
            scope.spawn(move || {
                let range = format!("bytes={start}-{}", start + length as u64 - 1);
                let response = fetch.fetch_cancellable(&job.path, Some(&range), Some(identity), length, deadline, control)?;
                if response.start != start || response.bytes.len() != length { return Err(invalid("invalid selected range length")); }
                Ok((start, Bytes::from(response.bytes)))
            })
        }).collect();
        let mut result = Ok(Vec::with_capacity(handles.len()));
        for handle in handles {
            let part = handle.join().unwrap_or_else(|_| Err(io::Error::other("range worker panicked")));
            match (&mut result, part) {
                (Ok(parts), Ok(part)) => parts.push(part),
                (Ok(_), Err(error)) => result = Err(error),
                (Err(_), _) => {},                                                         // join every worker while preserving the first error
            }
        }
        result
    })?;
    input.parts.extend(fetched);
    Ok(input)
}

pub(super) fn normalize_block(input: parquet::Ranges, job: &Job) -> io::Result<Vec<u8>> {
    let footer = input.parts.first().ok_or_else(|| invalid("missing downloaded footer"))?;
    let metadata = parquet::parse_footer(&footer.1)?;
    let schema = parquet::projected_schema(&metadata)?;
    let rows = parquet::read_group(input, job.group)?;
    if rows.len() != job.rows { return Err(invalid("MAST row count changed during download")); }
    parquet::write_block(&rows, schema)
}
