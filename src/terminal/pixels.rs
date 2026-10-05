//! Pixel renderer lifetime, protocol negotiation, physical sizing and presentation. Rasterization and text layout
//! are pure helpers; the simulation/observation/projection pipeline is shared with the character renderer.
mod detection;
mod text;
use crate::model::config::TerminalSettings;
use super::{TerminalSession, fit_square_viewport, open_terminal_session};
use super::graphics::{clear_image, compose_halfblocks, compose_image, encode_image, kitty, present_frame, serialize_frame_into};
use crate::astro::{Observer, SimulationClock};
use crate::metadata::{append_step_time_fields, fill_metadata_fields};
use crate::model::metadata::MetadataField;
use crate::model::projection::{ProjectedSky, ProjectionViewport as Viewport, View};
use crate::model::rendering::RenderOptions;
use crate::timing::StepTimes;
use crate::state::rendering::CompressionSupport;
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::{
    FontSize,
    picker::{Picker, ProtocolType},
};
use std::io;
use crate::{timing::memory::{Access, BufferId, BufferShape, IndexDomain, Operation} };


use crate::state::rendering::PixelState;

pub fn open_pixel_renderer(
    options: RenderOptions,
    settings: TerminalSettings,
    forced: Option<ProtocolType>,
    text_scale: f64,
) -> io::Result<(TerminalSession, PixelState)> {
    if !text_scale.is_finite() || !(0.25..=4.0).contains(&text_scale) {
        return Err(io::Error::other("text scale must be finite and between 0.25 and 4"));
    }
    let mut session = open_terminal_session()?;
    session.configure_graphics(false, false); // cleanup also covers a failed startup or capability query
    let picker = Picker::halfblocks();
    let tmux = picker.tmux_detected();
    let (protocol, font, compression) = detection::detect_protocol(forced, tmux, session.output())?;
    let font = font.unwrap_or(picker.font_size());
    session.configure_graphics(protocol == ProtocolType::Kitty, tmux);
    let mut renderer = PixelState {
        scene_cache: Default::default(),
        cache_diagnostics: Default::default(),
        reuse_assets: true,
        protocol,
        compression,
        kitty_image_id: kitty::IMAGE_IDS[0],
        font,
        tmux,
        screen: Rect::default(),
        area: Rect::default(),
        viewport: Viewport { width: 1, height: 1 },
        options,
        settings,
        time_zone: None,
        text_scale,
        frame_image: None,
        sky_image: image::RgbaImage::new(0, 0),
        rgb: image::RgbImage::new(0, 0),
        fields: Vec::new(),
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
            Some(crate::scene::raster_text::create_text_rasterizer().map_err(io::Error::other)?)
        },
    };
    fit_pixel_terminal(&mut renderer, &mut session)?;
    if protocol != ProtocolType::Kitty {
        let test = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        encode_image(DynamicImage::ImageRgba8(test), Rect::new(0, 0, 1, 1), protocol, tmux)?;
    }
    Ok((session, renderer))
}

pub fn pixel_viewport(state: &PixelState) -> Viewport {
    state.viewport
}

pub fn fit_pixel_terminal(state: &mut PixelState, session: &mut TerminalSession) -> io::Result<()> {
    state.scene_cache.invalidate();
    state.kitty_image_id = kitty::IMAGE_IDS[0];
    let (columns, rows) = crossterm::terminal::size()?;
    if columns == 0 || rows == 0 {
        return Err(io::Error::other("terminal has no drawable area"));
    }
    if let Ok(size) = crossterm::terminal::window_size()
        && size.width >= columns
        && size.height >= rows
    {
        state.font = FontSize::new(size.width / columns, size.height / rows);
    }
    state.font.width = state.font.width.max(1);
    state.font.height = state.font.height.max(1);
    state.screen = Rect::new(0, 0, columns, rows);
    let ratio = state
        .settings
        .aspect_ratio
        .unwrap_or(f64::from(state.font.height) / f64::from(state.font.width));
    let layout = fit_square_viewport(rows, columns, ratio);
    let (width, height) = (layout.width.max(1) as u16, layout.height.max(1) as u16);
    // floor the centering offset: encoders clear one row past the image before returning to its origin
    state.area = Rect::new((columns - width) / 2, (rows - height) / 2, width, height);
    state.viewport = Viewport {
        width: usize::from(state.area.width) * usize::from(state.font.width),
        height: usize::from(state.area.height) * usize::from(state.font.height),
    };
    if state.protocol != ProtocolType::Halfblocks {
        validate_frame_size(state.screen, state.font)?;
    } else if state
        .viewport
        .width
        .checked_mul(state.viewport.height)
        .is_none_or(|size| size > 16_777_216)
    {
        return Err(io::Error::other("terminal image exceeds 16 megapixels"));
    }
    clear_image(session.output(), state.protocol == ProtocolType::Kitty, state.tmux)
}

#[allow(clippy::too_many_arguments)]
pub fn render_pixel_frame(
    state: &mut PixelState,
    session: &mut TerminalSession,
    sky: &ProjectedSky<'_>,
    view: &View,
    date: f64,
    clock: &SimulationClock,
    observer: &Observer,
    times: &mut StepTimes,
) -> io::Result<()> {
    // resolve site metadata and rasterize the already projected sky
    if state.settings.metadata_panel && state.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
        state.time_zone = Some((*observer, crate::metadata::resolve_observer_timezone(observer)));
    }
    state.frame_image = times.measure("Frame canvas", || {
        if state.protocol == ProtocolType::Halfblocks {
            return Ok(None);
        }
        let (width, height) = validate_frame_size(state.screen, state.font)?;
        Ok::<_, io::Error>(Some(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba(crate::scene::pixels::BACKGROUND),
        )))
    })?;
    if let Some(frame) = &state.frame_image { times.record_shape(BufferId::FrameImage, Operation::Build, None, || BufferShape::vector(frame.as_raw(), IndexDomain::Bytes)); } // retained result is freshly allocated each frame
    times.describe("Frame canvas", || {
        format!(
            "protocol={:?}; screen={}x{} cells; font={}x{} pixels; allocated RGBA bytes={}",
            state.protocol,
            state.screen.width,
            state.screen.height,
            state.font.width,
            state.font.height,
            state.frame_image.as_ref().map_or(0, |frame| frame.len())
        )
    });
    state.sky_image = times
        .measure_steps("Raster", |times| {
            crate::scene::cached::draw_pixels(&mut state.scene_cache, sky, &state.options, crate::model::simulation::FrameTime::from_utc(date).tt, times)
        })
        .ok_or_else(|| io::Error::other("cannot allocate terminal image"))?;
    times.describe("Raster", || {
        format!(
            "output sky={}x{} pixels; RGBA bytes={}; cache={:?}",
            state.sky_image.width(),
            state.sky_image.height(),
            state.sky_image.len(),
            state.scene_cache.stats()
        )
    });

    // place the sky on the full frame before preparing labels, metadata and notices
    if let Some(frame) = &mut state.frame_image {
        times.measure("Sky composition", || {
            image::imageops::replace(
                frame,
                &state.sky_image,
                i64::from(state.area.x) * i64::from(state.font.width),
                i64::from(state.area.y) * i64::from(state.font.height),
            )
        });
        {
            times.record_borrow(BufferId::SkyImage, Access::ReadOnly, || BufferShape::vector(state.sky_image.as_raw(), IndexDomain::Bytes));
            times.record_borrow(BufferId::FrameImage, Access::Writable, || BufferShape::vector(frame.as_raw(), IndexDomain::Bytes));
            times.record_unknown(BufferId::FrameImage, Operation::Copy); // image::replace clips; no second pixel walk to count copied bytes
        }
        times.describe("Sky composition", || {
            format!(
                "source sky={}x{} pixels; target={}x{} pixels; output RGBA bytes={}",
                state.sky_image.width(),
                state.sky_image.height(),
                frame.width(),
                frame.height(),
                frame.len()
            )
        });
    }

    // prepare one shared text layout for raster text or native half-block text
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
            let (width, height) = state.frame_image
                .as_ref()
                .map_or((state.viewport.width, state.viewport.height), |frame| {
                    (frame.width() as usize, frame.height() as usize)
                });
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
    super::memory::record_field_rebuild(times, BufferId::MetadataFields, fields_before, &state.fields);
    let notice = (state.protocol == ProtocolType::Halfblocks)
        .then_some("Pixel renderer: half-block output (no graphics protocol selected).");
    let (text_screen, text_area, text_cell) = if state.protocol == ProtocolType::Halfblocks {
        (state.screen, state.area, (state.font.width, state.font.height))
    } else {
        compute_text_layout(state.screen, state.area, state.font, state.text_scale)
    };
    state.text = times.measure_steps("Text layout", |times| {
        text::compose_text(
            sky,
            &state.options,
            text_screen,
            text_area,
            &state.fields,
            notice,
            times,
            state.scene_cache.prepared(),
            state.scene_cache.named_candidates(),
        )
    });

    if let Some(text) = &mut state.raster_text {
        times.measure_memory_scope("Glyph frame setup", |_| crate::scene::raster_text::begin_text_frame(text, state.reuse_assets));
        {
            times.record_borrow(BufferId::GlyphMasks, Access::Writable, || BufferShape::unknown(IndexDomain::Glyphs));
            if !state.reuse_assets { times.record_unknown(BufferId::GlyphMasks, Operation::Clear); }
        }
    }

    // complete the bitmap before encoding it; half-blocks instead merge text into their final cell buffer
    state.composed = if let Some(frame) = state.frame_image.as_mut() {
        crate::scene::raster_text::paint_text_buffer_with_times(state.raster_text.as_mut().expect("graphics font initialized"), frame, &state.text, text_cell, times);
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
        if state.protocol == ProtocolType::Kitty {
            return present_kitty_frame(state, session, times);
        }
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
        state.encoded = Some(times.measure("Image encoding", || {
            encode_image(DynamicImage::ImageRgba8(std::mem::take(&mut state.sky_image)), state.area, state.protocol, state.tmux)
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

fn present_kitty_frame(state: &mut PixelState, session: &mut TerminalSession, times: &mut StepTimes) -> io::Result<()> {
    // all layers have already been blended onto the opaque background
    state.rgb = times.measure("Pixel conversion", || DynamicImage::ImageRgba8(state.frame_image.take().expect("graphics frame initialized")).into_rgb8());
    {
        times.record_unknown(BufferId::FrameImage, Operation::Move); // image conversion consumes RGBA ownership
        times.record_shape(BufferId::RgbImage, Operation::Build, None, || BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes)); // new RGB result, not reuse
    }
    times.describe("Pixel conversion", || {
        format!(
            "input RGBA bytes={}; output RGB bytes={}; dimensions={}x{}",
            state.rgb.width() as usize * state.rgb.height() as usize * 4,
            state.rgb.len(),
            state.rgb.width(),
            state.rgb.height()
        )
    });
    let transport_before = times.inspect_memory(|| (describe_upload(state), BufferShape::vector(&state.compressed, IndexDomain::Bytes)));
    times.measure("Image encoding", || {
        kitty::encode_upload_into(
            &state.rgb,
            state.kitty_image_id,
            state.compression == CompressionSupport::Supported,
            state.tmux,
            &mut state.compressed,
            &mut state.upload,
        )
    })?;
    {
        times.record_borrow(BufferId::RgbImage, Access::ReadOnly, || BufferShape::vector(state.rgb.as_raw(), IndexDomain::Bytes));
        times.record_shape(BufferId::UploadBytes, Operation::Clear, transport_before.map(|s| s.0), || { let mut shape = transport_before.unwrap().0; shape.len = Some(0); shape });
        times.record_shape(BufferId::CompressedBytes, Operation::Clear, transport_before.map(|s| s.1), || { let mut shape = transport_before.unwrap().1; shape.len = Some(0); shape });
        times.record_shape(BufferId::UploadBytes, Operation::Build, transport_before.map(|s| s.0), || describe_upload(state));
        if state.compression == CompressionSupport::Supported { times.record_shape(BufferId::CompressedBytes, Operation::Build, transport_before.map(|s| s.1), || BufferShape::vector(&state.compressed, IndexDomain::Bytes)); }
    }
    times.describe("Image encoding", || {
        format!(
            "input RGB bytes={}; compression={:?}; output protocol bytes={}; image ID={}; tmux={}",
            state.rgb.len(),
            state.compression,
            state.upload.len(),
            state.kitty_image_id,
            state.tmux
        )
    });
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

    // retain the front image during upload; synchronize only the completed image swap
    times.measure_steps("Present", |times| -> io::Result<()> {
        times.measure("Image upload", || present_frame(session.output(), state.upload.as_bytes()))?;
        times.record_shape(BufferId::UploadBytes, Operation::Output, None, || describe_upload(state)); // completed write_all/flush, not display completion
        times.describe("Image upload", || {
            format!(
                "bytes written/flushed={}; upload outside synchronized output",
                state.upload.len()
            )
        });
        times.measure("Image swap", || present_frame(session.output(), &state.serialized))?;
        times.record_shape(BufferId::SerializedBytes, Operation::Output, None, || BufferShape::vector(&state.serialized, IndexDomain::Bytes));
        times.describe("Image swap", || {
            format!(
                "bytes written/flushed={}; place completed image then delete prior image; frames=1",
                state.serialized.len()
            )
        });
        Ok(())
    })?;
    times.describe("Present", || {
        format!("Kitty total protocol bytes={}; frames=1", state.upload.len() + state.serialized.len())
    });
    state.kitty_image_id = kitty::other_image_id(state.kitty_image_id);
    Ok(())
}

/// Give raster text its own grid, so glyph size, line spacing and panel extent scale together. Sky pixels and
/// terminal image protocol dimensions continue to use the physical cell size.
fn compute_text_layout(screen: Rect, area: Rect, font: FontSize, scale: f64) -> (Rect, Rect, (u16, u16)) {
    let width = (f64::from(font.width) * scale).round().clamp(1.0, f64::from(u16::MAX)) as u16;
    let height = (f64::from(font.height) * scale).round().clamp(1.0, f64::from(u16::MAX)) as u16;
    let scale_rect = |rect: Rect| {
        let left = u32::from(rect.x) * u32::from(font.width) / u32::from(width);
        let top = u32::from(rect.y) * u32::from(font.height) / u32::from(height);
        let right = (u32::from(rect.right()) * u32::from(font.width)).div_ceil(u32::from(width));
        let bottom = (u32::from(rect.bottom()) * u32::from(font.height)).div_ceil(u32::from(height));
        Rect::new(
            left.min(65535) as u16,
            top.min(65535) as u16,
            (right - left).min(65535) as u16,
            (bottom - top).min(65535) as u16,
        )
    };
    let text_screen = scale_rect(screen);
    (text_screen, scale_rect(area).intersection(text_screen), (width, height))
}

/// Include margins and metadata in the allocation budget, not only the square sky viewport.
fn validate_frame_size(screen: Rect, font: FontSize) -> io::Result<(u32, u32)> {
    let (width, height) = (
        u32::from(screen.width) * u32::from(font.width),
        u32::from(screen.height) * u32::from(font.height),
    );
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 {
        return Err(io::Error::other(
            "terminal image exceeds 16 megapixels or has zero size",
        ));
    }
    Ok((width, height))
}

fn describe_upload(state: &PixelState) -> BufferShape {
    let mut shape = BufferShape::slice(state.upload.as_bytes(), IndexDomain::Bytes);
    shape.capacity = Some(state.upload.capacity());
    shape
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_changes_spacing_and_capacity_without_changing_sky_pixels() {
        let screen = Rect::new(0, 0, 100, 40);
        let sky = Rect::new(10, 0, 80, 40);
        let font = FontSize::new(10, 20);
        assert_eq!(compute_text_layout(screen, sky, font, 1.0), (screen, sky, (10, 20)));
        let (small, _, cell) = compute_text_layout(screen, sky, font, 0.85);
        assert_eq!(cell, (9, 17));
        assert!(small.width > screen.width && small.height > screen.height);
        let (large, _, cell) = compute_text_layout(screen, sky, font, 2.0);
        assert_eq!(cell, (20, 40));
        assert_eq!(large, Rect::new(0, 0, 50, 20));
    }

    #[test]
    fn tiny_cells_and_partial_text_rows_stay_nonzero_and_clipped() {
        let (screen, area, cell) =
            compute_text_layout(Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1), FontSize::new(1, 1), 0.25);
        assert_eq!(cell, (1, 1));
        assert_eq!(screen, area);
        assert_eq!(screen.width, 1);
    }
}
