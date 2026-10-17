//! Reusable worker pixels and the screen blocks that need clearing or composition.
use crate::constants::PROJECTION_SCREEN_BLOCK_SIZE;
use crate::model::Texel;
use super::screen_blend::blend_screen_images;

/// Scratch image keyed by square side length. On reuse, clear last frame's touched blocks; a resize
/// or failed drawing clears the whole image. Composition extends the destination's touched-block set.
#[derive(Default)]
pub(super) struct WorkerImage {
    pub pixels: Vec<Texel>,
    pub touched: TouchedBlocks,
    needs_full_clear: bool,
}

/// One entry per touched screen block, with flags preventing duplicate entries. Pixel writes must be
/// covered by this set; extra marked blank pixels are harmless. Reset whenever image dimensions change.
#[derive(Default)]
pub(super) struct TouchedBlocks {
    size: usize,
    columns: usize,
    indices: Vec<usize>,
    marked: Vec<bool>,
}

impl TouchedBlocks {
    fn reset(&mut self, size: usize) {
        const { assert!(PROJECTION_SCREEN_BLOCK_SIZE > 0, "screen blocks must have a nonzero size"); }
        self.size = size;
        self.columns = size.div_ceil(PROJECTION_SCREEN_BLOCK_SIZE);
        self.indices.clear();
        self.marked.resize(self.columns * self.columns, false);
        self.marked.fill(false);
    }

    /// Cover the inclusive written range of one row. Gaps within that range may be marked too.
    pub fn mark_span(&mut self, row: usize, first: usize, last: usize) {
        let block_row = row / PROJECTION_SCREEN_BLOCK_SIZE * self.columns;
        for column in first / PROJECTION_SCREEN_BLOCK_SIZE..=last / PROJECTION_SCREEN_BLOCK_SIZE {
            self.mark_block(block_row + column);
        }
    }

    fn mark_block(&mut self, index: usize) {
        if !self.marked[index] {
            self.marked[index] = true;
            self.indices.push(index);
        }
    }

    fn block_bounds(&self, index: usize) -> (usize, usize, usize, usize) {
        let left = index % self.columns * PROJECTION_SCREEN_BLOCK_SIZE;
        let top = index / self.columns * PROJECTION_SCREEN_BLOCK_SIZE;
        (left, (left + PROJECTION_SCREEN_BLOCK_SIZE).min(self.size),
            top, (top + PROJECTION_SCREEN_BLOCK_SIZE).min(self.size))
    }
}

impl WorkerImage {
    pub fn prepare(&mut self, size: usize) {
        // New dimensions invalidate block indices. Failed draws can contain unrecorded partial writes.
        let count = size.checked_mul(size).expect("drawing area pixel count overflow");
        if self.touched.size != size || self.needs_full_clear {
            let retained = self.pixels.len().min(count);
            self.pixels.resize(count, Texel::default());
            self.pixels[..retained].fill(Texel::default());
            self.touched.reset(size);
            self.needs_full_clear = false;
            return;
        }

        // Untouched blocks are already transparent. Retain both vector allocations for the next frame.
        for &index in &self.touched.indices {
            let (left, right, top, bottom) = self.touched.block_bounds(index);
            for row in top..bottom {
                self.pixels[row * size + left..row * size + right].fill(Texel::default());
            }
            self.touched.marked[index] = false;
        }
        self.touched.indices.clear();
    }

    pub fn invalidate(&mut self) {
        self.needs_full_clear = true;
    }

    pub fn merge(&mut self, source: &Self) {
        assert_eq!(self.pixels.len(), source.pixels.len(), "projection images must have matching dimensions");
        let size = self.touched.size;

        // Copy new blocks directly; only blocks touched by both images need screen blending.
        for &index in &source.touched.indices {
            let overlaps = self.touched.marked[index];
            let (left, right, top, bottom) = source.touched.block_bounds(index);
            self.touched.mark_block(index);
            for row in top..bottom {
                let start = row * size + left;
                let end = row * size + right;
                let destination = &mut self.pixels[start..end];
                let incoming = &source.pixels[start..end];
                if overlaps {
                    blend_screen_images(destination, incoming);
                } else {
                    destination.copy_from_slice(incoming);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(image: &mut WorkerImage, row: usize, column: usize, pixel: Texel) {
        image.pixels[row * image.touched.size + column] = pixel;
        image.touched.mark_span(row, column, column);
    }

    #[test]
    fn repeated_marks_and_partial_edge_blocks_clear_without_losing_capacity() {
        let size = PROJECTION_SCREEN_BLOCK_SIZE * 2 + 3;
        let mut image = WorkerImage::default();
        image.prepare(size);
        let bright = Texel { rgb: [0.5; 3], opacity: 0.5 };
        for _ in 0..4 {
            paint(&mut image, 0, 0, bright);
            paint(&mut image, size - 1, size - 1, bright);
        }
        assert_eq!(image.touched.indices.len(), 2);
        let addresses = (image.pixels.as_ptr(), image.touched.indices.as_ptr(), image.touched.marked.as_ptr());
        image.prepare(size);
        assert!(image.pixels.iter().all(|pixel| *pixel == Texel::default()));
        assert!(image.touched.indices.is_empty());
        assert!(image.touched.marked.iter().all(|&marked| !marked));
        assert_eq!(addresses, (image.pixels.as_ptr(), image.touched.indices.as_ptr(), image.touched.marked.as_ptr()));
    }

    #[test]
    fn merging_matches_full_scan_and_clears_the_union_on_reuse() {
        let size = PROJECTION_SCREEN_BLOCK_SIZE * 3 + 1;
        let mut destination = WorkerImage::default();
        let mut source = WorkerImage::default();
        destination.prepare(size);
        source.prepare(size);
        let faint = Texel { rgb: [1e-30, 2e-30, 3e-30], opacity: 4e-30 };
        let bright = Texel { rgb: [0.3, 0.4, 0.5], opacity: 0.5 };

        // Include actual overlap, disjoint blocks, blank gaps, and the cropped bottom-right block.
        paint(&mut destination, 0, 0, bright);
        paint(&mut source, 0, 0, bright);
        paint(&mut source, size - 1, size - 1, faint);
        paint(&mut source, PROJECTION_SCREEN_BLOCK_SIZE, 1, bright);
        let mut expected = destination.pixels.clone();
        blend_screen_images(&mut expected, &source.pixels);
        destination.merge(&source);
        assert_eq!(destination.pixels, expected);
        assert_eq!(destination.touched.indices.len(), 3);
        destination.prepare(size);
        assert!(destination.pixels.iter().all(|pixel| *pixel == Texel::default()));
    }

    #[test]
    fn resizing_and_failed_drawing_discard_stale_pixels_and_marks() {
        let mut image = WorkerImage::default();
        for size in [33, 17, 1, 0, 33] {
            image.prepare(size);
            assert_eq!(image.pixels.len(), size * size);
            assert!(image.pixels.iter().all(|pixel| *pixel == Texel::default()));
            if size > 0 {
                paint(&mut image, size - 1, size - 1, Texel { rgb: [0.5; 3], opacity: 0.5 });
            }
        }

        // A panic can interrupt a row before its written range is recorded. Clear everything next time.
        image.pixels[0] = Texel { rgb: [1.0; 3], opacity: 1.0 };
        image.invalidate();
        image.prepare(33);
        assert!(image.pixels.iter().all(|pixel| *pixel == Texel::default()));
    }
}
