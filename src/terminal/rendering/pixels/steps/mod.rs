//! Individual pixel-frame stages and their existing diagnostic boundaries.
mod presentation;
pub(in crate::terminal) use presentation::present_kitty_image;
mod composition;
pub(in crate::terminal) use composition::{prepare_kitty_pixels, encode_kitty_upload};
#[cfg(test)]
use composition::convert_kitty_pixels;
use crate::terminal::TerminalSession;
use crate::terminal::transport::graphics::{compose_halfblocks, compose_image, encode_image, kitty, present_frame, serialize_frame_into};
use crate::astro::{Observer, SimulationClock};
use crate::metadata::{append_step_time_fields, fill_metadata_fields};
use crate::model::{MetadataField, ProjectedSky, View};
use crate::timing::StepTimes;
#[cfg(test)]
use crate::state::CompressionSupport;
use image::DynamicImage;
use ratatui_image::picker::ProtocolType;
use std::io;
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, Operation};


use crate::state::PixelState;

use super::{detection, text, layout::{compute_text_layout, validate_frame_size}};

pub(in crate::terminal) fn prepare_pixel_timezone(state: &mut PixelState, observer: &Observer) {
    if state.settings.metadata_panel && state.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
        state.time_zone = Some((*observer, crate::metadata::resolve_observer_timezone(observer)));
    }
}

pub(in crate::terminal) fn initialize_pixel_canvas(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    let before = times.inspect_memory(|| state.frame_image.as_ref().map(|frame| BufferShape::vector(frame.as_raw(), IndexDomain::Bytes))).flatten();
    state.frame_image = times.measure("Frame canvas", || {
        if state.protocol == ProtocolType::Halfblocks {
            return Ok(None);
        }
        let (width, height) = validate_frame_size(state.screen, state.font)?;
        let mut bytes = state.frame_image.take().map_or_else(Vec::new, image::RgbaImage::into_raw);
        let length = width as usize * height as usize * 4;
        bytes.try_reserve(length.saturating_sub(bytes.len())).map_err(io::Error::other)?;
        bytes.resize(length, 0);
        for pixel in bytes.chunks_exact_mut(4) { pixel.copy_from_slice(&crate::constants::PIXEL_BACKGROUND_RGBA); }
        Ok::<_, io::Error>(Some(image::RgbaImage::from_raw(width, height, bytes).expect("validated RGBA length")))
    })?;
    if let Some(frame) = &state.frame_image { times.record_shape(BufferId::FrameImage, Operation::Build, before, || BufferShape::vector(frame.as_raw(), IndexDomain::Bytes)); } // reset retained pixels so text never accumulates
    times.describe("Frame canvas", || {
        format!(
            "protocol={:?}; screen={}x{} cells; font={}x{} pixels; initialized RGBA bytes={}",
            state.protocol,
            state.screen.width,
            state.screen.height,
            state.font.width,
            state.font.height,
            state.frame_image.as_ref().map_or(0, |frame| frame.len())
        )
    });
    Ok(())
}

pub(in crate::terminal) fn rasterize_pixel_sky(state: &mut PixelState, sky: &ProjectedSky<'_>, prepared: Option<&crate::model::RenderProjection<'_>>, date: f64, times: &mut StepTimes) -> io::Result<()> {
    times
        .measure_steps("Raster", |times| {
            let epoch = crate::model::FrameTime::from_utc(date).tt;
            match prepared {
                Some(projected) => crate::scene::draw_prepared_pixels(&mut state.scene_cache, projected, &state.options, epoch, times).map(|_| ()),
                None => crate::scene::draw_pixels(&mut state.scene_cache, sky, &state.options, epoch, times).map(|_| ()),
            }
        })
        .ok_or_else(|| io::Error::other("cannot allocate terminal image"))?;
    times.describe("Raster", || {
        format!(
            "output sky={}x{} pixels; RGBA bytes={}; cache={:?}",
            state.scene_cache.pixel_image().width(),
            state.scene_cache.pixel_image().height(),
            state.scene_cache.pixel_image().len(),
            state.scene_cache.stats()
        )
    });
    Ok(())
}

pub(in crate::terminal) fn compose_pixel_sky(state: &mut PixelState, times: &mut StepTimes) {
    if let Some(frame) = &mut state.frame_image {
        times.measure("Sky composition", || {
            compose_sky_image(
                state.scene_cache.pixel_image(),
                frame,
                i64::from(state.area.x) * i64::from(state.font.width),
                i64::from(state.area.y) * i64::from(state.font.height),
            )
        });
        {
            times.record_borrow(BufferId::PixelScene, Access::ReadOnly, || BufferShape::vector(state.scene_cache.pixel_image().as_raw(), IndexDomain::Bytes));
            times.record_borrow(BufferId::FrameImage, Access::Writable, || BufferShape::vector(frame.as_raw(), IndexDomain::Bytes));
            times.record_unknown(BufferId::FrameImage, Operation::Copy); // image::replace clips; no second pixel walk to count copied bytes
        }
        times.describe("Sky composition", || {
            format!(
                "source sky={}x{} pixels; target={}x{} pixels; output RGBA bytes={}",
                state.scene_cache.pixel_image().width(),
                state.scene_cache.pixel_image().height(),
                frame.width(),
                frame.height(),
                frame.len()
            )
        });
    }
}

fn compose_sky_image(sky: &image::RgbaImage, frame: &mut image::RgbaImage, x: i64, y: i64) {
    let target_x = x.clamp(0, i64::from(frame.width())) as usize;
    let target_y = y.clamp(0, i64::from(frame.height())) as usize;
    let source_x = x.saturating_neg().clamp(0, i64::from(sky.width())) as usize;
    let source_y = y.saturating_neg().clamp(0, i64::from(sky.height())) as usize;
    let width = (frame.width() as usize - target_x).min(sky.width() as usize - source_x);
    let height = (frame.height() as usize - target_y).min(sky.height() as usize - source_y);
    if width == 0 || height == 0 { return; }

    let source_stride = sky.width() as usize * 4;
    let target_stride = frame.width() as usize * 4;
    let row_bytes = width * 4;
    let source = sky.as_raw();
    let target = frame.as_mut();
    for row in 0..height {
        let source_start = (source_y + row) * source_stride + source_x * 4;
        let target_start = (target_y + row) * target_stride + target_x * 4;
        target[target_start..target_start + row_bytes].copy_from_slice(&source[source_start..source_start + row_bytes]); // copy the clipped row, including its unchanged alpha
    }
}

pub(in crate::terminal) fn prepare_pixel_fields(state: &mut PixelState, sky: &ProjectedSky<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) {
    let fields_before = times.inspect_memory(|| BufferShape::vector(&state.fields, IndexDomain::Objects));
    times.measure_memory_scope("Metadata fields", |times| {
        state.fields.clear();
        if state.settings.metadata_panel {
            fill_metadata_fields(
                &mut state.fields,
                date,
                clock,
                sky.moon.phase,
                observer,
                view,
                true,
                &state.time_zone.as_ref().unwrap().1,
            );
        }
        if state.settings.metadata_panel {
            let (width, height) = if state.protocol == ProtocolType::Halfblocks {
                (state.viewport.width, state.viewport.height)
            } else {
                (usize::from(state.screen.width) * usize::from(state.font.width), usize::from(state.screen.height) * usize::from(state.font.height))
            };
            state.fields.push(MetadataField {
                label: "Graphics".into(),
                value: format!("{:?} · {width}×{height}", state.protocol),
            });
            if state.protocol == ProtocolType::Kitty {
                state.fields.push(MetadataField {
                    label: "Compression".into(),
                    value: detection::describe_compression(state.compression).into(),
                });
            }
        }
        if state.settings.frame_times {
            state.fields.push(MetadataField {
                label: "Correction skips".into(),
                value: sky.correction_stats.skipped.to_string(),
            });
            state.fields.push(MetadataField {
                label: "Endpoint only".into(),
                value: sky.correction_stats.endpoint_only.to_string(),
            });
            state.fields.push(MetadataField {
                label: "Obs cache".into(),
                value: state.cache_diagnostics[0].clone(),
            });
            state.fields.push(MetadataField {
                label: "Proj cache".into(),
                value: state.cache_diagnostics[1].clone(),
            });
            state.fields.push(MetadataField {
                label: "Raster cache".into(),
                value: crate::cache::format_stats(state.scene_cache.stats()),
            });
            append_step_time_fields(&mut state.fields, times.steps());
        }
    });
    crate::terminal::diagnostics::record_field_rebuild(times, BufferId::MetadataFields, fields_before, &state.fields);
}

pub(in crate::terminal) fn layout_pixel_text(state: &mut PixelState, sky: &ProjectedSky<'_>, prepared: Option<&crate::model::RenderProjection<'_>>, times: &mut StepTimes) -> (u16, u16) {
    let notice = (state.protocol == ProtocolType::Halfblocks)
        .then_some("Pixel renderer: half-block output (no graphics protocol selected).");
    let (text_screen, text_area, text_cell) = if state.protocol == ProtocolType::Halfblocks {
        (state.screen, state.area, (state.font.width, state.font.height))
    } else {
        compute_text_layout(state.screen, state.area, state.font, state.text_scale)
    };
    times.measure_steps("Text layout", |times| {
        text::refresh_text(&mut state.text_cache, &mut state.text, &mut state.text_version, sky, prepared,
            &state.options, text_screen, text_area, &state.fields, notice, state.reuse_assets, times);
    });
    text_cell
}

pub(in crate::terminal) fn prepare_pixel_glyphs(state: &mut PixelState, times: &mut StepTimes) {
    if let Some(text) = &mut state.raster_text {
        times.measure_memory_scope("Glyph frame setup", |_| crate::scene::begin_text_frame(text, state.reuse_assets));
        {
            times.record_borrow(BufferId::GlyphMasks, Access::Writable, || BufferShape::unknown(IndexDomain::Glyphs));
            if !state.reuse_assets { times.record_unknown(BufferId::GlyphMasks, Operation::Clear); }
        }
    }
}

pub(in crate::terminal) fn paint_pixel_text(state: &mut PixelState, text_cell: (u16, u16), times: &mut StepTimes) {
    if let Some(frame) = state.frame_image.as_mut() {
        crate::scene::paint_text_buffer_with_times(state.raster_text.as_mut().expect("graphics font initialized"), frame, &state.text, text_cell, times);
        times.describe("Text rasterization", || {
            format!(
                "input text cells={}; nonblank cells={}; glyph cell={}x{} pixels; output RGBA bytes={}",
                state.text.content.len(),
                state.text.content.iter().filter(|c| !c.symbol().trim().is_empty()).count(),
                text_cell.0,
                text_cell.1,
                frame.len()
            )
        });
    }
}

pub(in crate::terminal) fn encode_pixel_cells(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    compose_encoded_pixel_cells(state, times)
}

fn compose_encoded_pixel_cells(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    state.composed = if state.frame_image.is_some() {
        state.encoded = Some(times.measure("Image encoding", || {
            encode_image(DynamicImage::ImageRgba8(state.frame_image.take().expect("graphics frame initialized")), state.screen, state.protocol, state.tmux)
        })?);
        times.record_unknown(BufferId::EncodedImage, Operation::Build); // opaque protocol internals, fresh result retained for inspection
        times.describe("Image encoding", || {
            format!(
                "protocol={:?}; full image={}x{} pixels; includes sky and text",
                state.protocol,
                u32::from(state.screen.width) * u32::from(state.font.width),
                u32::from(state.screen.height) * u32::from(state.font.height)
            )
        });
        let buffer = times.measure("Image composition", || {
            compose_image(state.encoded.as_ref().expect("protocol encoded"), state.screen, state.screen)
        });
        times.record_shape(BufferId::ComposedCells, Operation::Build, None, || BufferShape::vector(&buffer.content, IndexDomain::Cells));
        times.describe("Image composition", || {
            format!(
                "output terminal buffer cells={}; image protocol payload carried in buffer",
                buffer.content.len()
            )
        });
        buffer
    } else {
        // ratatui-image Halfblocks::new requires ownership; only this protocol needs a scoped transfer copy.
        times.record_borrow(BufferId::PixelScene, Access::ReadOnly, || BufferShape::vector(state.scene_cache.pixel_image().as_raw(), IndexDomain::Bytes));
        times.record_shape(BufferId::HalfblockTransfer, Operation::Copy, None, || BufferShape::vector(state.scene_cache.pixel_image().as_raw(), IndexDomain::Bytes));
        state.encoded = Some(times.measure("Image encoding", || {
            encode_image(DynamicImage::ImageRgba8(state.scene_cache.pixel_image().clone()), state.area, state.protocol, state.tmux)
        })?);
        times.record_unknown(BufferId::EncodedImage, Operation::Build); // opaque protocol internals, fresh result retained for inspection
        times.describe("Image encoding", || {
            format!(
                "protocol=Halfblocks; input sky={}x{} pixels; target sky area={}x{} cells",
                state.viewport.width, state.viewport.height, state.area.width, state.area.height
            )
        });
        let buffer = times.measure("Cell composition", || {
            compose_halfblocks(state.encoded.as_ref().expect("protocol encoded"), state.screen, state.area, &state.text)
        });
        times.record_shape(BufferId::ComposedCells, Operation::Build, None, || BufferShape::vector(&buffer.content, IndexDomain::Cells));
        times.describe("Cell composition", || {
            format!(
                "input text cells={}; output terminal cells={}; text merged over halfblocks",
                state.text.content.len(),
                buffer.content.len()
            )
        });
        buffer
    };
    Ok(())
}

pub(in crate::terminal) fn serialize_pixel_cells(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    let serialized_before = times.inspect_memory(|| BufferShape::vector(&state.serialized, IndexDomain::Bytes));
    times.measure("Frame serialization", || serialize_frame_into(&state.composed, &mut state.serialization_blank, &mut state.serialized))?;
    {
        times.record_borrow(BufferId::ComposedCells, Access::ReadOnly, || BufferShape::vector(&state.composed.content, IndexDomain::Cells));
        times.record_shape(BufferId::SerializedBytes, Operation::Clear, serialized_before, || { let mut shape = serialized_before.unwrap(); shape.len = Some(0); shape });
        times.record_shape(BufferId::SerializedBytes, Operation::Build, serialized_before, || BufferShape::vector(&state.serialized, IndexDomain::Bytes));
        times.record_shape(BufferId::SerializationBlank, Operation::Clear, None, || BufferShape::vector(&state.serialization_blank.content, IndexDomain::Cells)); // reset values after possible resize
    }
    times.describe("Frame serialization", || {
        format!(
            "input terminal buffer={} cells; output bytes={}",
            state.composed.content.len(),
            state.serialized.len()
        )
    });
    Ok(())
}

pub(in crate::terminal) fn present_pixel_cells(state: &mut PixelState, session: &mut TerminalSession, times: &mut StepTimes) -> io::Result<()> {
    times.measure("Present", || present_frame(session.output(), &state.serialized))?;
    times.record_shape(BufferId::SerializedBytes, Operation::Output, None, || BufferShape::vector(&state.serialized, IndexDomain::Bytes)); // write_all and flush completed, not terminal display completion
    times.describe("Present", || {
        format!(
            "protocol={:?}; bytes submitted={}; frames=1",
            state.protocol,
            state.serialized.len()
        )
    });
    Ok(())
}

pub(in crate::terminal) fn serialize_kitty_swap(state: &mut PixelState, times: &mut StepTimes) -> io::Result<()> {
    let serialized_before = times.inspect_memory(|| BufferShape::vector(&state.serialized, IndexDomain::Bytes));
    times.measure("Frame serialization", || {
        kitty::serialize_swap_into(state.kitty_image_id, state.screen, state.tmux, &mut state.serialized)
    })?;

    {
        times.record_shape(BufferId::SerializedBytes, Operation::Clear, serialized_before, || { let mut shape = serialized_before.unwrap(); shape.len = Some(0); shape });
        times.record_shape(BufferId::SerializedBytes, Operation::Build, serialized_before, || BufferShape::vector(&state.serialized, IndexDomain::Bytes));
    }
    times.describe("Frame serialization", || {
        format!(
            "swap commands bytes={}; image upload already encoded separately",
            state.serialized.len()
        )
    });

    Ok(())
}

fn describe_upload(state: &PixelState) -> BufferShape {
    let mut shape = BufferShape::slice(state.upload.as_bytes(), IndexDomain::Bytes);
    shape.capacity = Some(state.upload.capacity());
    shape
}


#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use clap::Parser;
    use ratatui::layout::Rect;

    pub(super) fn pixels(protocol: ProtocolType) -> PixelState {
        let config = crate::cli::build_config(crate::cli::Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
        PixelState {
            scene_cache: Default::default(),
            cache_diagnostics: Default::default(),
            reuse_assets: true,
            protocol,
            compression: CompressionSupport::Supported,
            kitty_image_id: kitty::IMAGE_IDS[0],
            font: ratatui_image::FontSize { width: 2, height: 2 },
            tmux: false,
            screen: Rect::new(0, 0, 8, 4),
            area: Rect::new(0, 0, 8, 4),
            viewport: crate::model::ProjectionViewport { width: 16, height: 8 },
            options: config.render,
            settings: config.terminal,
            time_zone: None,
            text_scale: 1.0,
            frame_image: None,
            rgb: image::RgbImage::new(0, 0),
            rgb_version: Default::default(),
            frame_key: None,
            encoding_key: None,
            displayed_key: None,
            display_valid: false,
            fields: Vec::new(),
            text_cache: Default::default(),
            text_version: Default::default(),
            text: ratatui::buffer::Buffer::empty(Rect::default()),
            composed: ratatui::buffer::Buffer::empty(Rect::default()),
            upload: String::new(),
            compressed: Vec::new(),
            encoded: None,
            serialization_blank: ratatui::buffer::Buffer::empty(Rect::default()),
            serialized: Vec::new(),
            raster_text: if protocol == ProtocolType::Halfblocks {
                None
            } else {
                Some(crate::scene::create_text_rasterizer().unwrap())
            },
        }
    }

    #[test]
    fn sky_row_copy_matches_image_replace_for_clipping_and_extreme_offsets() {
        for (source_width, source_height, target_width, target_height) in [(5, 3, 8, 6), (8, 6, 5, 3), (5, 3, 5, 3), (0, 3, 5, 3), (5, 0, 0, 3), (5, 3, 5, 0)] {
            let source = image::RgbaImage::from_fn(source_width, source_height, |x, y| image::Rgba([x as u8, y as u8, (x + y) as u8, (x * 31 + y * 17) as u8]));
            let original = image::RgbaImage::from_fn(target_width, target_height, |x, y| image::Rgba([91, x as u8, y as u8, 37]));
            for x in [i64::MIN, -9, -5, -4, -1, 0, 1, 4, 5, 8, i64::MAX] {
                for y in [i64::MIN, -7, -3, -2, -1, 0, 1, 2, 3, 6, i64::MAX] {
                    let mut expected = original.clone();
                    image::imageops::replace(&mut expected, &source, x, y);
                    let mut actual = original.clone();
                    compose_sky_image(&source, &mut actual, x, y);
                    assert_eq!(actual, expected, "source={source_width}x{source_height}, target={target_width}x{target_height}, offset=({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn completed_text_and_kitty_rgb_are_retained_after_encoding() {
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel, ProtocolType::Iterm2, ProtocolType::Halfblocks] {
            let mut state = pixels(protocol);
            let sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
            let data = crate::projection::project_sky(&sky, &View::default(), state.viewport);
            let projected = data.view(&sky);
            let mut times = StepTimes::with_trace(true);
            let mut cached_pointer = None;
            for frame in 0..2 {
                initialize_pixel_canvas(&mut state, &mut times).unwrap();
                rasterize_pixel_sky(&mut state, &projected, None, 2451545.0, &mut times).unwrap();
                let expected = state.scene_cache.pixel_image().clone();
                let pointer = state.scene_cache.pixel_image().as_ptr();
                if let Some(previous) = cached_pointer { assert_eq!(pointer, previous, "paused sky reuses its original image"); }
                cached_pointer = Some(pointer);
                compose_pixel_sky(&mut state, &mut times);
                state.text = ratatui::buffer::Buffer::empty(state.screen);
                state.text[(0, 0)].set_symbol(if frame == 0 { "A" } else { "B" });
                paint_pixel_text(&mut state, (2, 2), &mut times);
                assert!(!state.text.content.is_empty()); // completed grid survives both graphics and half-block consumers
                if protocol == ProtocolType::Kitty {
                    convert_kitty_pixels(&mut state, &mut times).unwrap();
                    assert!(!state.rgb.is_empty());
                    encode_kitty_upload(&mut state, &mut times).unwrap();
                    assert!(!state.rgb.is_empty()); // completed RGB is retained for encoding reuse
                    assert!(!state.upload.is_empty());
                    assert!(state.compressed.capacity() > 0);
                } else {
                    encode_pixel_cells(&mut state, &mut times).unwrap();
                    assert!(!state.text.content.is_empty());
                    assert!(!state.composed.content.is_empty());
                }
                assert_eq!(state.scene_cache.pixel_image(), &expected, "text/composition must never modify the cached sky");
            }
            assert_eq!(state.scene_cache.stats().hits, 1);
        }
    }

    #[test]
    fn failed_encoding_keeps_completed_text_and_preserves_the_error() {
        let mut state = pixels(ProtocolType::Kitty);
        state.frame_image = Some(image::RgbaImage::new(2, 2));
        state.text = ratatui::buffer::Buffer::empty(state.screen);
        let error = encode_pixel_cells(&mut state, &mut StepTimes::default()).unwrap_err();
        assert_eq!(error.to_string(), "Kitty output requires the RGB upload/swap pipeline");
        assert!(!state.text.content.is_empty());
    }
}
