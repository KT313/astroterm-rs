//! Assemble only the requested physical prefix; cached numerical records are not recomputed here.
use super::conversion::{self, ConvertedRow};
use crate::{catalog::{MastSelection, BlobStore, MastRow, StarNames, StarId}, model::{StarStorage, PreparedCatalog},
    progress::{DatasetProgress, Stage, Unit, Outcome}, timing::StepTimes};
use std::{io, collections::HashMap};

pub(super) fn assemble_selected_catalog(selection: &MastSelection, store: &BlobStore, keys: &[String], progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let mut stars = StarStorage::default();
    let mut names = StarNames::default();
    let mut bounds = Vec::new();
    let mut ids = Vec::new();
    let mut previous: Option<MastRow> = None;
    let (mut selected, mut duplicates, mut missing) = (0u64, 0u64, 0u64);
    let mut stats = [0u64; 5];

    progress.begin_stage(Stage::Convert, "Loading MAST blocks", Unit::Blocks, Some(selection.blocks.len() as u64));
    progress.set_position(0);                                                                    // even cached conversions still need loading and assembly
    for (completed, (block, key)) in selection.blocks.iter().zip(keys).enumerate() {
        let converted = conversion::load_converted_block(selection, block, store, key, progress, times)?;
        let repeats = compare_boundary(previous.as_ref(), &converted.first)?;                     // a duplicate run can start in the preceding file or group
        stars.try_reserve(block.selected)?;
        bounds.try_reserve(block.selected).map_err(io::Error::other)?;
        ids.try_reserve(block.selected).map_err(io::Error::other)?;

        for (ordinal, entry) in converted.entries.into_iter().take(block.selected).enumerate() {
            selected += 1;
            if (ordinal == 0 && repeats) || matches!(entry.result, ConvertedRow::Duplicate) { duplicates += 1; continue; }
            ids.push(entry.id);                                                                  // keep missing-field representatives in the uniqueness check too
            match entry.result {
                ConvertedRow::Ready { mut row, bound, clipping, label, stats: counts } => {
                    row.id = StarId::try_from_index(block.base + ordinal as u64).map_err(io::Error::other)?.0;
                    row.name = names.insert(&label)?.entry();                                    // copy supplied text and rebase its reference; do not regenerate it
                    crate::catalog::Catalog::check_star_count(stars.len() as u64 + 1).map_err(io::Error::other)?;
                    stars.push_prepared(row, clipping);
                    bounds.push(bound);
                    for (bit, total) in stats.iter_mut().enumerate() { *total += u64::from(counts & (1 << bit) != 0); }
                },
                ConvertedRow::Missing => missing += 1,
                ConvertedRow::Error { message, unsupported } => {
                    let kind = if unsupported { io::ErrorKind::Unsupported } else { io::ErrorKind::InvalidData };
                    return Err(io::Error::new(kind, format!("Gaia DR3 {} (raw row {}): {message}", entry.id, block.base + ordinal as u64)));
                },
                ConvertedRow::Duplicate => unreachable!(),
            }
        }
        previous = Some(converted.last);
        progress.set_position(completed as u64 + 1);                                             // advance only after all selected rows in this block were accepted
    }

    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|p| p[0] == p[1]) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected nonadjacent Gaia duplicate {}", pair[0])));
    }
    drop(ids);                                                                                   // the exact-ID check is startup-only working memory
    writeln!(progress, "MAST Gaia DR3: selected raw={selected}; identical duplicate copies removed={duplicates}; missing RA/Dec/G={missing}; loaded={}; missing motion={}; angular-only={}; assumed RV0={}; missing color={}; saturated color={}.", stars.len(), stats[0], stats[1], stats[2], stats[3], stats[4])?;
    times.describe("Gaia loading", || format!("selected raw={selected}; duplicate copies={duplicates}; missing required={missing}; loaded={}", stars.len()));
    stars.shrink_to_fit();
    bounds.shrink_to_fit();
    progress.finish_stage(Outcome::Complete);
    times.measure("Catalog preparation", || super::super::super::pipeline::finish_catalog(stars, bounds, names, &HashMap::new(), &[], progress))
}

fn compare_boundary(previous: Option<&MastRow>, first: &MastRow) -> io::Result<bool> {
    let repeats = previous.is_some_and(|row| row.source_id == first.source_id);
    if repeats && previous != Some(first) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("conflicting Gaia duplicate at block boundary: {}", first.source_id)));
    }
    Ok(repeats)
}
