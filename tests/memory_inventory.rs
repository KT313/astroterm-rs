//! Inventories use live typed containers without changing computation or cache state.
use std::{collections::HashMap, mem::size_of, sync::Arc};
use astroterm::cache::{
    Cache, CacheConfig, BufferSink, InventorySnapshot, Kind, Owner, Quality, ReportBuffers, report_field,
};
use astroterm::state::{
    ApplicationState, RunState, collect_inventory, write_inventory, MAX_ROWS, MAX_DEPTH, MAX_CHILDREN,
};
use astroterm::cli::Arguments;
use astroterm::cli::build_config;
use clap::Parser;

fn heap(snapshot: &InventorySnapshot, owner: Owner) -> (usize, usize) {
    snapshot.rows.iter().filter(|r| r.kind == Kind::Heap && r.owner == owner)
        .fold((0, 0), |(used, reserved), r| (used + r.used.unwrap(), reserved + r.reserved.unwrap()))
}

#[test]
fn flat_nested_optional_and_zero_sized_payloads_do_not_double_count_headers() {
    let mut bytes = Vec::<u8>::with_capacity(32);
    bytes.extend([1, 2, 3]);
    let snapshot = collect_inventory("bytes", &bytes);
    assert_eq!(heap(&snapshot, Owner::Application), (3, 32));
    assert_eq!(snapshot.root_inline, size_of::<Vec<u8>>());
    bytes.clear();
    assert_eq!(heap(&collect_inventory("cleared", &bytes), Owner::Application), (0, 32));

    let mut a = Vec::<u8>::with_capacity(8); a.extend([1, 2, 3, 4]);
    let mut b = Vec::<u8>::with_capacity(16); b.extend([0; 6]);
    let mut nested = Vec::with_capacity(4); nested.push(a); nested.push(b);
    assert_eq!(heap(&collect_inventory("nested", &nested), Owner::Application),
        (2 * size_of::<Vec<u8>>() + 10, 4 * size_of::<Vec<u8>>() + 24));
    let some = Some(vec![1_u32, 2, 3]);
    assert_eq!(heap(&collect_inventory("some", &some), Owner::Application).0, 12);
    assert_eq!(heap(&collect_inventory("none", &None::<Vec<u32>>), Owner::Application), (0, 0));
    assert_eq!(heap(&collect_inventory("zst", &vec![(); 3]), Owner::Application), (0, 0));
    let mut text = String::with_capacity(24); text.push('星');
    assert_eq!(heap(&collect_inventory("string", &text), Owner::Application), (3, 24));
}

#[test]
fn shared_payloads_count_once_and_identity_is_local_to_each_snapshot() {
    let mut data = Vec::<u8>::with_capacity(16); data.extend([1, 2, 3]);
    let shared = Arc::new(data);
    let pair = (shared.clone(), shared.clone());
    let first = collect_inventory("first", &pair);
    let second = collect_inventory("second", &pair);
    assert_eq!(heap(&first, Owner::Shared), (size_of::<Vec<u8>>() + 3, size_of::<Vec<u8>>() + 16));
    assert_eq!(heap(&first, Owner::Shared), heap(&second, Owner::Shared));
    assert_eq!(first.rows.iter().filter(|r| r.kind == Kind::Alias).count(), 1);
    assert!(first.rows.iter().any(|r| r.note.contains("Arc control block") && r.quality == Quality::LowerBound));
    assert_eq!(Arc::strong_count(&shared), 3); // enumeration does not acquire another owning reference
    let repeated = vec![shared.clone(); 20];
    let grouped = collect_inventory("grouped aliases", &repeated);
    assert_eq!(heap(&grouped, Owner::Shared), heap(&first, Owner::Shared));
    assert!(grouped.rows.iter().any(|r| r.kind == Kind::Alias && r.grouped_rows > 1));
}

#[test]
fn invalidated_cache_is_inspected_without_reading_through_its_validity_gate() {
    let mut cache = Cache::default();
    cache.store(vec![1_u32], 2451545.0, 1.0, vec![9_u8; 12]);
    cache.invalidate();
    let before = cache.clone();
    let snapshot = collect_inventory("invalid", &cache);
    assert_eq!(heap(&snapshot, Owner::Application).0, 16);
    assert_eq!(cache, before);
    assert!(cache.has_been_invalidated);
}

#[test]
fn large_flat_maps_do_not_visit_each_cached_star() {
    let values: HashMap<usize, Cache<(), f64>> = (0..10000).map(|i| (i, Cache::default())).collect();
    let snapshot = collect_inventory("map", &values);
    assert_eq!(snapshot.rows.len(), 2); // header plus logical payload; no scan of individual entries
    assert_eq!(snapshot.rows[1].quality, Quality::LowerBound);
    assert_eq!(snapshot.rows[1].elements, Some(10000));
    assert!(snapshot.rows[1].note.contains("control bytes"));
}

#[test]
fn bounded_nested_inspection_and_unknown_sizes_are_explicit() {
    struct Opaque;
    impl ReportBuffers for Opaque {
        fn report_buffers(&self, sink: &mut dyn BufferSink) { sink.unknown("opaque fixture"); }
    }
    struct Overflow;
    impl ReportBuffers for Overflow {
        fn report_buffers(&self, sink: &mut dyn BufferSink) {
            sink.payload(usize::MAX, usize::MAX, 2, Quality::ExactPayload, "overflow fixture");
        }
    }
    struct Node(Option<Box<Node>>);
    impl ReportBuffers for Node {
        fn report_buffers(&self, sink: &mut dyn BufferSink) { report_field(sink, "child", &self.0); }
    }
    let mut node = Node(None);
    for _ in 0..MAX_DEPTH * 2 { node = Node(Some(Box::new(node))); }
    assert!(collect_inventory("deep", &node).omitted_nodes > 0);
    let nested = vec![vec![String::from("payload"); MAX_CHILDREN + 1]; MAX_CHILDREN + 1];
    let report = collect_inventory("large", &nested);
    assert!(report.rows.len() <= MAX_ROWS);
    assert!(report.omitted_nodes > 0);
    let opaque = collect_inventory("unknown", &Opaque);
    assert!(opaque.rows.iter().any(|r| r.kind == Kind::Unknown && r.reserved.is_none()));
    let overflow = collect_inventory("overflow", &Overflow);
    assert!(overflow.rows.iter().any(|r| r.kind == Kind::Heap && r.quality == Quality::Unknown && r.reserved.is_none()));
    let mut output = Vec::new(); write_inventory(&overflow, &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("reserved unknown"));
}

#[test]
fn mapped_sections_and_name_clones_share_one_logical_mapping() {
    use astroterm::catalog::datasets::{Dataset, DatasetDirectories};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stars.csv");
    std::fs::write(&path, "ra,dec,mag,proper\n0,0,1,One\n1,2,3,Two\n").unwrap();
    let dirs = DatasetDirectories { data: None, cache: Some(temp.path().join("cache")) };
    let dataset = Some(Dataset::Path(path));
    let owned = astroterm::sky::load_sky_catalog(dataset.as_ref(), &dirs, &mut Vec::new()).unwrap();
    assert!(!owned.stars.is_mapped());
    let mapped = astroterm::sky::load_sky_catalog(dataset.as_ref(), &dirs, &mut Vec::new()).unwrap();
    assert!(mapped.stars.is_mapped());
    let catalog = Arc::new(mapped);
    let sky = astroterm::model::Sky::new(catalog.clone());
    let snapshot = collect_inventory("mapped", &(catalog, sky));
    let mappings: Vec<_> = snapshot.rows.iter().filter(|r| r.kind == Kind::Mapping).collect();
    assert_eq!(mappings.len(), 1);
    let cache_files: Vec<_> = std::fs::read_dir(dirs.cache.as_ref().unwrap()).unwrap()
        .map(|entry| entry.unwrap().path()).collect();
    assert_eq!(cache_files.len(), 1);
    let mapped_length = usize::try_from(std::fs::metadata(&cache_files[0]).unwrap().len()).unwrap();
    assert_eq!((mappings[0].used, mappings[0].reserved), (Some(mapped_length), Some(mapped_length)));
    assert_eq!(mappings[0].quality, Quality::ExactPayload); // file extent, not resident pages or heap capacity
    assert!(snapshot.rows.iter().filter(|r| r.kind == Kind::Alias).count() > 10);
    assert!(!collect_inventory("owned", &owned).rows.iter().any(|r| r.kind == Kind::Mapping));
}

#[test]
fn inventory_does_not_change_output_or_cache_statistics() {
    use astroterm::astro::{J2000, Observer};
    use astroterm::model::{ProjectionViewport as Viewport, FrameTime};
    let config = build_config(Arguments::try_parse_from(["astroterm", "--debug-singleframe", "--debug-memory"]).unwrap(), &[]).unwrap();
    let mut state = ApplicationState::new(config, astroterm::timing::StepTimes::with_trace(true));
    state.replace_catalog(Arc::new(astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog().unwrap())));
    for policy in [CacheConfig::default(), CacheConfig::disabled()] {
        state.run.simulation.configure_cache(&policy);
        state.run.observation = astroterm::state::ObservationCache::new(policy.clone());
        state.run.projection = astroterm::state::ProjectionCache::new(policy);
        let active = &mut state.run;
        let RunState { sky, simulation, observation, projection, .. } = &mut *active;
        let time = FrameTime::from_utc(J2000);
        astroterm::sky::update_simulation(simulation, time, &[], &mut state.timings).unwrap();
        let mut site = astroterm::sky::prepare_cached_observer(observation, simulation, time, Observer::default()).unwrap();
        astroterm::sky::prepare_cached_light_time(observation, simulation, &mut site, &mut state.timings).unwrap();
        astroterm::sky::observe_cached_sky(observation, simulation, &site, 5.0, true, astroterm::model::SkyRegion::All, sky, &mut state.timings).unwrap();
        astroterm::projection::project_cached_sky(projection, sky, &astroterm::model::View::default(), Viewport { width: 80, height: 40 }, time.tt, &mut state.timings);
        let before = (sky.stars.clone(), observation.reports(), projection.stats());
        let _ = collect_inventory("run", &*active);
        assert_eq!(before, (active.sky.stars.clone(), active.observation.reports(), active.projection.stats()));
    }
    assert!(!state.run.sky.stars.is_empty());
    assert!(state.run.observation.stats().refreshes > 0); // completed working data remains inspectable in the root
}

#[test]
fn snapshot_callbacks_are_lazy_and_history_is_limited_to_two() {
    let mut off = astroterm::timing::StepTimes::default();
    off.capture_memory(|_| panic!("disabled trace must not call collector"));
    let mut on = astroterm::timing::StepTimes::with_trace(true);
    for _ in 0..2 { on.capture_memory(|_| collect_inventory("small", &vec![1_u8])); }
    on.capture_memory(|_| panic!("single-frame bound must skip callback"));
    assert_eq!(on.trace().unwrap().memory_snapshots.len(), 2);
    assert!(on.trace().unwrap().unscoped_diagnostic_seconds >= 0.0);
}

#[test]
fn nested_children_are_grouped_without_losing_inspected_payloads() {
    use astroterm::state::{DETAIL_CHILDREN, sum_known_payload};
    let values: Vec<Vec<u8>> = (0..88).map(|i| vec![7; i + 1]).collect();
    let snapshot = collect_inventory("figures", &values);
    let total = sum_known_payload(&snapshot, Owner::Application);
    let expected = values.len() * size_of::<Vec<u8>>() + values.iter().map(Vec::len).sum::<usize>();
    assert_eq!(total.used, Some(expected));
    assert_eq!(total.unknown_records, 0);
    assert_eq!(snapshot.omitted_nodes, 0);
    let grouped = snapshot.rows.iter().find(|r| r.kind == Kind::Heap && r.path.ends_with("[*]")).unwrap();
    assert_eq!(grouped.grouped_rows, 88 - DETAIL_CHILDREN);
    assert!(snapshot.rows.len() < 20);
    let large: Vec<Vec<u8>> = (0..MAX_CHILDREN + 10).map(|_| vec![0; 3]).collect();
    let partial = collect_inventory("bounded", &large);
    assert!(partial.omitted_nodes >= 10);
    assert_eq!(sum_known_payload(&partial, Owner::Application).used,
        Some(large.len() * size_of::<Vec<u8>>() + MAX_CHILDREN * 3)); // no extrapolation of uninspected children
}

#[test]
fn unknown_payloads_do_not_erase_known_totals_and_overflow_is_distinct() {
    use astroterm::state::sum_known_payload;
    struct Mixed;
    impl ReportBuffers for Mixed {
        fn report_buffers(&self, sink: &mut dyn BufferSink) {
            sink.payload(8, 16, 1, Quality::ExactPayload, "known fixture");
            sink.unknown("opaque fixture");
            sink.payload(usize::MAX, usize::MAX, 2, Quality::Unknown, "unavailable fixture");
        }
    }
    let snapshot = collect_inventory("mixed", &Mixed);
    let total = sum_known_payload(&snapshot, Owner::Application);
    assert_eq!((total.used, total.reserved, total.unknown_records), (Some(8), Some(16), 2));
    struct TooMuch;
    impl ReportBuffers for TooMuch {
        fn report_buffers(&self, sink: &mut dyn BufferSink) {
            sink.payload(usize::MAX, usize::MAX, 1, Quality::ExactPayload, "known large fixture");
            sink.payload(1, 1, 1, Quality::ExactPayload, "known second fixture");
        }
    }
    let total = sum_known_payload(&collect_inventory("overflow", &TooMuch), Owner::Application);
    assert_eq!((total.used, total.reserved, total.unknown_records), (None, None, 0));
}

#[test]
fn report_groups_working_data_and_distinguishes_references_from_owned_payload() {
    use astroterm::state::InventoryCollector;
    let mut collector = InventoryCollector::new("latest completed frame", Some(2451545.0));
    collector.enter("state", 64);
    collector.enter("catalog", 8);
    assert!(collector.begin_shared(123, 8));
    collector.payload(2, 4, 16, Quality::ExactPayload, "catalog payload");
    collector.mapping(456, 4096);
    collector.end_shared();
    collector.leave();
    collector.enter("run", 32);
    for name in ["simulation", "observation", "projection", "rendering"] {
        collector.enter(name, 8);
        collector.payload(0, 16, 1, Quality::ExactPayload, "cleared retained scratch");
        collector.leave();
    }
    collector.enter("sky", 8);
    assert!(!collector.begin_shared(123, 8));
    collector.mapping(456, 4096);
    collector.borrowed(2, 16, "read-only fixture");
    collector.leave();
    collector.leave();
    collector.set_owner(Owner::External);
    collector.unknown("opaque writer buffer");
    collector.leave();
    let snapshot = collector.finish();
    let before = snapshot.clone();
    let mut output = Vec::new();
    write_inventory(&snapshot, &mut output).unwrap();
    assert_eq!(snapshot, before); // formatting reads saved metadata and cannot revisit live state
    let text = String::from_utf8(output).unwrap();
    for label in ["Catalog —", "Simulation —", "Observation —", "Projection —", "Rendering —", "External —"] {
        assert!(text.contains(label), "missing group {label}");
    }
    assert!(text.contains("Known Application payload: used 0 B; reserved 64 B; unknown records=0"));
    assert!(text.contains("Known Shared payload: used 40 B; reserved 72 B; unknown records=0"));
    assert!(text.contains("Mapped logical bytes: 4.0 KiB"));
    assert!(text.contains("reference only; adds no payload to totals"));
    assert!(text.contains("len=2; referenced 32 B; adds no owned payload"));
    assert!(text.contains("size unknown; excluded from known totals"));
    assert!(text.contains("len=0 capacity=16; used 0 B; reserved 16 B"));
    assert!(!text.contains("Some("));
}

#[test]
fn inventory_reports_propagate_output_failure_without_mutating_the_snapshot() {
    struct Fails;
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> { Err(std::io::ErrorKind::BrokenPipe.into()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let snapshot = collect_inventory("fixture", &vec![1_u8; 3]);
    let before = snapshot.clone();
    assert_eq!(write_inventory(&snapshot, &mut Fails).unwrap_err().kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(snapshot, before);
}


#[test]
fn canvas_and_image_payloads_use_actual_element_capacity() {
    use astroterm::canvas::{Canvas, Cell};
    let mut pixels = Vec::with_capacity(128);
    pixels.resize(4 * 3 * 4, 0_u8);
    let capacity = pixels.capacity();
    let image = image::RgbaImage::from_raw(4, 3, pixels).unwrap();
    let snapshot = collect_inventory("image", &image);
    assert_eq!(heap(&snapshot, Owner::Application), (48, capacity));
    let payload = snapshot.rows.iter().find(|row| row.kind == Kind::Heap).unwrap();
    assert_eq!(payload.quality, Quality::ExactPayload);
    assert_eq!(payload.elements, Some(48)); // bytes rather than the 12-pixel count

    let mut canvas = Canvas::new(3, 4);
    let before = collect_inventory("canvas", &canvas);
    assert_eq!(heap(&before, Owner::Application), (12 * size_of::<Cell>(), 12 * size_of::<Cell>()));
    canvas.clear();
    assert_eq!(heap(&collect_inventory("cleared canvas", &canvas), Owner::Application), heap(&before, Owner::Application));
    canvas.resize(1, 2);
    assert_eq!(heap(&collect_inventory("resized canvas", &canvas), Owner::Application), (2 * size_of::<Cell>(), 2 * size_of::<Cell>()));
}

#[test]
fn glyph_masks_are_exact_but_map_and_font_coverage_remain_partial() {
    use astroterm::scene::{begin_text_frame, create_text_rasterizer, draw_text};
    let mut rasterizer = create_text_rasterizer().unwrap();
    let mut image = image::RgbaImage::new(40, 24);
    draw_text(&mut rasterizer, &mut image, "AA", (0, 0), (40, 24), [255, 255, 255]);
    let snapshot = collect_inventory("glyphs", &rasterizer);
    let map = snapshot.rows.iter().find(|row| row.kind == Kind::Heap && row.path.ends_with(".glyphs")).unwrap();
    assert_eq!(map.elements, Some(1)); // repeated glyph uses the same owned mask
    assert_eq!(map.quality, Quality::LowerBound);
    let coverage: Vec<_> = snapshot.rows.iter().filter(|row| row.kind == Kind::Heap && row.path.ends_with(".coverage")).collect();
    assert_eq!(coverage.len(), 1);
    assert!(coverage[0].used.unwrap() > 0);
    assert_eq!(coverage[0].used, coverage[0].elements); // one byte of coverage per mask pixel
    assert_eq!(coverage[0].quality, Quality::ExactPayload);
    assert!(snapshot.rows.iter().any(|row| row.kind == Kind::Unknown && row.note.contains("fontdue")));

    begin_text_frame(&mut rasterizer, false);
    let cleared = collect_inventory("cleared glyphs", &rasterizer);
    let cleared_map = cleared.rows.iter().find(|row| row.kind == Kind::Heap && row.path.ends_with(".glyphs")).unwrap();
    assert_eq!(cleared_map.elements, Some(0));
    assert_eq!(cleared_map.capacity, map.capacity);
    assert!(!cleared.rows.iter().any(|row| row.path.ends_with(".coverage"))); // masks are dropped; map buckets remain
    assert!(cleared.rows.iter().any(|row| row.kind == Kind::Unknown && row.note.contains("fontdue")));
}

#[test]
fn saved_inventory_accounts_for_descriptors_without_recounting_the_subject() {
    let subject = vec![0_u8; 1_000_000];
    let saved = collect_inventory("large subject", &subject);
    assert_eq!(saved.collector_retained_bytes, saved.retained_bytes());
    assert!(saved.collector_temporary_bytes.unwrap() > 0); // collector path/dedup/scope storage is separate
    let saved_bytes = (saved.used_bytes().unwrap(), saved.retained_bytes().unwrap());
    let mut times = astroterm::timing::StepTimes::with_trace(true);
    times.capture_memory(|_| saved);
    let inventory = collect_inventory("diagnostics", &times);
    let descriptor_rows: Vec<_> = inventory.rows.iter()
        .filter(|row| row.note == "captured descriptors and path bytes/capacities").collect();
    assert_eq!(descriptor_rows.len(), 1);
    assert_eq!((descriptor_rows[0].used, descriptor_rows[0].reserved), (Some(saved_bytes.0), Some(saved_bytes.1)));
    assert!(heap(&inventory, Owner::Application).1 < subject.len()); // saved counts are metadata, not another MB buffer
    assert!(!inventory.rows.iter().any(|row| row.path.contains("large subject")));
}

#[test]
fn root_capture_keeps_diagnostic_and_external_payloads_separate() {
    use astroterm::state::{capture_run_inventory, sum_known_payload};
    let config = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
    let mut times = astroterm::timing::StepTimes::with_trace(true);
    times.measure("fixture", || ());
    times.describe("fixture", || "retained diagnostic detail".into());
    times.capture_memory(|_| collect_inventory("prior fixture", &vec![0_u8; 32]));
    let state = ApplicationState::new(config, times); // the empty startup catalog is enough for a root capture
    let writer = Vec::<u8>::with_capacity(256); // stand-in for the external writer's known buffer
    let snapshot = capture_run_inventory(&state.config, &state.catalog, &state.run, &writer, &state.timings, "root", None);
    assert_eq!(snapshot.omitted_nodes, 0);
    assert!(sum_known_payload(&snapshot, Owner::Diagnostics).used.unwrap() > 0);
    assert_eq!(sum_known_payload(&snapshot, Owner::External).reserved, Some(writer.capacity()));
    assert!(sum_known_payload(&snapshot, Owner::External).unknown_records > 0);
    assert!(snapshot.rows.iter().filter(|row| row.path.starts_with("state.timings")).all(|row| row.owner == Owner::Diagnostics));
    assert!(snapshot.rows.iter().any(|row| row.kind == Kind::Alias && row.path == "state.run.sky.catalog"));
    assert!(snapshot.rows.iter().any(|row| row.owner == Owner::External && row.note.contains("startup data")));
}
