//! Pixel renderer lifetime, protocol negotiation, physical sizing and presentation. Rasterization and text layout
//! are pure helpers; the simulation/observation/projection pipeline is shared with the character renderer.
mod detection;
mod text;
use super::{
    TerminalSession, TerminalSettings, fit_square_viewport,
    graphics::{clear_image, compose_halfblocks, compose_image, encode_image, present_frame, serialize_frame},
    open_terminal_session,
};
use crate::{
    astro::{Observer, SimulationClock},
    metadata::{MetadataField, ObserverTimeZone, collect_metadata_fields, format_step_time_fields},
    projection::{ProjectedSky, View, Viewport},
    scene::{RenderOptions, raster_text::TextRasterizer},
    timing::StepTimes,
};
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::{
    FontSize,
    picker::{Picker, ProtocolType},
};
use std::io;

pub struct PixelRenderer {
    pub(super) scene_cache: crate::scene::cached::SceneCache,
    pub(super) cache_diagnostics: [String; 2],
    pub(super) reuse_assets: bool,
    session: TerminalSession,
    protocol: ProtocolType,
    font: FontSize,
    tmux: bool,
    screen: Rect,
    area: Rect,
    viewport: Viewport,
    options: RenderOptions,
    settings: TerminalSettings,
    time_zone: Option<(Observer, ObserverTimeZone)>,
    raster_text: Option<TextRasterizer>,
    text_scale: f64,
}

impl PixelRenderer {
    pub fn open(
        options: RenderOptions,
        settings: TerminalSettings,
        forced: Option<ProtocolType>,
        text_scale: f64,
    ) -> io::Result<Self> {
        if !text_scale.is_finite() || !(0.25..=4.0).contains(&text_scale) {
            return Err(io::Error::other("text scale must be finite and between 0.25 and 4"));
        }
        let mut session = open_terminal_session()?;
        session.configure_graphics(false, false); // cleanup also covers a failed startup or capability query
        let picker = Picker::halfblocks();
        let tmux = picker.tmux_detected();
        let (protocol, font) = match forced {
            Some(protocol) => (protocol, picker.font_size()),
            None => {
                let (protocol, font) = detection::detect_protocol(tmux, session.output())?;
                (protocol, font.unwrap_or(picker.font_size()))
            }
        };
        session.configure_graphics(protocol == ProtocolType::Kitty, tmux);
        let mut renderer = Self {
            scene_cache: Default::default(),
            cache_diagnostics: Default::default(),
            reuse_assets: true,
            session,
            protocol,
            font,
            tmux,
            screen: Rect::default(),
            area: Rect::default(),
            viewport: Viewport { width: 1, height: 1 },
            options,
            settings,
            time_zone: None,
            text_scale,
            raster_text: if protocol == ProtocolType::Halfblocks {
                None
            } else {
                Some(TextRasterizer::new().map_err(io::Error::other)?)
            },
        };
        renderer.fit_to_terminal()?;
        let test = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        encode_image(DynamicImage::ImageRgba8(test), Rect::new(0, 0, 1, 1), protocol, tmux)?;
        Ok(renderer)
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn fit_to_terminal(&mut self) -> io::Result<()> {
        self.scene_cache.invalidate();
        let (columns, rows) = crossterm::terminal::size()?;
        if columns == 0 || rows == 0 {
            return Err(io::Error::other("terminal has no drawable area"));
        }
        if let Ok(size) = crossterm::terminal::window_size()
            && size.width >= columns
            && size.height >= rows
        {
            self.font = FontSize::new(size.width / columns, size.height / rows);
        }
        self.font.width = self.font.width.max(1);
        self.font.height = self.font.height.max(1);
        self.screen = Rect::new(0, 0, columns, rows);
        let ratio = self
            .settings
            .aspect_ratio
            .unwrap_or(f64::from(self.font.height) / f64::from(self.font.width));
        let layout = fit_square_viewport(rows, columns, ratio);
        let (width, height) = (layout.width.max(1) as u16, layout.height.max(1) as u16);
        // floor the centering offset: encoders clear one row past the image before returning to its origin
        self.area = Rect::new((columns - width) / 2, (rows - height) / 2, width, height);
        self.viewport = Viewport {
            width: usize::from(self.area.width) * usize::from(self.font.width),
            height: usize::from(self.area.height) * usize::from(self.font.height),
        };
        if self.protocol != ProtocolType::Halfblocks {
            validate_frame_size(self.screen, self.font)?;
        } else if self
            .viewport
            .width
            .checked_mul(self.viewport.height)
            .is_none_or(|size| size > 16_777_216)
        {
            return Err(io::Error::other("terminal image exceeds 16 megapixels"));
        }
        clear_image(self.session.output(), self.protocol == ProtocolType::Kitty, self.tmux)
    }

    pub fn render_frame(
        &mut self,
        sky: &ProjectedSky<'_>,
        view: &View,
        date: f64,
        clock: &SimulationClock,
        observer: &Observer,
        times: &mut StepTimes,
    ) -> io::Result<()> {
        // resolve site metadata and rasterize the already projected sky
        if self.settings.metadata_panel && self.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
            self.time_zone = Some((*observer, ObserverTimeZone::new(observer)));
        }
        let mut frame_image = times.measure("Frame canvas", || {
            if self.protocol == ProtocolType::Halfblocks {
                return Ok(None);
            }
            let (width, height) = validate_frame_size(self.screen, self.font)?;
            Ok::<_, io::Error>(Some(image::RgbaImage::from_pixel(
                width,
                height,
                image::Rgba(crate::scene::pixels::BACKGROUND),
            )))
        })?;
        let image = times
            .measure_steps("Raster", |times| {
                self.scene_cache
                    .draw_pixels(sky, &self.options, crate::sky::FrameTime::from_utc(date).tt, times)
            })
            .ok_or_else(|| io::Error::other("cannot allocate terminal image"))?;

        // place the sky on the full frame before preparing labels, metadata and notices
        if let Some(frame) = &mut frame_image {
            times.measure("Sky composition", || {
                image::imageops::replace(
                    frame,
                    &image,
                    i64::from(self.area.x) * i64::from(self.font.width),
                    i64::from(self.area.y) * i64::from(self.font.height),
                )
            });
        }

        // prepare one shared text layout for raster text or native half-block text
        let mut fields = if self.settings.metadata_panel {
            collect_metadata_fields(
                date,
                clock,
                sky.moon.phase,
                observer,
                view,
                true,
                &self.time_zone.as_ref().unwrap().1,
            )
        } else {
            Vec::new()
        };
        if self.settings.metadata_panel {
            let (width, height) = frame_image
                .as_ref()
                .map_or((self.viewport.width, self.viewport.height), |frame| {
                    (frame.width() as usize, frame.height() as usize)
                });
            fields.push(MetadataField {
                label: "Graphics".into(),
                value: format!("{:?} · {width}×{height}", self.protocol),
            });
        }
        if self.settings.frame_times {
            fields.push(MetadataField {
                label: "Obs cache".into(),
                value: self.cache_diagnostics[0].clone(),
            });
            fields.push(MetadataField {
                label: "Proj cache".into(),
                value: self.cache_diagnostics[1].clone(),
            });
            fields.push(MetadataField {
                label: "Raster cache".into(),
                value: crate::cache::format_stats(self.scene_cache.stats()),
            });
            fields.extend(format_step_time_fields(times.steps()));
        }
        let notice = (self.protocol == ProtocolType::Halfblocks)
            .then_some("Pixel renderer: half-block output (no graphics protocol selected).");
        let (text_screen, text_area, text_cell) = if self.protocol == ProtocolType::Halfblocks {
            (self.screen, self.area, (self.font.width, self.font.height))
        } else {
            compute_text_layout(self.screen, self.area, self.font, self.text_scale)
        };
        let text = times.measure_steps("Text layout", |times| {
            text::compose_text(sky, &self.options, text_screen, text_area, &fields, notice, times)
        });

        if let Some(text) = &mut self.raster_text {
            text.begin_frame(self.reuse_assets);
        }

        // complete the bitmap before encoding it; half-blocks instead merge text into their final cell buffer
        let buffer = if let Some(mut frame) = frame_image {
            times.measure("Text rasterization", || {
                self.raster_text
                    .as_mut()
                    .expect("graphics font initialized")
                    .paint_buffer(&mut frame, &text, text_cell)
            });
            let encoded = times.measure("Image encoding", || {
                encode_image(DynamicImage::ImageRgba8(frame), self.screen, self.protocol, self.tmux)
            })?;
            times.measure("Image composition", || {
                compose_image(&encoded, self.screen, self.screen)
            })
        } else {
            let encoded = times.measure("Image encoding", || {
                encode_image(DynamicImage::ImageRgba8(image), self.area, self.protocol, self.tmux)
            })?;
            times.measure("Cell composition", || {
                compose_halfblocks(&encoded, self.screen, self.area, &text)
            })
        };
        let frame = times.measure("Frame serialization", || serialize_frame(&buffer))?;
        times.measure("Present", || present_frame(self.session.output(), &frame))
    }
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
