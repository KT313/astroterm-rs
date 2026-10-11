//! Assemble only the requested physical prefix; cached numerical records are not recomputed here.
use super::conversion::{self, ConvertedRow};
use super::super::super::preparation::{create_sort_pool, sort_source_ids};
use crate::{catalog::{MastSelection, BlobStore, MastRow, StarNames, StarId}, model::{StarStorage, PreparedCatalog},
    progress::{DatasetProgress, Stage, Unit, Outcome}, timing::StepTimes};
use std::{io, collections::HashMap};

pub(super) fn assemble_selected_catalog(selection: &mut MastSelection, store: &BlobStore, policy: &str, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let mut stars = StarStorage::gaia();
    let names = StarNames::absent();
    let mut bounds = Vec::new();
    let mut ids = Vec::new();
    let mut previous: Option<MastRow> = None;
    let (mut selected, mut duplicates, mut missing) = (0u64, 0u64, 0u64);
    let mut stats = [0u64; 5];

    let planned = times.measure("MAST assembly capacity", || reserve_assembly(selection, &mut stars, &mut bounds, &mut ids))?;
    #[cfg(test)]
    let allocations = tests::allocations(&stars, &bounds, &ids);
    progress.begin_stage(Stage::Convert, "Loading MAST blocks", Unit::Blocks, Some(selection.blocks.len() as u64));
    progress.set_position(0);                                                                    // even cached conversions still need loading and assembly
    for (completed, block) in selection.blocks.iter_mut().enumerate() {
        let converted = conversion::load_converted_block(&selection.store, block, store, policy, progress, times)?;
        let repeats = compare_boundary(previous.as_ref(), &converted.first)?;                     // a duplicate run can start in the preceding file or group
        progress.set_detail("Assembling selected rows and source IDs");

        times.measure("MAST row and ID assembly", || -> io::Result<()> {
            for (ordinal, entry) in converted.entries.into_iter().take(block.selected).enumerate() {
                selected += 1;
                if (ordinal == 0 && repeats) || matches!(entry.result, ConvertedRow::Duplicate) { duplicates += 1; continue; }
                ids.push(entry.id);                                                                  // keep missing-field representatives in the uniqueness check too
                match entry.result {
                    ConvertedRow::Ready { mut row, bound, clipping, stats: counts } => {
                        row.id = StarId::try_from_index(block.base + ordinal as u64).map_err(io::Error::other)?.0;
                        stars.push_prepared(row, clipping);
                        stars.push_source_id(entry.id);                                             // preserve Gaia identity alongside its retained row
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
            Ok(())
        })?;
        previous = Some(converted.last);
        progress.set_position(completed as u64 + 1);                                             // advance only after all selected rows in this block were accepted
    }

    if selected != planned as u64 { return Err(io::Error::new(io::ErrorKind::InvalidData, "MAST selected row count changed during assembly")); }
    #[cfg(test)]
    assert_eq!(tests::allocations(&stars, &bounds, &ids), allocations, "MAST assembly must not grow or replace reserved output buffers");
    progress.finish_stage(Outcome::Complete);
    progress.begin_stage(Stage::CheckIds, "", Unit::Work, None);
    progress.set_detail("Sorting source IDs");
    let pool = times.measure("Startup sort workers", || create_sort_pool(ids.len()))?;
    times.describe("Startup sort workers", || format!("worker threads={}; parallel sort threshold={} rows", pool.as_ref().map_or(0, rayon::ThreadPool::current_num_threads), crate::constants::STARTUP_PARALLEL_SORT_ROWS));
    times.measure("Gaia source-ID sort", || sort_source_ids(&mut ids, pool.as_ref()));
    progress.set_work(Unit::Rows, Some(ids.len() as u64));
    progress.set_detail("Checking for duplicate source IDs");
    times.measure("Gaia source-ID uniqueness", || -> io::Result<()> {
        for (index, pair) in ids.windows(2).enumerate() {
            if pair[0] == pair[1] { return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected nonadjacent Gaia duplicate {}", pair[0]))); }
            if (index + 1).is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS as usize) { progress.set_position((index + 1) as u64); }
        }
        progress.set_position(ids.len() as u64);
        Ok(())
    })?;
    progress.set_detail("Releasing temporary IDs and finishing loaded storage");
    times.measure("Gaia source-ID scratch release", || drop(ids));
    times.measure("MAST storage finalization", || -> io::Result<()> {
        if bounds.len() != stars.len() || stars.source_ids().unwrap().len() != stars.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "MAST assembled column lengths differ"));
        }
        Ok(())                                                                                // keep skipped-row capacity instead of copying columns to compact them
    })?;
    times.describe("MAST storage finalization", || format!("planned raw rows={planned}; retained stars={}; source-ID capacity={}; bounds capacity={}; unused capacity retained", stars.len(), stars.owned_source_ids().unwrap().capacity(), bounds.capacity()));
    progress.finish_stage(Outcome::Complete);
    writeln!(progress, "MAST Gaia DR3: selected raw={selected}; identical duplicate copies removed={duplicates}; missing RA/Dec/G={missing}; loaded={}; missing motion={}; angular-only={}; assumed RV0={}; missing color={}; saturated color={}.", stars.len(), stats[0], stats[1], stats[2], stats[3], stats[4])?;
    times.describe("Gaia loading", || format!("selected raw={selected}; duplicate copies={duplicates}; missing required={missing}; loaded={}", stars.len()));
    times.measure_steps("Catalog preparation", |times| super::super::super::pipeline::finish_catalog_with_pool(stars, bounds, names, &HashMap::new(), &[], progress, times, pool.as_ref()))
}

fn compare_boundary(previous: Option<&MastRow>, first: &MastRow) -> io::Result<bool> {
    let repeats = previous.is_some_and(|row| row.source_id == first.source_id);
    if repeats && previous != Some(first) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("conflicting Gaia duplicate at block boundary: {}", first.source_id)));
    }
    Ok(repeats)
}

fn reserve_assembly(selection: &MastSelection, stars: &mut StarStorage, bounds: &mut Vec<f32>, ids: &mut Vec<u64>) -> io::Result<usize> {
    let count = count_selected_rows(selection)?;                                                // raw rows include skips and duplicates, so this is an upper bound
    stars.try_reserve_exact(count)?;
    bounds.try_reserve_exact(count).map_err(io::Error::other)?;
    ids.try_reserve_exact(count).map_err(io::Error::other)?;
    Ok(count)
}

/// Check descriptor arithmetic before reserving output. Overfetched suffix rows are never included.
fn count_selected_rows(selection: &MastSelection) -> io::Result<usize> {
    let mut count = 0usize;
    for block in &selection.blocks {
        if block.selected > block.rows { return Err(io::Error::new(io::ErrorKind::InvalidData, "MAST selection exceeds block row count")); }
        if block.selected > 0 {
            let last = block.base.checked_add(u64::try_from(block.selected - 1).map_err(io::Error::other)?)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MAST ordinal overflow"))?;
            StarId::try_from_index(last).map_err(io::Error::other)?;
        }
        count = count.checked_add(block.selected).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MAST selected row count overflow"))?;
    }
    let count_u64 = u64::try_from(count).map_err(io::Error::other)?;
    crate::catalog::Catalog::check_star_count(count_u64).map_err(io::Error::other)?;
    if count_u64 > u64::from(selection.requested) { return Err(io::Error::new(io::ErrorKind::InvalidData, "MAST selection exceeds requested rows")); }
    std::alloc::Layout::array::<[f32; 3]>(count).map_err(io::Error::other)?;                         // all reserved columns and scratch entries fit within this largest element size
    Ok(count)
}

#[cfg(test)]
mod tests;
