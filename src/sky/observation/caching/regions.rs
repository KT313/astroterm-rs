//! Regional flags and correction records feed observed output directly; no combined selection lists.
use super::*;
use crate::state::StellarResults;
use crate::model::{CorrectionStats, ObservedRegion, SelectedStar};

/// Cache drawing eligibility within each requested region, aligned with its selected rows.
pub(in crate::sky::observation) fn update_regional_brightness(storage: &mut ObservationCache, stars: StellarResults<'_>, threshold: f64, output: &mut ObservedSky, times: &mut StepTimes) {
    let regional_before = storage.region_stats[0];
    times.measure_steps("Current brightness", |times| {
        let reuse = storage.config.allows(Group::StellarVisibility);                       // one policy lookup, not one per region
        let mut stale = false;
        times.measure("Brightness region decisions", || {
            for &region in stars.selection.regions() {
                let key = (stars.selection.region_generation(region), stars.region_generation(region), threshold);
                let entry = &mut storage.regions[region].eligible;
                let before = entry.stats;
                stale |= entry.needs_refresh(&key, stars.selection.epoch, None, reuse);
                add_region_stats(&mut storage.region_stats[0], before, entry.stats);
            }
        });
        if stale { times.measure("Regional brightness calculation", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region].eligible;
                if !entry.has_been_invalidated { continue; }
                let rows = borrow_region_rows(stars, region);
                let samples = stars.region_samples(region);
                let start = stars.selection.catalog.grid.offsets[region];
                let flags = rows.iter().map(|row| row.drawable && samples[row.source_index - start].magnitude <= threshold).collect();
                let key = (stars.selection.region_generation(region), stars.region_generation(region), threshold);
                let before = entry.stats;
                entry.store(key, stars.selection.epoch, 0.0, flags);
                add_region_stats(&mut storage.region_stats[0], before, entry.stats);
            }
        }); }                                                                               // on hits no region is stale; skip the second pass over all regions
        output.magnitude_threshold = threshold;
    });
    times.record_regional_counts(BufferId::RegionalVisibility, regional_before, storage.region_stats[0]);
}

/// Keep drawable stars and exclusive constellation endpoints, with stable catalog references in each slot.
pub(in crate::sky::observation) fn update_regional_corrections(storage: &mut ObservationCache, stars: StellarResults<'_>, output: &mut ObservedSky, times: &mut StepTimes) {
    let regional_before = storage.region_stats[1];
    times.measure_steps("Correction selection", |times| {
        let reuse = storage.config.allows(Group::StellarVisibility);
        let mut stale = false;
        times.measure("Correction cache decision", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region];
                let key = (stars.selection.region_generation(region), entry.eligible.generation);
                let before = entry.corrections.stats;
                stale |= entry.corrections.needs_refresh(&key, stars.selection.epoch, None, reuse);
                add_region_stats(&mut storage.region_stats[1], before, entry.corrections.stats);
            }
        });
        if stale { times.measure("Regional correction selection", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region];
                if !entry.corrections.has_been_invalidated { continue; }
                let rows = borrow_region_rows(stars, region);
                let endpoints = region == crate::constants::CONSTELLATION_REGION;
                let mut stats = CorrectionStats { evaluated: rows.len(), ..Default::default() };
                let records = rows.iter().zip(entry.eligible.value()).filter_map(|(row, &drawable)| {
                    if drawable || endpoints {
                        stats.endpoint_only += usize::from(!drawable);
                        Some(SelectedStar { source_index: row.source_index, drawable })
                    } else {
                        stats.skipped += 1;
                        None
                    }
                }).collect();
                let key = (stars.selection.region_generation(region), entry.eligible.generation);
                let before = entry.corrections.stats;
                entry.corrections.store(key, stars.selection.epoch, 0.0, (records, stats));
                add_region_stats(&mut storage.region_stats[1], before, entry.corrections.stats);
            }
        }); }
        prepare_observed_layout(storage, stars, times);
        output.corrections = storage.layout_stats;
    });
    times.record_regional_counts(BufferId::RegionalCorrections, regional_before, storage.region_stats[1]);
}

/// Reuse stellar aberration independently per region; solar-system vectors keep their own small cache.
pub(in crate::sky::observation) fn update_regional_aberration(storage: &mut ObservationCache, stars: StellarResults<'_>, observer: &ObserverState, times: &mut StepTimes) {
    let regional_before = storage.region_stats[2];
    times.measure_steps("Aberration", |times| {
        let epoch = observer.time.tt;
        let velocity = observer.state.velocity;
        let mut refresh_regions = false;
        let reuse = storage.config.allows(Group::ApparentDirections);
        times.measure("Apparent region decisions", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region];
                let key = (entry.corrections.generation, stars.region_generation(region), velocity);
                let before = entry.apparent.stats;
                refresh_regions |= entry.apparent.needs_refresh(&key, epoch, None, reuse);
                add_region_stats(&mut storage.region_stats[2], before, entry.apparent.stats);
            }
        });
        if refresh_regions { times.measure("Aberration calculation", || {
            let beta = velocity * (1.0 / crate::astro::LIGHT_SPEED_AU_DAY);
            for descriptor in &mut storage.regional_output {
                let region = descriptor.region;
                let entry = &mut storage.regions[region];
                if entry.apparent.has_been_invalidated {
                    let samples = stars.region_samples(region);
                    let start = stars.selection.catalog.grid.offsets[region];
                    let rows = &entry.corrections.value().0;
                    let key = (entry.corrections.generation, stars.region_generation(region), velocity);
                    let before = entry.apparent.stats;
                    entry.apparent.store_in_place(key, epoch, 0.0, |apparent| crate::cache::rewrite_in_place(apparent, rows.iter().map(|row|
                        super::super::apply_unit_aberration(samples[row.source_index - start].direction, beta)))); // keep each region's allocation
                    add_region_stats(&mut storage.region_stats[2], before, entry.apparent.stats);
                }
            }
        }); }
        for descriptor in &mut storage.regional_output { descriptor.apparent_generation = storage.regions[descriptor.region].apparent.generation; }
        let body_before = times.inspect_memory(|| snapshot_cache(&storage.body_apparent));
        times.measure("Body aberration", || {
            let body_key = (storage.relative.generation, velocity);
            storage.body_apparent.get_or_update(body_key, epoch, storage.config.allows(Group::ApparentDirections), || {
                let relative = storage.relative.value();
                (relative.0.iter().map(|&position| super::super::apply_aberration(position, velocity)).collect(),
                    super::super::apply_aberration(relative.1, velocity))
            });
        });
        record_cache(times, BufferId::BodyApparentDirections, body_before, &storage.body_apparent);
    });
    times.record_regional_counts(BufferId::RegionalApparent, regional_before, storage.region_stats[2]);
}

fn borrow_region_rows(stars: StellarResults<'_>, region: usize) -> &[SelectedStar] {
    let rows = stars.selection.working.value();
    let offsets = &stars.selection.catalog.grid.offsets;
    let start = rows.partition_point(|row| row.source_index < offsets[region]);
    let end = rows.partition_point(|row| row.source_index < offsets[region + 1]);
    &rows[start..end]
}

fn prepare_observed_layout(storage: &mut ObservationCache, stars: StellarResults<'_>, times: &mut StepTimes) {
    let before = times.inspect_memory(|| BufferShape::vector(&storage.layout_sources, IndexDomain::Regions));
    let changed = times.measure("Observed region layout", || {
        let sources = || stars.selection.regions().iter().map(|&region| (region, storage.regions[region].corrections.generation, stars.region_generation(region)));
        if sources().eq(storage.layout_sources.iter().copied()) { return false; }
        storage.layout_sources.clear();
        storage.layout_sources.extend(sources());
        storage.regional_output.clear();
        storage.layout_stats = CorrectionStats::default();
        let mut start = 0;
        for &(region, selection_generation, motion_generation) in &storage.layout_sources {
            let (rows, stats) = storage.regions[region].corrections.value();
            let end = start + rows.len();
            storage.layout_stats.evaluated += stats.evaluated;
            storage.layout_stats.skipped += stats.skipped;
            storage.layout_stats.endpoint_only += stats.endpoint_only;
            storage.regional_output.push(ObservedRegion { region, start, end, selection_generation, motion_generation, apparent_generation: 0 });
            start = end;
        }
        true
    }); // compare regional versions on hits; no selected-star scan or combined record construction
    times.record_shape(BufferId::ObservedLayout, if changed { Operation::Build } else { Operation::Reuse }, before, || BufferShape::vector(&storage.layout_sources, IndexDomain::Regions));
}

fn add_region_stats(total: &mut crate::cache::CacheStats, before: crate::cache::CacheStats, after: crate::cache::CacheStats) {
    total.hits += after.hits - before.hits;
    total.refreshes += after.refreshes - before.refreshes;
    total.bypasses += after.bypasses - before.bypasses;
    if after.hits == before.hits { total.last_reason = after.last_reason; }
}
