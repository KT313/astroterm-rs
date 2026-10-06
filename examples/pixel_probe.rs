//! Headless phase-7 prototype: raster quality, protocol encoding and image/text composition cost.
use astroterm::terminal::{
    KITTY_IMAGE_IDS, encode_kitty_upload, serialize_kitty_swap, compose_halfblocks, compose_image, encode_image,
    present_frame, serialize_frame,
};
use image::{DynamicImage, RgbaImage};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Widget},
};
use ratatui_image::picker::ProtocolType;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut pixmap = tiny_skia::Pixmap::new(1000, 600).unwrap();
    pixmap.fill(tiny_skia::Color::from_rgba8(3, 6, 14, 255));
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(180, 205, 255, 255);
    for i in 0..2000 {
        let (x, y) = ((i * 7919 % 1000) as f32, (i * 5381 % 600) as f32);
        let circle = tiny_skia::PathBuilder::from_circle(x, y, 0.5 + (i % 5) as f32 * 0.5).unwrap();
        pixmap.fill_path(
            &circle,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    eprintln!("raster_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    let image = RgbaImage::from_raw(1000, 600, pixmap.take()).unwrap();
    if let Some(path) = std::env::args().nth(1) {
        image.save(path)?;
    }
    let area = Rect::new(0, 0, 100, 30);
    let mut text = Buffer::empty(area);
    Paragraph::new("Pixel prototype — text over image")
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .render(Rect::new(0, 0, 40, 1), &mut text);
    let mut raster_text = astroterm::scene::create_text_rasterizer()?;
    for protocol in [
        ProtocolType::Kitty,
        ProtocolType::Sixel,
        ProtocolType::Iterm2,
        ProtocolType::Halfblocks,
    ] {
        let start = Instant::now();
        let mut frame_image = image.clone();
        if protocol != ProtocolType::Halfblocks {
            astroterm::scene::paint_text_buffer(&mut raster_text, &mut frame_image, &text, (10, 20));
        }
        if protocol == ProtocolType::Kitty {
            let rgb = DynamicImage::ImageRgba8(frame_image).into_rgb8();
            let encoded = encode_kitty_upload(&rgb, KITTY_IMAGE_IDS[0], true, false)?;
            let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let mut out = Vec::new();
            present_frame(&mut out, &encoded)?;
            present_frame(&mut out, &serialize_kitty_swap(KITTY_IMAGE_IDS[0], area, false)?)?;
            eprintln!(
                "{protocol:?}: encode_ms={encode_ms:.3}, compose_write_memory_ms={:.3}, bytes={}",
                start.elapsed().as_secs_f64() * 1000.0,
                out.len()
            );
            continue;
        }
        let encoded = encode_image(DynamicImage::ImageRgba8(frame_image), area, protocol, false)?;
        let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let buffer = if protocol == ProtocolType::Halfblocks {
            compose_halfblocks(&encoded, area, area, &text)
        } else {
            compose_image(&encoded, area, area)
        };
        let mut out = Vec::new();
        present_frame(&mut out, &serialize_frame(&buffer)?)?;
        eprintln!(
            "{protocol:?}: encode_ms={encode_ms:.3}, compose_write_memory_ms={:.3}, bytes={}",
            start.elapsed().as_secs_f64() * 1000.0,
            out.len()
        );
    }
    Ok(())
}
