//! Regional validity checks followed by bounded numerical passes. Catalog inputs are read-only.
mod regions;
#[cfg(test)] mod tests;
use crate::{state::StellarMotionBuffers, model::StellarFields, timing::{StepTimes, BufferId, BufferShape, IndexDomain, Operation}};

pub(crate) fn update_stellar_motion(mut storage: StellarMotionBuffers<'_>, catalog: StellarFields<'_>, epoch: f64, times: &mut StepTimes) {
    regions::check_requested_regions(&mut storage, epoch, times); // inspect one timestamp/flag per requested region
    regions::refresh_regions(&mut storage, catalog, epoch, times); // calculate every row of expired regions

    let before = times.inspect_memory(|| BufferShape::vector(storage.scratch, IndexDomain::Catalog));
    times.measure("Stellar scratch clear", || storage.scratch.clear());
    times.record_shape(BufferId::StellarScratch, Operation::Clear, before, || BufferShape::vector(storage.scratch, IndexDomain::Catalog));
    let before = times.inspect_memory(|| BufferShape::vector(storage.refresh_regions, IndexDomain::Regions));
    times.measure("Stellar refresh list clear", || storage.refresh_regions.clear());
    times.record_shape(BufferId::StellarRefreshRegions, Operation::Clear, before, || BufferShape::vector(storage.refresh_regions, IndexDomain::Regions));
}
