//! Block-local results preserve physical ordinals through missing fields, duplicates and deferred errors.
use crate::{catalog::{MastRow, MastBlock, MastSelection, BlobStore, encode_mast, decode_mast, mast_digest, read_mast_block, validate_gaia_input, StarId},
    model::{StarRow, StarStorage}, progress::DatasetProgress, timing::StepTimes};
use super::super::conversion::{convert_source, ConversionStats};
use serde::{Serialize, Deserialize};
use std::io;

#[derive(Serialize, Deserialize)]
pub(super) struct ConvertedBlock {
    pub version: u8,
    pub first: MastRow,                      // original inputs needed to compare across block boundaries
    pub last: MastRow,
    pub entries: Vec<ConvertedEntry>,        // one status per physical row, including skipped rows
}
#[derive(Serialize, Deserialize)]
pub(super) struct ConvertedEntry { pub id: u64, pub result: ConvertedRow }
#[derive(Serialize, Deserialize)]
pub(super) enum ConvertedRow {
    Ready { row: StarRow, bound: f32, clipping: [u64; 2], label: String, stats: u8 },
    Missing,
    Duplicate,
    Error { message: String, unsupported: bool },
}

pub(super) fn fingerprint() -> String {
    let mut bytes = super::super::super::cache::catalog_fingerprint().to_vec();
    bytes.extend_from_slice(&crate::constants::GAIA_DISTANCE_MIN_SIGNAL.to_le_bytes());
    for source in [include_bytes!("conversion.rs").as_slice(), include_bytes!("assembly.rs").as_slice(), include_bytes!("mod.rs").as_slice(),
        include_bytes!("../conversion.rs").as_slice(), include_bytes!("../../../../catalog/mast/parquet/mod.rs").as_slice()] {
        bytes.extend_from_slice(source);
    }
    mast_digest(&bytes)
}
pub(super) fn block_key(block: &MastBlock, policy: &str) -> String { format!("converted-v1:{}:{}:{policy}", block.key, block.hash) }

pub(super) fn load_converted_block(selection: &MastSelection, block: &MastBlock, store: &BlobStore, key: &str, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<ConvertedBlock> {
    match store.read(key).and_then(|bytes| bytes.map(|b| decode_mast::<ConvertedBlock>(&b)).transpose()) {
        Ok(Some(data)) if validate_block(&data, block.rows).is_ok() => {
            times.describe("Gaia loading", || format!("converted block reused: {} rows; no source decode or conversion", block.rows));
            return Ok(data);
        },
        Ok(None) => {},
        Ok(Some(_)) => writeln!(progress, "Invalid converted MAST block; rebuilding from local Parquet.")?,
        Err(error) => writeln!(progress, "Ignoring converted MAST cache: {error}; rebuilding locally.")?,
    }

    let rows = times.measure("MAST Parquet decode", || read_mast_block(selection.read(block)?))?;     // only decode source columns when conversion results are missing
    let data = times.measure("MAST block conversion", || convert_block(&rows))?;
    validate_block(&data, block.rows)?;
    store.publish(key, &encode_mast(&data)?, false)?;                                              // publish the complete block before claiming it can be reused
    Ok(data)
}

fn convert_block(rows: &[MastRow]) -> io::Result<ConvertedBlock> {
    let first = rows.first().ok_or_else(|| io::Error::other("empty converted block"))?.clone();
    let last = rows.last().unwrap().clone();
    let mut entries = Vec::new();
    entries.try_reserve(rows.len()).map_err(io::Error::other)?;

    for (ordinal, input) in rows.iter().enumerate() {
        let previous = ordinal.checked_sub(1).map(|i| &rows[i]);
        let result = prepare_row_status(input, ordinal, previous);
        entries.push(ConvertedEntry { id: input.source_id, result });
    }
    Ok(ConvertedBlock { version: 2, first, last, entries })
}

fn prepare_row_status(input: &MastRow, ordinal: usize, previous: Option<&MastRow>) -> ConvertedRow {
    if previous.is_some_and(|row| row.source_id == input.source_id) {
        if previous == Some(input) { return ConvertedRow::Duplicate; }                           // compare stored bits and null flags, including NaNs and signed zero
        return ConvertedRow::Error { message: "conflicting adjacent Gaia source_id".into(), unsupported: false };
    }
    match prepare_entry(input, ordinal) {
        Ok(row) => row,
        Err(error) => ConvertedRow::Error { message: error.to_string(), unsupported: error.kind() == io::ErrorKind::Unsupported },
    }
}

fn prepare_entry(input: &MastRow, ordinal: usize) -> io::Result<ConvertedRow> {
    let id = StarId::try_from_index(ordinal as u64).map_err(io::Error::other)?;
    let Some(source) = validate_gaia_input(id, input.source_id, input.numbers())? else { return Ok(ConvertedRow::Missing); };
    let label = input.designation.as_ref().filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Gaia designation"))?.clone();
    let mut stats = ConversionStats::default();
    let star = convert_source(source, &mut stats)?;
    let (row, bound, clipping) = StarStorage::prepare_row(star)?;                                   // qualify and encode once; assembly later appends these exact bytes
    Ok(ConvertedRow::Ready { row, bound, clipping, label,
        stats: pack_stats(&stats) })
}

fn validate_block(data: &ConvertedBlock, count: usize) -> io::Result<()> {
    if data.version != 2 || data.entries.len() != count || count == 0
        || data.entries[0].id != data.first.source_id || data.entries[count - 1].id != data.last.source_id {
        return Err(io::Error::other("invalid converted MAST shape"));
    }
    for (i, entry) in data.entries.iter().enumerate() {
        match &entry.result {
            ConvertedRow::Duplicate if i == 0 || data.entries[i - 1].id != entry.id => return Err(io::Error::other("invalid duplicate marker")),
            ConvertedRow::Ready { row, bound, label, stats, .. } if *stats & !0x1f != 0 || !is_valid_record(row, *bound, label, i) => {
                return Err(io::Error::other("invalid converted MAST record"));
            },
            _ => {},
        }
    }
    Ok(())
}

/// Five per-source outcomes occupy bits 0..4 in the same order as the printed totals.
fn pack_stats(stats: &ConversionStats) -> u8 {
    [stats.missing_motion, stats.angular_only, stats.missing_rv, stats.missing_color, stats.saturated_color]
        .into_iter().enumerate().fold(0, |flags, (bit, count)| flags | (u8::from(count != 0) << bit))
}

fn is_valid_record(row: &StarRow, bound: f32, label: &str, ordinal: usize) -> bool {
    row.u0.iter().chain(&row.w).all(|v| v.is_finite()) && bound.is_finite() && bound >= 0.0
        && row.distance.is_finite() && row.distance >= 0.0 && !label.is_empty() && row.name == 0
        && row.id == ordinal as u32 && crate::model::StarColor::from_index(row.display_color).is_some()
}

#[cfg(test)]
mod tests;
