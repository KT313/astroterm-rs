//! Region decisions never inspect individual samples. Numerical work runs only for requested stale regions.
use crate::constants::{CONSTELLATION_REGION, STELLAR_BATCH_SIZE};
use crate::{cache::{Group, RefreshReason}, state::StellarMotionBuffers, model::{StellarFields, StellarWork},
    timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}, astro::models::stars::years_since_j2000};

pub(super) fn check_requested_regions(storage: &mut StellarMotionBuffers<'_>, epoch: f64, times: &mut StepTimes) {
    let ttl = storage.config.age_seconds(Group::StellarState);
    let reuse = storage.config.allows(Group::StellarState);
    let mut reasons = [0_usize; 5];
    let mut hits = 0;
    let mut empty = 0;
    times.measure("Stellar region decisions", || {
        storage.refresh_regions.clear();
        for &region in storage.requested_regions {
            empty += usize::from(storage.offsets[region] == storage.offsets[region + 1]);
            let entry = &mut storage.regions.entries[region];
            if entry.needs_refresh(&(), epoch, Some(ttl), reuse) {
                storage.refresh_regions.push(region);
                let reason = entry.stats.last_reason.expect("refresh has a reason");
                let index = match reason { RefreshReason::Missing => 0, RefreshReason::Invalidated => 1,
                    RefreshReason::Dependencies => 2, RefreshReason::Expired => 3, RefreshReason::Bypassed => 4 };
                reasons[index] += 1;
                storage.stats.last_reason = Some(reason);
                storage.stats.bypasses += u64::from(reason == RefreshReason::Bypassed);
            } else { hits += 1; }
        }
        storage.stats.hits += hits as u64;
    });
    times.record_borrow(BufferId::RegionSelection, Access::ReadOnly, || BufferShape::slice(storage.requested_regions, IndexDomain::Regions));
    times.record_build(BufferId::StellarRefreshRegions, || BufferShape::vector(storage.refresh_regions, IndexDomain::Regions));
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarSamples, Operation::Reuse, None, None, Some(hits), None));
    for (count, reason) in reasons.into_iter().zip([RefreshReason::Missing, RefreshReason::Invalidated, RefreshReason::Dependencies, RefreshReason::Expired, RefreshReason::Bypassed]) {
        if count != 0 {
            times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarSamples, Operation::Refresh(reason), None, None, Some(count), None));
        }
    }
    times.describe("Stellar region decisions", || format!("requested spatial regions={}; constellation requested={}; empty regions={empty}; region hits={hits}; refreshes={}; missing={}; invalidated={}; dependencies={}; expired={}; bypassed={}; effective TTL={ttl} simulated seconds; no per-star cache checks",
        storage.requested_regions.iter().filter(|&&id| id != CONSTELLATION_REGION).count(), storage.requested_regions.contains(&CONSTELLATION_REGION),
        storage.refresh_regions.len(), reasons[0], reasons[1], reasons[2], reasons[3], reasons[4]));
}

pub(super) fn refresh_regions(storage: &mut StellarMotionBuffers<'_>, catalog: StellarFields<'_>, epoch: f64, times: &mut StepTimes) {
    if storage.refresh_regions.is_empty() { return; }
    let years = years_since_j2000(epoch);
    let ttl = storage.config.age_seconds(Group::StellarState);
    let mut simulated = 0;
    let before = times.inspect_memory(|| BufferShape::vector(storage.scratch, IndexDomain::Catalog));
    times.measure("Stellar scratch preparation", || {
        storage.scratch.clear();
        storage.scratch.reserve(STELLAR_BATCH_SIZE);
    });
    times.record_shape(BufferId::StellarScratch, Operation::Reserve, before, || BufferShape::vector(storage.scratch, IndexDomain::Catalog));

    times.measure_batches("Stellar batches", |times| {
        for &region in storage.refresh_regions.iter() {
            let (start, end) = (storage.offsets[region], storage.offsets[region + 1]);
            let mut samples = times.measure("Region output allocation", || Vec::with_capacity(end - start));
            times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarSamples, Operation::Reserve, None,
                Some(BufferShape::vector(&samples, IndexDomain::Catalog)), Some(end - start), None));
            for batch_start in (start..end).step_by(STELLAR_BATCH_SIZE) {
                let batch_end = (batch_start + STELLAR_BATCH_SIZE).min(end);
                times.measure("Trajectory reads", || {
                    storage.scratch.clear();
                    for index in batch_start..batch_end {
                        let motion = catalog.motion(index);
                        let class = storage.prepared_classes.map_or_else(|| motion.classify(), |classes| classes[index]);
                        storage.scratch.push(StellarWork { magnitude: catalog.magnitude(index), motion, class, sample: None });
                    }
                });
                times.record_borrow(BufferId::CatalogTrajectories, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
                times.record_build(BufferId::StellarScratch, || BufferShape::vector(storage.scratch, IndexDomain::Catalog));
                times.measure("Motion and magnitude calculation", || {
                    for item in storage.scratch.iter_mut() {
                        item.sample = Some(item.motion.evaluate_classified(years, item.magnitude, item.class));
                    }
                });
                times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Write, None, None, Some(batch_end - batch_start), None));
                times.measure("Region sample assembly", || samples.extend(storage.scratch.iter().map(|item| item.sample.unwrap())));
                times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarSamples, Operation::Append, None, None,
                    Some(batch_end - batch_start), (batch_end - batch_start).checked_mul(std::mem::size_of::<crate::astro::models::stars::StellarSample>())));
            }
            simulated += samples.len();
            let outcome = times.measure("Stellar region stores", || {
                let outcome = storage.regions.entries[region].store((), epoch, ttl, samples);
                if outcome.value_changed { *storage.generation = storage.generation.wrapping_add(1); }
                storage.stats.refreshes += 1;
                outcome
            });
            times.record_store(BufferId::StellarSamples, outcome);
        }
    });
    times.describe("Stellar batches", || format!("refreshed regions={}; newly simulated stars={simulated}; complete region ranges, including faint stars; batch limit={STELLAR_BATCH_SIZE}; fixed TTL={ttl} simulated seconds; no per-star validity qualification", storage.refresh_regions.len()));
}
