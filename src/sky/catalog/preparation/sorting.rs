//! Exact catalog ordering and one optional startup-only CPU pool. Rendering never uses this pool.
use crate::{catalog::{StarId, cache::invalid}, model::StarStorage, time::StepTimes,
    constants::{STARTUP_SORT_THREADS, STARTUP_PARALLEL_SORT_ROWS, SIMULATION_REGION_COUNT, CONSTELLATION_REGION},
    progress::{DatasetProgress, Stage, Unit, Outcome}};
use super::super::grid;
use rayon::{prelude::*, ThreadPool, ThreadPoolBuilder};
use std::{collections::HashSet, io, mem::size_of, cmp::Ordering};

#[derive(Clone, Copy, Debug)]
struct SortRecord { key: u64, original_index: u32 }

pub(in crate::sky::catalog) fn create_sort_pool(rows: usize) -> io::Result<Option<ThreadPool>> {
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    let threads = choose_threads(available);
    if rows < STARTUP_PARALLEL_SORT_ROWS || threads == 1 { return Ok(None); }
    ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("catalog-sort-{i}"))
        .build().map(Some).map_err(io::Error::other)
}

fn choose_threads(available: usize) -> usize { available.clamp(1, STARTUP_SORT_THREADS.max(1)) }

pub(in crate::sky::catalog) fn sort_source_ids(ids: &mut [u64], pool: Option<&ThreadPool>) {
    sort_values(ids, pool, STARTUP_PARALLEL_SORT_ROWS, u64::cmp);
}

fn sort_values<T: Send>(values: &mut [T], pool: Option<&ThreadPool>, threshold: usize, compare: impl Fn(&T, &T) -> Ordering + Sync) {
    if let Some(pool) = pool.filter(|_| values.len() >= threshold) {
        pool.install(|| values.par_sort_unstable_by(&compare));
    } else { values.sort_unstable_by(compare); }
}

pub(in crate::sky::catalog) fn sort_stars_by_region_and_brightness(stars: &mut StarStorage, bounds: &mut Vec<f32>, endpoints: &HashSet<StarId>, progress: &mut dyn DatasetProgress, times: &mut StepTimes, pool: Option<&ThreadPool>) -> io::Result<Vec<usize>> {
    progress.set_work(Unit::Stars, Some(stars.len() as u64));
    progress.set_detail("Calculating star regions and sort keys");
    times.measure("Dataset loading.Catalog preparation.Region assignment");
    let (mut records, counts) = build_records(stars, endpoints, progress)?;
    times.describe("Dataset loading.Catalog preparation.Region assignment", || format!("sort records={}; record bytes={}; allocated payload bytes={}", records.len(), size_of::<SortRecord>(), records.capacity() * size_of::<SortRecord>()));
    progress.finish_stage(Outcome::Complete);

    progress.begin_stage(Stage::Sort, "", Unit::Work, None);
    times.measure("Dataset loading.Catalog preparation.Region and brightness sort");
    sort_values(&mut records, pool, STARTUP_PARALLEL_SORT_ROWS, |a, b| a.key.cmp(&b.key));
    progress.finish_stage(Outcome::Complete);

    progress.begin_stage(Stage::Reorder, "Field values copied across nine columns", Unit::Elements, Some(stars.len() as u64 * 9));
    let order = records.iter().map(|record| usize::try_from(record.original_index).expect("u32 index fits supported targets"));
    times.measure("Dataset loading.Catalog preparation.Star field permutation");
    stars.gather_ordered(order, bounds, progress)?;
    times.describe("Dataset loading.Catalog preparation.Star field permutation", || format!("largest replacement column bytes={}; one column replaced at a time", largest_replacement_bytes(stars, bounds)));
    times.measure("Dataset loading.Catalog preparation.Sort scratch release");
    drop(records);
    progress.finish_stage(Outcome::Complete);
    Ok(counts)
}

fn build_records(stars: &StarStorage, endpoints: &HashSet<StarId>, progress: &mut dyn DatasetProgress) -> io::Result<(Vec<SortRecord>, Vec<usize>)> {
    crate::catalog::Catalog::check_star_count(stars.len() as u64).map_err(io::Error::other)?;
    let mut records = Vec::new();
    records.try_reserve_exact(stars.len()).map_err(io::Error::other)?;
    let mut counts = vec![0usize; SIMULATION_REGION_COUNT + 1];
    let columns = stars.columns();
    for i in 0..stars.len() {
        let region = if endpoints.contains(&stars.id(i)) { CONSTELLATION_REGION } else { grid::stored_cell(stars, i) };
        records.push(build_record(region, columns.brightness_key[i], columns.id[i], i)?);
        counts[region + 1] += 1;
        if (i + 1).is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS as usize) { progress.set_position((i + 1) as u64); }
    }
    progress.set_position(stars.len() as u64);
    Ok((records, counts))
}

fn largest_replacement_bytes(stars: &StarStorage, bounds: &Vec<f32>) -> usize {
    let columns = stars.owned_columns();
    [columns.u0.capacity() * size_of::<[f32; 3]>(), columns.w.capacity() * size_of::<[f32; 3]>(),
        columns.magnitude.capacity() * size_of::<u16>(), columns.brightness_key.capacity() * size_of::<u16>(),
        columns.distance.capacity() * size_of::<f32>(), columns.id.capacity() * size_of::<u32>(),
        columns.display_color.capacity(), stars.label_capacity(), bounds.capacity() * size_of::<f32>()]
        .into_iter().max().unwrap_or(0)
}

fn build_record(region: usize, brightness: u16, id: u32, index: usize) -> io::Result<SortRecord> {
    let region = u16::try_from(region).map_err(|_| invalid("catalog region exceeds sort-key range"))?;
    let original_index = u32::try_from(index).map_err(|_| invalid("catalog row exceeds u32 index range"))?;
    let key = (u64::from(region) << 48) | (u64::from(brightness) << 32) | u64::from(!id);
    Ok(SortRecord { key, original_index })
}

#[cfg(test)]
mod tests;
