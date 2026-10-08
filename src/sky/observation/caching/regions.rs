//! Region-local corrections retain catalog indices; frame assembly alone uses transient working-row indices.
use super::*;
use crate::state::StellarResults;
use crate::model::{CorrectionStats, ObservedRegion, SelectedStar};

/// Cache one brightness decision for each requested region, then expose the existing working-order flags.
pub(in crate::sky::observation) fn update_regional_brightness(storage: &mut ObservationCache, stars: StellarResults<'_>, threshold: f64, output: &mut ObservedSky, times: &mut StepTimes) {
    let regional_before = storage.region_stats[0];
    let before = times.inspect_memory(|| snapshot_cache(&storage.eligible));
    times.measure_steps("Current brightness", |times| {
        times.measure("Brightness region decisions", || {
            for &region in stars.selection.regions() {
                let key = (stars.selection.region_generation(region), stars.region_generation(region), threshold);
                let entry: &mut crate::state::EligibleCache = &mut storage.regions[region].eligible;
                let before = entry.stats;
                entry.needs_refresh(&key, stars.selection.epoch, None, storage.config.allows(Group::StellarVisibility));
                add_region_stats(&mut storage.region_stats[0], before, entry.stats);
            }
        });
        times.measure("Regional brightness calculation", || {
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
        });
        times.measure("Brightness output assembly", || {
            let motion: &crate::state::MotionCache = stars.motion;
            storage.eligible.get_or_update((stars.selection.working.generation, motion.generation, threshold), stars.selection.epoch,
                storage.config.allows(Group::StellarVisibility), || stars.selection.regions().iter()
                    .flat_map(|&region| storage.regions[region].eligible.value().iter().copied()).collect()); // rebuild flags only when the combined working set changes
        });
        output.magnitude_threshold = threshold;
    });
    record_cache(times, BufferId::VisibilityFlags, before, &storage.eligible);
    times.record_regional_counts(BufferId::RegionalVisibility, regional_before, storage.region_stats[0]);
}

/// Keep drawable stars and exclusive constellation endpoints, with stable catalog references in each slot.
pub(in crate::sky::observation) fn update_regional_corrections(storage: &mut ObservationCache, stars: StellarResults<'_>, output: &mut ObservedSky, times: &mut StepTimes) {
    let regional_before = storage.region_stats[1];
    times.measure_steps("Correction selection", |times| {
        times.measure("Correction cache decision", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region];
                let key = (stars.selection.region_generation(region), entry.eligible.generation);
                let before = entry.corrections.stats;
                entry.corrections.needs_refresh(&key, stars.selection.epoch, None, storage.config.allows(Group::StellarVisibility));
                add_region_stats(&mut storage.region_stats[1], before, entry.corrections.stats);
            }
        });
        times.measure("Regional correction selection", || {
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
        });
        let key = (stars.selection.working.generation, storage.eligible.generation);
        if storage.corrections.needs_refresh(&key, stars.selection.epoch, None, storage.config.allows(Group::StellarVisibility)) {
            let selected = times.measure("Correction index selection", || assemble_correction_indices(storage, stars));
            let outcome = times.measure("Correction cache store", || storage.corrections.store(key, stars.selection.epoch, 0.0, selected));
            times.record_store(BufferId::CorrectionSelection, outcome);
        }
        times.measure("Corrected-star buffer construction", || assemble_observed_regions(storage, stars, output));
        times.record_build(BufferId::ObservedStars, || BufferShape::vector(&output.stars, IndexDomain::Observed));
        times.describe("Corrected-star buffer construction", || format!("output records={}; stable regional catalog indices; requested regions={}", output.stars.len(), storage.regional_output.len()));
    });
    times.record_regional_counts(BufferId::RegionalCorrections, regional_before, storage.region_stats[1]);
}

/// Reuse stellar aberration independently per region; solar-system vectors keep their own small cache.
pub(in crate::sky::observation) fn update_regional_aberration(storage: &mut ObservationCache, stars: StellarResults<'_>, observer: &ObserverState, output: &mut ObservedSky, times: &mut StepTimes) {
    let regional_before = storage.region_stats[2];
    times.measure_steps("Aberration", |times| {
        let epoch = observer.time.tt;
        let velocity = observer.state.velocity;
        let mut refresh_regions = false;
        times.measure("Apparent region decisions", || {
            for &region in stars.selection.regions() {
                let entry = &mut storage.regions[region];
                let key = (entry.corrections.generation, stars.region_generation(region), velocity);
                let before = entry.apparent.stats;
                refresh_regions |= entry.apparent.needs_refresh(&key, epoch, None, storage.config.allows(Group::ApparentDirections));
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
                    let positions = entry.corrections.value().0.iter().map(|row|
                        super::super::apply_unit_aberration(samples[row.source_index - start].direction, beta)).collect();
                    let key = (entry.corrections.generation, stars.region_generation(region), velocity);
                    let before = entry.apparent.stats;
                    entry.apparent.store(key, epoch, 0.0, positions);
                    add_region_stats(&mut storage.region_stats[2], before, entry.apparent.stats);
                }
            }
        }); }
        for descriptor in &mut storage.regional_output { descriptor.apparent_generation = storage.regions[descriptor.region].apparent.generation; }
        times.measure("Body aberration", || {
            let body_key = (storage.relative.generation, velocity);
            storage.body_apparent.get_or_update(body_key, epoch, storage.config.allows(Group::ApparentDirections), || {
                let relative = storage.relative.value();
                (relative.0.iter().map(|&position| super::super::apply_aberration(position, velocity)).collect(),
                    super::super::apply_aberration(relative.1, velocity))
            });
        });
        let key = (stars.motion.generation, storage.relative.generation, storage.corrections.generation, velocity);
        let refresh = times.measure("Apparent cache decision", || storage.apparent.needs_refresh(&key, epoch, None, storage.config.allows(Group::ApparentDirections)));
        if refresh {
            let positions = times.measure("Direction capture", || assemble_apparent_directions(storage, output));
            record_direction_pass(times, output); // assembly publishes directions while capturing the current-frame snapshot
            times.describe("Direction capture", || "assemble regional directions into the frame and its snapshot in one pass".into());
            record_direction_capture(times, BufferId::ApparentDirections, &positions);
            let outcome = times.measure("Direction cache store", || storage.apparent.store(key, epoch, 0.0, positions));
            times.record_store(BufferId::ApparentDirections, outcome);
        } else {
            times.measure("Direction restoration", || super::corrections::restore_directions(output, storage.apparent.value()));
            record_direction_restoration(times, BufferId::ApparentDirections, output);
        }
    });
    times.record_regional_counts(BufferId::RegionalApparent, regional_before, storage.region_stats[2]);
}

fn assemble_apparent_directions(storage: &ObservationCache, output: &mut ObservedSky) -> crate::model::Directions {
    let mut positions = Vec::with_capacity(output.stars.len());
    for region in &storage.regional_output {
        let directions = storage.regions[region.region].apparent.value();
        for (star, &direction) in output.stars[region.start..region.end].iter_mut().zip(directions) {
            star.position = direction;
            positions.push(direction);
        }
    }
    let bodies = storage.body_apparent.value();
    for (planet, &direction) in output.planets.iter_mut().zip(&bodies.0) { planet.position = direction; }
    output.moon.position = bodies.1;
    (positions, bodies.0.clone(), bodies.1)
}

fn borrow_region_rows(stars: StellarResults<'_>, region: usize) -> &[SelectedStar] {
    let rows = stars.selection.working.value();
    let offsets = &stars.selection.catalog.grid.offsets;
    let start = rows.partition_point(|row| row.source_index < offsets[region]);
    let end = rows.partition_point(|row| row.source_index < offsets[region + 1]);
    &rows[start..end]
}

fn assemble_correction_indices(storage: &ObservationCache, stars: StellarResults<'_>) -> CorrectionSelection {
    let working = stars.selection.rows();
    let mut cursor = 0;
    let mut indices = Vec::with_capacity(working.len());
    let mut stats = CorrectionStats::default();
    for &region in stars.selection.regions() {
        let (records, region_stats) = storage.regions[region].corrections.value();
        stats.evaluated += region_stats.evaluated;
        stats.skipped += region_stats.skipped;
        stats.endpoint_only += region_stats.endpoint_only;
        for row in records {
            while working[cursor].source_index < row.source_index { cursor += 1; }
            indices.push(cursor);
            cursor += 1;
        }
    }
    CorrectionSelection { indices, stats }
}

fn assemble_observed_regions(storage: &mut ObservationCache, stars: StellarResults<'_>, output: &mut ObservedSky) {
    output.stars.clear();
    output.stars.reserve(storage.corrections.value().indices.len());
    output.corrections = storage.corrections.value().stats;
    storage.regional_output.clear();
    for &region in stars.selection.regions() {
        let start = output.stars.len();
        let samples = stars.region_samples(region);
        let offset = stars.selection.catalog.grid.offsets[region];
        let selection = &storage.regions[region].corrections;
        output.stars.extend(selection.value().0.iter().map(|row| {
            let sample = samples[row.source_index - offset];
            ObservedStar { source_index: row.source_index, drawable: row.drawable, position: sample.direction, magnitude: sample.magnitude }
        }));
        storage.regional_output.push(ObservedRegion { region, start, end: output.stars.len(), selection_generation: selection.generation,
            motion_generation: stars.region_generation(region), apparent_generation: 0 }); // filled when stellar aberration completes
    }
}

fn add_region_stats(total: &mut crate::cache::CacheStats, before: crate::cache::CacheStats, after: crate::cache::CacheStats) {
    total.hits += after.hits - before.hits;
    total.refreshes += after.refreshes - before.refreshes;
    total.bypasses += after.bypasses - before.bypasses;
    if after.hits == before.hits { total.last_reason = after.last_reason; }
}
