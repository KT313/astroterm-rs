use super::*;
use crate::{model::StarRow, progress::NoProgress};

fn fixture(count: usize, gaia: bool, one_region: bool) -> (StarStorage, Vec<f32>) {
    let mut stars = if gaia { StarStorage::gaia() } else { StarStorage::default() };
    stars.try_reserve_exact(count + 7).unwrap();
    let mut bounds = Vec::with_capacity(count + 7);
    for i in 0..count {
        let angle = i as f64 * 2.399963229728653;
        let u0 = if one_region { [1.0, 0.0, 0.0] } else { [angle.cos() as f32, angle.sin() as f32, 0.0] };
        let row = StarRow { u0, w: [f32::from_bits(i as u32), -0.0, 0.0], magnitude: (i % 65536) as u16,
            brightness_key: (i % 17) as u16, distance: i as f32, id: u32::MAX - i as u32,
            name: if gaia { 0 } else { i as u32 + 1 }, display_color: (i % 8) as u8 };
        stars.push_prepared(row, [0, 0]);
        if gaia { stars.push_source_id((1u64 << 54) + i as u64); }
        bounds.push(f32::from_bits(i as u32));
    }
    (stars, bounds)
}

fn old_order(stars: &StarStorage, endpoints: &HashSet<StarId>) -> Vec<usize> {
    let cells: Vec<_> = (0..stars.len()).map(|i| if endpoints.contains(&stars.id(i)) { CONSTELLATION_REGION } else { grid::stored_cell(stars, i) }).collect();
    let mut order: Vec<_> = (0..stars.len()).collect();
    order.sort_unstable_by(|&a, &b| cells[a].cmp(&cells[b])
        .then_with(|| stars.brightness_key(a).total_cmp(&stars.brightness_key(b)))
        .then_with(|| stars.id(b).cmp(&stars.id(a))));
    order
}

#[test]
fn encoded_sort_key_matches_every_decoded_brightness_code_and_tie_break() {
    for code in 0..u16::MAX {
        assert!(crate::catalog::decode_magnitude(code) < crate::catalog::decode_magnitude(code + 1));
        assert!(build_record(0, code, 0, 0).unwrap().key < build_record(0, code + 1, u32::MAX, 0).unwrap().key);
    }
    assert!(build_record(0, u16::MAX, 0, 0).unwrap().key < build_record(1, 0, u32::MAX, 0).unwrap().key);
    assert!(build_record(0, 123, u32::MAX, 0).unwrap().key < build_record(0, 123, 0, 0).unwrap().key);
    assert!(build_record(65536, 0, 0, 0).is_err());
    #[cfg(target_pointer_width = "64")]
    assert!(build_record(0, 0, 0, u32::MAX as usize + 1).is_err());
}

#[test]
fn packed_order_gathered_columns_and_reused_counts_match_the_cycle_oracle() {
    for count in [0, 1, 129, 4097] {
        for gaia in [false, true] {
            for one_region in [false, true] {
                let (original, bounds) = fixture(count, gaia, one_region);
                let endpoints: HashSet<_> = (0..count).filter(|i| i % 7 == 0).map(|i| original.id(i)).collect();
                let order = old_order(&original, &endpoints);
                let mut expected = original.clone();
                let mut expected_bounds = bounds.clone();
                expected.reorder(&order, &mut expected_bounds, &mut crate::progress::NoProgress(std::io::sink()));
                let (mut actual, mut actual_bounds) = (original, bounds);
                let capacity = actual.label_capacity();
                let bounds_capacity = actual_bounds.capacity();
                let mut progress = NoProgress(io::sink());
                let counts = sort_stars_by_region_and_brightness(&mut actual, &mut actual_bounds, &endpoints, &mut progress, &mut StepTimes::default(), None).unwrap();
                assert_eq!(actual.cache_sections(), expected.cache_sections());              // raw bytes also check negative zero and subnormal motion components
                assert_eq!(actual.source_ids(), expected.source_ids());
                assert_eq!(bytemuck::cast_slice::<_, u8>(&actual_bounds), bytemuck::cast_slice::<_, u8>(&expected_bounds));
                assert_eq!((actual.label_capacity(), actual_bounds.capacity()), (capacity, bounds_capacity));
                let endpoints: Vec<_> = (count - endpoints.len()..count).collect();
                let expected_grid = grid::build_grid(&actual, &endpoints, &mut progress);
                let actual_grid = grid::build_grid_from_counts(count, counts, &endpoints, &mut progress).unwrap();
                assert_eq!(actual_grid, expected_grid);
            }
        }
    }
}

#[test]
fn parallel_sorts_match_serial_order_for_different_worker_counts() {
    let (stars, _) = fixture(20_000, true, false);
    let (records, _) = build_records(&stars, &HashSet::new(), &mut NoProgress(io::sink())).unwrap();
    let expected = old_order(&stars, &HashSet::new());
    let ids: Vec<_> = (0..20000u64).rev().map(|id| id / 3).collect();
    let mut expected_ids = ids.clone();
    expected_ids.sort_unstable();
    for workers in [1, 2, 4] {
        let pool = ThreadPoolBuilder::new().num_threads(workers).build().unwrap();
        let mut sorted = records.clone();
        sort_values(&mut sorted, Some(&pool), 0, |a, b| a.key.cmp(&b.key));                      // exercise production parallel sorting on a small fixture
        assert_eq!(sorted.iter().map(|r| r.original_index as usize).collect::<Vec<_>>(), expected);
        let mut actual_ids = ids.clone();
        sort_values(&mut actual_ids, Some(&pool), 0, u64::cmp);
        assert_eq!(actual_ids, expected_ids);                                                // duplicates remain for the later uniqueness check
    }
}

#[test]
fn startup_threads_are_capped_by_available_cpus_with_a_minimum_of_one() {
    for (available, expected) in [(0, 1), (1, 1), (2, 2), (3, 3), (4, 4), (8, 4), (24, 4)] {
        assert_eq!(choose_threads(available), expected);
    }
}

#[test]
fn bad_counts_and_permutations_fail_before_publication() {
    let (mut stars, mut bounds) = fixture(5, true, false);
    let original = stars.clone();
    assert!(stars.gather_ordered([0, 1, 2, 3, 5].into_iter(), &mut bounds, &mut NoProgress(io::sink())).is_err());
    assert_eq!(stars, original);
    assert!(grid::build_grid_from_counts(5, vec![0; SIMULATION_REGION_COUNT + 1], &[], &mut NoProgress(io::sink())).is_err());
}

#[test]
#[ignore = "manual release comparison; 400k synthetic rows, no downloads"]
fn compare_gather_and_cycle_reordering() {
    let (original, bounds) = fixture(400_000, true, false);
    let started = std::time::Instant::now();
    let old = old_order(&original, &HashSet::new());
    let old_sort = started.elapsed();
    let started = std::time::Instant::now();
    let (mut records, _) = build_records(&original, &HashSet::new(), &mut NoProgress(io::sink())).unwrap();
    let assignment = started.elapsed();
    let started = std::time::Instant::now();
    records.sort_unstable_by_key(|record| record.key);
    let sort = started.elapsed();
    let order: Vec<_> = records.iter().map(|r| r.original_index as usize).collect();
    assert_eq!(order, old);
    let (mut cycle, mut cycle_bounds) = (original.clone(), bounds.clone());
    let started = std::time::Instant::now();
    cycle.reorder(&order, &mut cycle_bounds, &mut crate::progress::NoProgress(std::io::sink()));
    let cycle_time = started.elapsed();
    let (mut gathered, mut gathered_bounds) = (original, bounds);
    let started = std::time::Instant::now();
    gathered.gather_ordered(order.iter().copied(), &mut gathered_bounds, &mut NoProgress(io::sink())).unwrap();
    let gather_time = started.elapsed();
    assert_eq!(gathered.cache_sections(), cycle.cache_sections());
    assert_eq!(gathered.source_ids(), cycle.source_ids());
    assert_eq!(gathered_bounds, cycle_bounds);
    eprintln!("rows=400000; record_bytes={}; old_assignment_sort={old_sort:?}; record_assignment={assignment:?}; packed_sort={sort:?}; cycle={cycle_time:?}; gather={gather_time:?}", size_of::<SortRecord>());
}

#[test]
fn sort_and_reorder_descriptions_attach_to_their_completed_steps() {
    let (mut stars, mut bounds) = fixture(7, true, false);
    let expected_bytes = largest_replacement_bytes(&stars, &bounds);
    let mut times = StepTimes::with_trace(true);
    sort_stars_by_region_and_brightness(&mut stars, &mut bounds, &HashSet::new(), &mut NoProgress(io::sink()), &mut times, None).unwrap();
    let steps = &times.trace().unwrap().steps;
    let assignment = steps.iter().find(|step| step.name == "Region assignment").unwrap();
    let reorder = steps.iter().find(|step| step.name == "Star field permutation").unwrap();
    assert_eq!(assignment.details, [format!("sort records=7; record bytes={}; allocated payload bytes={}", size_of::<SortRecord>(), 7 * size_of::<SortRecord>())]);
    assert_eq!(reorder.details, [format!("largest replacement column bytes={expected_bytes}; one column replaced at a time")]);
}
