//! Screen layout and reusable star, overlay and text buffers for the frame pipeline.
use crate::model::Texel;
use crate::text::{TextRasterizer, create_text_rasterizer, set_text_cell_size};

/// The square of the frame that shows the sky: the projection's radius-1 circle is inscribed in it. The rest of
/// the frame is margin for text (owner's layout: a centred square sky, the metadata panel at the top left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkyArea {
    pub x: usize,                           // left column of the square in frame pixels
    pub y: usize,                           // top row
    pub size: usize,                        // side in pixels
}

impl SkyArea {
    /// The largest centred square of a `width` × `height` frame.
    pub fn centred(width: usize, height: usize) -> Self {
        let size = width.min(height);
        Self { x: (width - size) / 2, y: (height - size) / 2, size }
    }
    /// The frame pixel (row, column) of a sky-square cell, as `project_to_cell` numbers them.
    pub fn frame_pixel(&self, cell: (i32, i32)) -> (i32, i32) { (cell.0 + self.y as i32, cell.1 + self.x as i32) }
    /// Whether a sky cell and the three cells above and to the left of it lie inside the square (a star glyph fits).
    pub fn glyph_fits(&self, (row, column): (i32, i32)) -> bool { row > 0 && column > 0 && (row as usize) < self.size && (column as usize) < self.size }
    pub fn viewport(&self) -> crate::model::ProjectionViewport { crate::model::ProjectionViewport { width: self.size, height: self.size } }
}

/// The screen-sized buffers of a frame: the star layer step 5 fills, the overlay step 6 draws on top of it, the
/// text rasterizer the overlay's labels use, and their screen layout.
pub struct Frame {
    width: usize,
    height: usize,
    sky: SkyArea,
    stars: Vec<Texel>,                      // row-major, width × height: light per pixel before tone mapping
    overlay: Vec<u8>,                       // premultiplied RGBA, transparent where nothing was drawn: points, lines and text over the stars
    text: TextRasterizer,
    text_cell: (u16, u16),                  // pixel size of one text cell: glyph size and line spacing
}

impl Frame {
    /// A frame with the sky in the given square and text cells of `text_cell` pixels.
    pub fn new(width: usize, height: usize, sky: SkyArea, text_cell: (u16, u16)) -> Self {
        let mut text = create_text_rasterizer().expect("the bundled font parses");
        set_text_cell_size(&mut text, text_cell.0, text_cell.1);
        Self { width, height, sky, stars: vec![Texel::default(); width * height], overlay: vec![0; width * height * 4], text, text_cell }
    }

    /// A square frame that is all sky, with the default text cell: the shape of the tests and the parity fixtures.
    pub fn square(size: usize) -> Self { Self::new(size, size, SkyArea { x: 0, y: 0, size }, (10, 20)) }

    /// Adopt a new size and sky square, clearing both image layers.
    pub fn resize(&mut self, width: usize, height: usize, sky: SkyArea) {
        if (width, height, sky) == (self.width, self.height, self.sky) { return; }
        self.width = width;
        self.height = height;
        self.sky = sky;
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
    pub fn sky(&self) -> SkyArea { self.sky }
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
        f.debug_struct("Frame").field("width", &self.width).field("height", &self.height).field("sky", &self.sky).field("text_cell", &self.text_cell).finish()
    }
}

