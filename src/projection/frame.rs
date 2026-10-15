//! Screen layout and reusable star, overlay and text buffers for the frame pipeline.
use crate::model::Texel;
use crate::text::{TextRasterizer, create_text_rasterizer, set_text_cell_size};

/// The screen-sized buffers of a frame: the star layer step 5 fills, the overlay step 6 draws on top of it, the
/// text rasterizer the overlay's labels use, and their screen layout.
pub struct Frame {
    width: usize,
    height: usize,
    pub drawing_area_x_offset: usize,       // left edge of the square drawing area in full-frame pixels
    pub drawing_area_y_offset: usize,       // top edge of the square drawing area in full-frame pixels
    pub drawing_area_width_and_height: usize, // side length of the square used by star projection
    stars: Vec<Texel>,                      // row-major, width × height: light per pixel before tone mapping
    overlay: Vec<u8>,                       // premultiplied RGBA, transparent where nothing was drawn: points, lines and text over the stars
    text: TextRasterizer,
    text_cell: (u16, u16),                  // pixel size of one text cell: glyph size and line spacing
}

impl Frame {
    /// A frame with a centered drawing area of the given side length and text cells of `text_cell` pixels.
    pub fn new(width: usize, height: usize, drawing_area_width_and_height: usize, text_cell: (u16, u16)) -> Self {
        assert!(drawing_area_width_and_height <= width.min(height), "drawing area must fit inside the frame");
        let drawing_area_x_offset = (width - drawing_area_width_and_height) / 2;
        let drawing_area_y_offset = (height - drawing_area_width_and_height) / 2;
        let mut text = create_text_rasterizer().expect("the bundled font parses");
        set_text_cell_size(&mut text, text_cell.0, text_cell.1);
        Self {
            width,
            height,
            drawing_area_x_offset,
            drawing_area_y_offset,
            drawing_area_width_and_height,
            stars: vec![Texel::default(); width * height],
            overlay: vec![0; width * height * 4],
            text,
            text_cell,
        }
    }

    /// A square frame that is all sky, with the default text cell: the shape of the tests and the parity fixtures.
    pub fn square(size: usize) -> Self { Self::new(size, size, size, (10, 20)) }

    /// Adopt a new size and sky square, clearing both image layers.
    pub fn resize(&mut self, width: usize, height: usize, drawing_area_width_and_height: usize) {
        assert!(drawing_area_width_and_height <= width.min(height), "drawing area must fit inside the frame");
        let drawing_area_x_offset = (width - drawing_area_width_and_height) / 2;
        let drawing_area_y_offset = (height - drawing_area_width_and_height) / 2;
        if width == self.width && height == self.height
            && drawing_area_width_and_height == self.drawing_area_width_and_height
            && drawing_area_x_offset == self.drawing_area_x_offset
            && drawing_area_y_offset == self.drawing_area_y_offset {
            return;
        }
        self.width = width;
        self.height = height;
        self.drawing_area_width_and_height = drawing_area_width_and_height;
        self.drawing_area_x_offset = drawing_area_x_offset;
        self.drawing_area_y_offset = drawing_area_y_offset;
        self.stars.clear();
        self.stars.resize(width * height, Texel::default());
        self.overlay.clear();
        self.overlay.resize(width * height * 4, 0);
    }

    pub fn set_text_cell(&mut self, text_cell: (u16, u16)) {
        self.text_cell = text_cell;
        set_text_cell_size(&mut self.text, text_cell.0, text_cell.1);
    }

    pub fn width(&self) -> usize { self.width }
    pub fn height(&self) -> usize { self.height }

    /// The square viewport used to place sky objects; offsets are applied only when drawing into this frame.
    pub fn drawing_area_viewport(&self) -> crate::model::ProjectionViewport {
        crate::model::ProjectionViewport { width: self.drawing_area_width_and_height, height: self.drawing_area_width_and_height }
    }

    /// Translate a drawing-area (row, column) into the full frame for sky graphics and labels.
    pub fn drawing_area_pixel(&self, (row, column): (i32, i32)) -> (i32, i32) {
        (row + self.drawing_area_y_offset as i32, column + self.drawing_area_x_offset as i32)
    }

    /// Replace the star layer with a projected square, leaving the overlay untouched and margins empty.
    pub fn insert_projected_stars(&mut self, projected_stars: &[Texel]) {
        // Validate the square and its placement before changing any pixels.
        let size = self.drawing_area_width_and_height;
        assert_eq!(Some(projected_stars.len()), size.checked_mul(size), "projected stars must fill the drawing area");
        assert!(self.drawing_area_x_offset.checked_add(size).is_some_and(|right| right <= self.width)
            && self.drawing_area_y_offset.checked_add(size).is_some_and(|bottom| bottom <= self.height),
            "drawing area must fit inside the frame");

        // Clear stars from the previous frame, including any constellation glyphs and old margins.
        self.stars.fill(Texel::default());
        if size == 0 { return; }

        // Copy each square row into the corresponding row of the full-size frame.
        for (row, source) in projected_stars.chunks_exact(size).enumerate() {
            let start = (self.drawing_area_y_offset + row) * self.width + self.drawing_area_x_offset;
            self.stars[start..start + size].copy_from_slice(source);
        }
    }

    pub fn text_cell(&self) -> (u16, u16) { self.text_cell }
    pub fn stars(&self) -> &[Texel] { &self.stars }
    pub fn stars_mut(&mut self) -> &mut [Texel] { &mut self.stars }
    pub fn star(&self, row: usize, column: usize) -> Texel { self.stars[row * self.width + column] }
    pub fn overlay(&self) -> &[u8] { &self.overlay }
    /// The overlay and the text rasterizer together, for drawing text onto the overlay.
    pub fn overlay_and_text(&mut self) -> (&mut [u8], &mut TextRasterizer) { (&mut self.overlay, &mut self.text) }
    pub fn clear_overlay(&mut self) { self.overlay.fill(0); }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("width", &self.width).field("height", &self.height).field("drawing_area_x_offset", &self.drawing_area_x_offset).field("drawing_area_y_offset", &self.drawing_area_y_offset).field("drawing_area_width_and_height", &self.drawing_area_width_and_height).field("text_cell", &self.text_cell).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_insertion_centers_rows_and_clears_old_stars_without_touching_overlay() {
        // Distinct pixel values make transposition, row-stride and offset mistakes visible.
        let square: Vec<_> = (0..9).map(|index| Texel { rgb: [index as f32, 0.25, 0.5], opacity: 0.75 }).collect();
        for (width, height, x_offset, y_offset) in [(8, 5, 2, 1), (5, 8, 1, 2), (3, 3, 0, 0)] {
            let mut frame = Frame::new(width, height, 3, (10, 20));
            assert_eq!((frame.drawing_area_x_offset, frame.drawing_area_y_offset), (x_offset, y_offset));
            frame.stars_mut().fill(Texel { rgb: [9.0; 3], opacity: 1.0 });
            frame.overlay_and_text().0.fill(17);
            frame.insert_projected_stars(&square);

            // Only the designated square receives source pixels; every margin is reset.
            for row in 0..height {
                for column in 0..width {
                    let expected = if (y_offset..y_offset + 3).contains(&row) && (x_offset..x_offset + 3).contains(&column) {
                        square[(row - y_offset) * 3 + column - x_offset]
                    } else {
                        Texel::default()
                    };
                    assert_eq!(frame.star(row, column), expected);
                }
            }
            assert!(frame.overlay().iter().all(|&value| value == 17));

            // An empty selection on the next frame must erase the previously projected stars.
            frame.insert_projected_stars(&[Texel::default(); 9]);
            assert!(frame.stars().iter().all(|texel| *texel == Texel::default()));
        }
    }

    #[test]
    fn resizing_recomputes_placement_and_supports_zero_and_one_pixel_squares() {
        let mut frame = Frame::new(8, 5, 3, (10, 20));
        frame.resize(5, 8, 3);
        assert_eq!((frame.drawing_area_x_offset, frame.drawing_area_y_offset), (1, 2));
        frame.resize(5, 8, 1);
        assert_eq!((frame.drawing_area_x_offset, frame.drawing_area_y_offset), (2, 3));
        let pixel = Texel { rgb: [0.25; 3], opacity: 0.5 };
        frame.insert_projected_stars(&[pixel]);
        assert_eq!(frame.star(3, 2), pixel);
        frame.resize(0, 8, 0);
        frame.insert_projected_stars(&[]);
        assert!(frame.stars().is_empty());
    }

    #[test]
    fn invalid_square_is_rejected_before_existing_pixels_change() {
        let mut frame = Frame::square(3);
        let pixel = Texel { rgb: [0.25; 3], opacity: 0.5 };
        frame.stars_mut().fill(pixel);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| frame.insert_projected_stars(&[pixel; 8])));
        assert!(result.is_err());
        assert!(frame.stars().iter().all(|texel| *texel == pixel));
    }
}
