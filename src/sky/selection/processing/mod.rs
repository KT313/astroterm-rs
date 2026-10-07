//! Conservative brightness validation and ordered endpoint union.
use crate::model::{SkyCatalog, SelectedStar};
pub(crate) fn filter_brightness_candidates(
    catalog: &SkyCatalog,
    tt: f64,
    threshold: f64,
    candidates: Option<&[usize]>,
) -> Vec<usize> {
    let keys = catalog.stars.brightness_keys();
    if !crate::astro::COMPUTATIONAL_INTERVAL.contains(tt) {
        (0..catalog.stars.len()).collect()
    } else if let Some(indices) = candidates {
        indices
            .iter()
            .copied()
            .filter(|&i| i < keys.len() && crate::catalog::passes_brightness_bound(keys[i], threshold))
            .collect()
    } else {
        (0..catalog.stars.len())
            .filter(|&i| crate::catalog::passes_brightness_bound(keys[i], threshold))
            .collect()
    }
}

pub(crate) fn merge_constellation_endpoints(
    mut selected: Vec<usize>,
    endpoints: &[usize],
    times: &mut crate::timing::StepTimes,
) -> Vec<SelectedStar> {
    let input = selected.len();
    let before = times.inspect_memory(|| crate::timing::BufferShape::vector(&selected, crate::timing::IndexDomain::Catalog));
    times.measure("Candidate index sort and dedup", || {
        selected.sort_unstable();
        selected.dedup();
    });
    {
        use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
        if let Some(shape) = before { times.record_memory(times.last_memory_step(), || MemoryEvent::borrow(BufferId::ValidatedCandidates, Access::Writable, shape)); }
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::ValidatedCandidates, Operation::Write,
            before, Some(BufferShape::vector(&selected, IndexDomain::Catalog)), None, None)); // sort/dedup writes depend on comparisons
    }
    times.describe("Candidate index sort and dedup", || {
        format!(
            "input indices={input}; duplicates removed={}; output indices={}",
            input - selected.len(),
            selected.len()
        )
    });
    let input_shape = times.inspect_memory(|| crate::timing::BufferShape::vector(&selected, crate::timing::IndexDomain::Catalog));
    let working = times.measure("Endpoint index merge", || {
        let mut working = Vec::with_capacity(selected.len() + endpoints.len());
        let mut candidates = selected.into_iter().peekable();
        let mut endpoints = endpoints.iter().copied().peekable();
        while candidates.peek().is_some() || endpoints.peek().is_some() {
            let index = candidates
                .peek()
                .copied()
                .unwrap_or(usize::MAX)
                .min(endpoints.peek().copied().unwrap_or(usize::MAX));
            let drawable = candidates.peek() == Some(&index);
            if drawable {
                candidates.next();
            }
            if endpoints.peek() == Some(&index) {
                endpoints.next();
            }
            working.push(SelectedStar {
                source_index: index,
                drawable,
            });
        }
        working
    });
    {
        use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent};
        if let Some(shape) = input_shape { times.record_memory(times.last_memory_step(), || MemoryEvent::borrow(BufferId::ValidatedCandidates, Access::ReadOnly, shape)); }
        times.record_borrow(BufferId::CatalogEndpoints, Access::ReadOnly, || BufferShape::slice(endpoints, IndexDomain::Catalog));
        times.record_build(BufferId::WorkingStars, || BufferShape::vector(&working, IndexDomain::Working));
    }
    times.describe("Endpoint index merge", || {
        format!(
            "output selected indices/flags={}; element bytes={}; catalog metadata copied=0",
            working.len(),
            std::mem::size_of::<SelectedStar>()
        )
    });
    working
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_merge_preserves_source_order_and_membership_without_metadata() {
        let merged = merge_constellation_endpoints(vec![9, 1, 5, 1], &[0, 1, 7, 9, 12], &mut Default::default());
        assert_eq!(
            merged.iter().map(|s| (s.source_index, s.drawable)).collect::<Vec<_>>(),
            [(0, false), (1, true), (5, true), (7, false), (9, true), (12, false)]
        );
        assert!(std::mem::size_of::<SelectedStar>() <= 2 * std::mem::size_of::<usize>());
        assert_eq!(
            merge_constellation_endpoints(vec![], &[], &mut Default::default()),
            vec![]
        );
    }
}
