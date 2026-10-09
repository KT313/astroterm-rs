use super::*;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;
use crate::cache::{CacheConfig, GroupPolicy};
use crate::model::View;

fn pixels() -> PixelState { super::super::lifetime_tests::pixels(ProtocolType::Kitty) }

fn prepare_sky(state: &mut PixelState) {
    let sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
    let projected = crate::projection::project_sky(&sky, &View::default(), state.viewport);
    super::super::rasterize_pixel_sky(state, &projected.view(&sky), None, 2451545.0, &mut StepTimes::default()).unwrap();
}

fn set_text(state: &mut PixelState, symbol: &str) {
    state.text = ratatui::buffer::Buffer::empty(state.screen);
    state.text[(0, 0)].set_symbol(symbol);
    state.text_version.publish(true);
}

fn used(times: &StepTimes, name: &str) -> bool { times.steps().iter().any(|step| step.name == name) }

#[test]
fn unchanged_complete_image_skips_all_pixel_work_and_keeps_published_version() {
    let mut state = pixels();
    prepare_sky(&mut state);
    set_text(&mut state, "A");
    let text_revision = state.text_version;
    let completed_text = state.text.clone();
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    let version = state.rgb_version.current();
    let pointer = state.rgb.as_ptr();
    state.text_version = text_revision;
    state.text = completed_text;
    let mut times = StepTimes::with_trace(true);
    prepare_kitty_pixels(&mut state, (2, 2), &mut times).unwrap();
    assert_eq!(state.rgb_version.current(), version);
    assert_eq!(state.rgb.as_ptr(), pointer);
    for pass in ["Frame canvas", "Sky composition", "Text rasterization", "Pixel conversion"] { assert!(!used(&times, pass), "{pass} ran on a hit"); }
    assert!(used(&times, "Full image cache decision"));
}

#[test]
fn pixel_storage_reuses_capacity_and_matches_the_previous_conversion() {
    let mut state = pixels();
    let mut rgba_capacity = None;
    let mut rgb_capacity = None;
    for (width, height) in [(31, 17), (1, 1), (7, 9), (31, 17)] {
        state.screen = Rect::new(0, 0, width, height);
        state.font = ratatui_image::FontSize::new(1, 1);
        initialize_pixel_canvas(&mut state, &mut StepTimes::default()).unwrap();
        let frame = state.frame_image.as_mut().unwrap();
        for (x, y, pixel) in frame.enumerate_pixels_mut() { *pixel = image::Rgba([x as u8, y as u8, (x * 7 + y) as u8, 255]); }
        let expected = image::DynamicImage::ImageRgba8(frame.clone()).into_rgb8();
        convert_kitty_pixels(&mut state, &mut StepTimes::default()).unwrap();
        assert_eq!(state.rgb, expected);
        assert_eq!(*rgba_capacity.get_or_insert(state.frame_image.as_ref().unwrap().as_raw().capacity()), state.frame_image.as_ref().unwrap().as_raw().capacity());
        assert_eq!(*rgb_capacity.get_or_insert(state.rgb.as_raw().capacity()), state.rgb.as_raw().capacity());
    }
}

#[test]
fn changing_or_removing_text_recomposes_clean_sky_and_changes_version() {
    let mut state = pixels();
    prepare_sky(&mut state);
    set_text(&mut state, "A");
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    let first_version = state.rgb_version.current();
    let first_pixels = state.rgb.clone();
    set_text(&mut state, " ");
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    assert_ne!(state.rgb_version.current(), first_version);
    assert_ne!(state.rgb, first_pixels);
    let clean = image::DynamicImage::ImageRgba8(state.scene_cache.pixel_image().clone()).into_rgb8();
    assert_eq!(state.rgb, clean, "old text must not accumulate or survive in the retained composition buffer");
}

#[test]
fn missing_text_version_size_failure_and_each_disabled_group_prevent_reuse() {
    let mut state = pixels();
    prepare_sky(&mut state);
    set_text(&mut state, " ");
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    let version = state.rgb_version.current();
    state.text_version.invalidate();
    assert!(prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).is_err());
    assert_ne!(state.rgb_version.current(), version);
    assert!(state.frame_key.is_none());
    assert!(state.rgb_version.current().is_none());
    state.scene_cache.invalidate();
    set_text(&mut state, " ");
    assert!(prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).is_err());

    for group in [Group::Raster, Group::RasterAssets] {
        let mut config = CacheConfig::default();
        config.groups.insert(group, GroupPolicy { enabled: false, ..Default::default() });
        state.scene_cache.configure(&config);
        prepare_sky(&mut state);
        set_text(&mut state, " ");
        let text_version = state.text_version;
        let completed_text = state.text.clone();
        prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
        let version = state.rgb_version.current();
        state.text_version = text_version;
        state.text = completed_text;
        prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
        assert_ne!(state.rgb_version.current(), version, "{group:?} bypass did not rebuild RGB");
    }
    state.screen = Rect::new(0, 0, u16::MAX, u16::MAX);
    assert!(prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).is_err());
    assert!(state.rgb_version.current().is_none());
    assert!(state.frame_key.is_none());
    state.screen = Rect::new(0, 0, 8, 4);
    set_text(&mut state, " ");
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    assert!(state.rgb_version.current().is_some());
}

#[test]
fn composition_key_covers_layout_inputs_and_sky_generation() {
    let mut state = pixels();
    prepare_sky(&mut state);
    set_text(&mut state, " ");
    let text_revision = state.text_version;
    let completed_text = state.text.clone();
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    let first = state.frame_key.unwrap();
    for change in 0..5 {
        state.text_version = text_revision;
        state.text = completed_text.clone();
        let before = state.rgb_version.current();
        match change {
            0 => state.area.x += 1,
            1 => state.screen.width += 1,
            2 => state.font.height += 1,
            3 => state.scene_cache.pixels.generation += 1,
            _ => {},
        }
        prepare_kitty_pixels(&mut state, if change == 4 { (3, 2) } else { (2, 2) }, &mut StepTimes::default()).unwrap();
        assert_ne!(state.rgb_version.current(), before);
        assert_ne!(state.frame_key.unwrap(), first);
    }
}

fn prepare_rgb(state: &mut PixelState) {
    state.rgb = image::RgbImage::from_fn(127, 83, |x, y| image::Rgb([(x * 31 + y * 17) as u8, (x * y + 71) as u8, (x * 91 + y * 43) as u8]));
    state.rgb_version.publish(true);
}

#[test]
fn encoding_reuses_same_revision_target_and_transport_without_calling_encoder() {
    let mut state = pixels();
    prepare_rgb(&mut state);
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    let payload = state.upload.clone();
    let pointer = state.upload.as_ptr();
    let mut times = StepTimes::with_trace(true);
    encode_kitty_upload_with(&mut state, &mut times, |_| panic!("encoder ran on a hit")).unwrap();
    assert_eq!(state.upload, payload);
    assert_eq!(state.upload.as_ptr(), pointer);
    assert!(!used(&times, "Image encoding"));
}

fn decode_upload(upload: &str, compressed: bool) -> Vec<u8> {
    use std::io::Read;
    let unwrapped = upload.replace("\x1bPtmux;", "").replace("\x1b\x1b", "\x1b");
    let mut bytes = Vec::new();
    for command in unwrapped.split("\x1b_G").skip(1) {
        let payload = command.split_once(';').unwrap().1.split("\x1b\\").next().unwrap();
        bytes.extend(base64_simd::STANDARD.decode_to_vec(payload).unwrap());
    }
    if !compressed { return bytes; }
    let mut decoded = Vec::new();
    flate2::read::ZlibDecoder::new(bytes.as_slice()).read_to_end(&mut decoded).unwrap();
    decoded
}

#[test]
fn encoding_key_changes_produce_correct_multichunk_pixels_headers_and_dimensions() {
    let mut state = pixels();
    prepare_rgb(&mut state);
    for compression in [CompressionSupport::Supported, CompressionSupport::Unsupported, CompressionSupport::Unknown] {
        for tmux in [false, true] {
            for id in kitty::IMAGE_IDS {
                state.compression = compression;
                state.tmux = tmux;
                state.kitty_image_id = id;
                let mut times = StepTimes::with_trace(true);
                encode_kitty_upload(&mut state, &mut times).unwrap();
                assert!(used(&times, "Image encoding"));
                assert!(state.upload.contains(&format!("i={id},")));
                assert!(state.upload.matches("m=").count() > 1);
                assert_eq!(decode_upload(&state.upload, compression == CompressionSupport::Supported), *state.rgb.as_raw());
            }
        }
    }
    state.rgb = image::RgbImage::from_pixel(3, 5, image::Rgb([17, 29, 101]));
    state.rgb_version.publish(true);
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    assert!(state.upload.contains("s=3,v=5,"));
    assert_eq!(decode_upload(&state.upload, false), *state.rgb.as_raw());
    state.rgb.put_pixel(0, 0, image::Rgb([11, 22, 33]));
    state.rgb_version.publish(true);
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    assert_eq!(decode_upload(&state.upload, false), *state.rgb.as_raw());
}

#[test]
fn incomplete_failed_and_bypassed_encoding_never_reuses_partial_bytes() {
    let mut state = pixels();
    prepare_rgb(&mut state);
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    state.kitty_image_id = kitty::other_image_id(state.kitty_image_id);
    let error = encode_kitty_upload_with(&mut state, &mut StepTimes::default(), |state| {
        state.upload.clear();
        state.upload.push_str("partial command");
        Err(io::Error::other("injected encoder failure"))
    }).unwrap_err();
    assert_eq!(error.to_string(), "injected encoder failure");
    assert!(state.encoding_key.is_none());
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    assert!(!state.upload.contains("partial command"));
    assert_eq!(decode_upload(&state.upload, true), *state.rgb.as_raw());
    state.rgb_version.invalidate();
    assert!(encode_kitty_upload(&mut state, &mut StepTimes::default()).is_err());
    assert!(state.encoding_key.is_none());
    state.rgb_version.publish(true);
    for group in [Group::Raster, Group::RasterAssets] {
        let mut config = CacheConfig::default();
        config.groups.insert(group, GroupPolicy { enabled: false, ..Default::default() });
        state.scene_cache.configure(&config);
        encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
        assert!(encode_kitty_upload_with(&mut state, &mut StepTimes::default(), |_| Err(io::Error::other("bypass reached encoder"))).is_err());
        assert!(state.encoding_key.is_none());
    }
}


#[cfg(feature = "memory-diagnostics")]
#[test]
fn reused_pixels_and_encoding_report_reuse_without_build_or_conversion_events() {
    let mut state = pixels();
    prepare_sky(&mut state);
    set_text(&mut state, "A");
    let completed_text = state.text.clone();
    let text_version = state.text_version;
    prepare_kitty_pixels(&mut state, (2, 2), &mut StepTimes::default()).unwrap();
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    state.text = completed_text;
    state.text_version = text_version;
    let mut times = StepTimes::with_trace(true);
    times.enable_memory_events(true);
    prepare_kitty_pixels(&mut state, (2, 2), &mut times).unwrap();
    encode_kitty_upload(&mut state, &mut times).unwrap();
    let trace = times.trace().unwrap();
    for buffer in [BufferId::RgbImage, BufferId::UploadBytes] {
        assert!(trace.steps.iter().flat_map(|step| &step.memory_events).any(|event| matches!(event.event,
            crate::timing::MemoryEvent::Operation { buffer: actual, operation: Operation::Reuse, .. } if actual == buffer)));
    }
    assert!(trace.steps.iter().flat_map(|step| &step.memory_events).all(|event| !matches!(event.event,
        crate::timing::MemoryEvent::Operation { operation: Operation::Build | Operation::Clear | Operation::Copy, .. })));
}

#[test]
fn compressor_is_lazy_and_each_warm_encode_starts_a_new_stream() {
    let mut state = pixels();
    state.rgb = image::RgbImage::from_pixel(40, 20, image::Rgb([11, 22, 33]));
    state.rgb_version.publish(true);
    state.compression = CompressionSupport::Unsupported;
    encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
    assert!(state.compressor.is_none());
    for (width, height, color) in [(40, 20, [11, 22, 33]), (7, 3, [54, 21, 9]), (40, 20, [0, 0, 0])] {
        state.rgb = image::RgbImage::from_pixel(width, height, image::Rgb(color));
        state.rgb_version.publish(true);
        state.compression = CompressionSupport::Supported;
        encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
        assert_eq!(state.compressor.as_ref().unwrap().total_in(), state.rgb.len() as u64);
        assert_eq!(decode_upload(&state.upload, true), *state.rgb.as_raw());
        state.compression = CompressionSupport::Unsupported;
        encode_kitty_upload(&mut state, &mut StepTimes::default()).unwrap();
        assert!(state.compressor.is_some()); // retained working memory remains available for the next compressed image
        assert_eq!(decode_upload(&state.upload, false), *state.rgb.as_raw());
    }
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn changed_images_report_engine_build_once_then_working_memory_reuse() {
    let mut state = pixels();
    state.rgb = image::RgbImage::from_pixel(12, 8, image::Rgb([1, 2, 3]));
    for expected in [Operation::Build, Operation::Reuse, Operation::Reuse] {
        state.rgb_version.publish(true); // force real compression, not an encoded-result hit
        let mut times = StepTimes::with_trace(true); times.enable_memory_events(true);
        encode_kitty_upload(&mut state, &mut times).unwrap();
        let events: Vec<_> = times.trace().unwrap().steps.iter().flat_map(|step| &step.memory_events).filter_map(|event| {
            if let crate::timing::MemoryEvent::Operation { buffer: BufferId::CompressionEngine, operation, .. } = event.event { Some(operation) } else { None }
        }).collect();
        assert_eq!(events, [expected]);
    }
}
