//! Compact comparison inputs for exact current-magnitude draw order. Sorting never follows observed-star
//! references; both the cached pipeline and stateless projection use the same comparison semantics.
# [cfg (test)] use crate::model::ProjectedStar;
use crate::catalog::StarId;
use std::cmp::Ordering;
use crate::timing::{BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};

use crate::model::DrawRecord;
fn compare_for_drawing(record: &DrawRecord, other: &DrawRecord) -> Ordering {
    if record.magnitude == other.magnitude {
        record.id.cmp(&other.id) // includes +0.0 and -0.0; preserve the original equality check
    } else {
        other.magnitude.total_cmp(&record.magnitude)
    }
}

pub(super) fn prepare_draw_order(records: &mut Vec<DrawRecord>, stars: impl IntoIterator<Item = (f64, StarId)>) {
    prepare_draw_order_with_times(records, stars, &mut crate::timing::StepTimes::default());
}

pub(in crate::projection) fn prepare_draw_order_with_times(
    records: &mut Vec<DrawRecord>,
    stars: impl IntoIterator<Item = (f64, StarId)>,
    times: &mut crate::timing::StepTimes,
) {
    let before = times.inspect_memory(|| BufferShape::vector(records, IndexDomain::Visible));
    times.measure("Sort record construction", || {
        records.clear(); // retained capacity is scratch space, never a cached result
        records.extend(
            stars
                .into_iter()
                .enumerate()
                .map(|(projected_index, (magnitude, id))| DrawRecord {
                    magnitude,
                    id,
                    projected_index,
                }),
        );
    });
    {
        let step = times.last_memory_step();
        times.record_memory(step, || MemoryEvent::operation(BufferId::DrawOrderScratch, Operation::Clear, before, before.map(|mut shape| { shape.len = Some(0); shape }), before.and_then(|shape| shape.len), None));
        times.record_memory(step, || {
            let after = BufferShape::vector(records, IndexDomain::Visible);
            MemoryEvent::operation(BufferId::DrawOrderScratch, Operation::Build, before.map(|mut shape| { shape.len = Some(0); shape }), Some(after), after.len, after.logical_bytes())
        });
    }
    times.measure("Magnitude and ID sort", || records.sort_unstable_by(compare_for_drawing));
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::DrawOrderScratch, Operation::Write, Some(BufferShape::vector(records, IndexDomain::Visible)), Some(BufferShape::vector(records, IndexDomain::DrawOrder)), None, None)); // sorting access counts are not measured

}

#[cfg(test)]
pub(super) fn sort_stars_for_drawing(stars: &mut [ProjectedStar<'_>]) {
    let mut records = Vec::new();
    prepare_draw_order(
        &mut records,
        stars.iter().map(|star| (star.star.magnitude, star.star.id())),
    );

    // Apply each permutation cycle once, without allocating another array of projected stars.
    for start in 0..records.len() {
        let mut current = start;
        while records[current].projected_index != start {
            let next = records[current].projected_index;
            stars.swap(current, next);
            records[current].projected_index = current;
            current = next;
        }
        records[current].projected_index = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_preserves_signed_zero_infinities_and_nan_semantics() {
        let values = [
            f64::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            1.0,
            f64::INFINITY,
            -f64::NAN,
            f64::NAN,
        ];
        for a in values {
            for b in values {
                for (a_id, b_id) in [(1, 2), (2, 1)] {
                    let expected = if a == b { a_id.cmp(&b_id) } else { b.total_cmp(&a) };
                    let record = |magnitude, id| DrawRecord {
                        magnitude,
                        id: StarId(id),
                        projected_index: 0,
                    };
                    assert_eq!(compare_for_drawing(&record(a, a_id), &record(b, b_id)), expected);
                }
            }
        }
    }

    #[test]
    fn compact_sort_and_permutation_match_reference_with_ties_and_reused_scratch() {
        let catalog = crate::catalog::load_embedded_catalog().unwrap();
        let mut sky = crate::sky::create_sky_from_catalog(&catalog);
        sky.stars.truncate(128);
        let mut scratch = Vec::new();
        for count in [128, 3, 0, 17, 128] {
            for (index, star) in sky.stars.iter_mut().enumerate() {
                star.source_index = index * 73 % 128; // permuted unique IDs, independent of source order
                star.magnitude = ((index * 19 + count) % 7) as f64;
                if index % 7 == 0 {
                    star.magnitude = -0.0;
                }
            }
            let mut actual: Vec<_> = sky.stars[..count]
                .iter()
                .enumerate()
                .map(|(index, star)| ProjectedStar {
                    star: crate::model::ObservedStarView {
                        state: star,
                        catalog: &sky.catalog.stars,
                    },
                    cell: Some((index as i32, 1)),
                })
                .collect();
            let mut expected = actual.clone();
            expected.sort_unstable_by(|a, b| {
                if a.star.magnitude == b.star.magnitude {
                    a.star.id().cmp(&b.star.id())
                } else {
                    b.star.magnitude.total_cmp(&a.star.magnitude)
                }
            });
            prepare_draw_order(&mut scratch, actual.iter().map(|s| (s.star.magnitude, s.star.id())));
            assert_eq!(
                scratch
                    .iter()
                    .map(|r| actual[r.projected_index].clone())
                    .collect::<Vec<_>>(),
                expected
            );
            sort_stars_for_drawing(&mut actual);
            assert_eq!(actual, expected);
            assert_eq!(scratch.capacity(), 128);
        }
    }
}
