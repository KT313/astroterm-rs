//! Scanline rasterization of one region's screen quadrilateral. For every pixel row whose square overlaps the
//! convex quad, the quad's span along the row's top and bottom edges gives the columns the quad touches and the
//! columns it covers whole. Whole pixels sample the tile once at their centre. Pixels on the quad's outline are
//! resolved through `SUB_ROWS` horizontal sub-rows: the covered fraction of each sub-row gives the coverage,
//! and the covered part of the pixel (not the whole pixel) is what the tile is sampled over, so a star that
//! ends up a fraction of a pixel inside the outline keeps its light. Each row is split where the quad's
//! diagonal crosses it, so each pixel maps through the affine mapping of the triangle it lies in. Rows and
//! columns whose pixel centre lies outside the frame or the sky disc are skipped, as before.
use crate::projection::SkyArea;

pub(super) type Point = [f64; 2];

/// Horizontal sub-rows per pixel row for the pixels on the quad's outline: coverage is exact to 1/8 pixel.
const SUB_ROWS: usize = 4;

/// One edge of the quad, ordered by y: it crosses the line at `y` between `low` and `high` inclusive.
#[derive(Clone, Copy)]
struct Edge { low: Point, high: Point, slope: f64 }

/// A quad prepared for scanning: its edges and the diagonal between its two triangles.
pub(super) struct Quad {
    corners: [Point; 4],
    edges: [Edge; 4],
    first_triangle_right: bool,             // moving right along a row crosses the diagonal towards corner 1's side
}

impl Quad {
    fn new(corners: [Point; 4]) -> Self {
        let edges = std::array::from_fn(|index| {
            let (p, q) = (corners[index], corners[(index + 1) % 4]);
            let (low, high) = if p[1] <= q[1] { (p, q) } else { (q, p) };
            let slope = if high[1] > low[1] { (high[0] - low[0]) / (high[1] - low[1]) } else { 0.0 };
            Edge { low, high, slope }
        });
        let diagonal = [corners[2][0] - corners[0][0], corners[2][1] - corners[0][1]];
        let corner_side = diagonal[0] * (corners[1][1] - corners[0][1]) - diagonal[1] * (corners[1][0] - corners[0][0]);
        Self { corners, edges, first_triangle_right: -diagonal[1] * corner_side > 0.0 }
    }

    /// The columns the quad covers along the horizontal line at `y`, or `None` where the line misses it.
    fn span_at(&self, y: f64) -> Option<(f64, f64)> {
        let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
        for edge in &self.edges {
            if y < edge.low[1] || y > edge.high[1] { continue; }
            if edge.high[1] > edge.low[1] {
                let x = edge.low[0] + (y - edge.low[1]) * edge.slope;
                left = left.min(x);
                right = right.max(x);
            } else {                                                                         // a horizontal edge on the line: both ends
                left = left.min(edge.low[0]).min(edge.high[0]);
                right = right.max(edge.low[0]).max(edge.high[0]);
            }
        }
        (left <= right).then_some((left, right))
    }

    /// Where the diagonal crosses the row at `y`, and which triangle lies to the left of it.
    fn diagonal_split(&self, y: f64, first: usize, last: usize) -> (usize, usize) {
        let (c0, c2) = (self.corners[0], self.corners[2]);
        if c2[1] == c0[1] { return (last + 1, 0); }                                          // the row runs along the diagonal: both mappings agree
        let crossing = c0[0] + (y - c0[1]) * (c2[0] - c0[0]) / (c2[1] - c0[1]);
        (crossing.ceil().clamp(first as f64, last as f64 + 1.0) as usize, usize::from(self.first_triangle_right))
    }
}

/// The covered part of a pixel on the quad's outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EdgePixel {
    pub coverage: f32,                      // the fraction of the pixel inside the quad
    pub centre: Point,                      // of the covered part, in frame pixels
    pub size: Point,                        // of the covered part, in pixels
}

/// One pixel row of a quad: the columns `first..=last` the quad touches, the columns it covers whole, and the
/// diagonal split: columns below `split` lie in triangle `left_triangle` (0 for corners 0, 1, 2; 1 for corners
/// 0, 2, 3), the rest in the other one. Outline pixels are resolved on request through `edge_pixel`.
pub(super) struct Row<'a> {
    quad: &'a Quad,
    pub row: usize,
    pub first: usize,
    pub last: usize,
    pub full: Option<(usize, usize)>,       // the columns covered whole, inclusive
    pub split: usize,
    pub left_triangle: usize,
    sub_spans: Option<[Option<(f64, f64)>; SUB_ROWS]>, // the quad's span on each sub-row, computed on first use
}

impl Row<'_> {
    /// The two runs of the row as half-open column ranges with their triangle; either may be empty.
    pub(super) fn runs(&self) -> [(std::ops::Range<usize>, usize); 2] {
        [(self.first..self.split, self.left_triangle), (self.split..self.last + 1, 1 - self.left_triangle)]
    }

    /// Whether the pixel square at `column` lies inside the quad.
    #[inline]
    pub(super) fn is_whole(&self, column: usize) -> bool {
        self.full.is_some_and(|(first, last)| first <= column && column <= last)
    }

    /// The covered part of an outline pixel, or `None` when the quad misses the pixel after all.
    pub(super) fn edge_pixel(&mut self, column: usize) -> Option<EdgePixel> {
        let top = self.row as f64 - 0.5;
        let quad = self.quad;
        let spans = self.sub_spans.get_or_insert_with(|| std::array::from_fn(|k| quad.span_at(top + (k as f64 + 0.5) / SUB_ROWS as f64)));
        let (pixel_left, pixel_right) = (column as f64 - 0.5, column as f64 + 0.5);
        let (mut covered, mut left, mut right, mut first_sub, mut last_sub) = (0.0, f64::INFINITY, f64::NEG_INFINITY, SUB_ROWS, 0);
        for (k, span) in spans.iter().enumerate() {
            let Some((span_left, span_right)) = *span else { continue; };
            let (inside_left, inside_right) = (span_left.max(pixel_left), span_right.min(pixel_right));
            if inside_right <= inside_left { continue; }
            covered += inside_right - inside_left;
            left = left.min(inside_left);
            right = right.max(inside_right);
            first_sub = first_sub.min(k);
            last_sub = last_sub.max(k);
        }
        if covered == 0.0 { return None; }
        let (part_top, part_bottom) = (top + first_sub as f64 / SUB_ROWS as f64, top + (last_sub + 1) as f64 / SUB_ROWS as f64);
        Some(EdgePixel {
            coverage: (covered / SUB_ROWS as f64) as f32,
            centre: [(left + right) * 0.5, (part_top + part_bottom) * 0.5],
            size: [right - left, part_bottom - part_top],
        })
    }
}

/// Visit every pixel row that the quad (corners in order, either winding) touches inside the frame and the sky disc.
pub(super) fn scan_quad(corners: [Point; 4], width: usize, height: usize, sky: SkyArea, mut visit: impl FnMut(&mut Row<'_>)) {
    // Rows whose pixel square overlaps the quad's vertical extent, inside the frame.
    let (top, bottom) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(top, bottom), corner| (top.min(corner[1]), bottom.max(corner[1])));
    if !(top.is_finite() && bottom.is_finite()) || width == 0 || height == 0 { return; }
    let (first_row, last_row) = (((top - 0.5).floor() + 1.0).max(0.0), ((bottom + 0.5).ceil() - 1.0).min((height - 1) as f64)); // the square must overlap, not touch
    if first_row > last_row { return; }
    let quad = Quad::new(corners);

    // The sky disc, tested by pixel centre as before.
    let radius = (sky.size as f64 - 1.0) * 0.5;
    let (cx, cy) = (sky.x as f64 + radius, sky.y as f64 + radius);

    let mut upper = quad.span_at(first_row - 0.5);                                           // the span along the row's top edge; the next row's is this row's bottom
    for row in first_row as usize..=last_row as usize {
        let y = row as f64;
        let lower = quad.span_at(y + 0.5);
        let (upper_span, lower_span) = (upper, lower);
        upper = lower;
        let reach = radius * radius + 1e-9 - (y - cy) * (y - cy);
        if reach < 0.0 { continue; }                                                         // the row misses the sky disc
        let half = reach.sqrt();
        let (disc_first, disc_last) = ((cx - half).ceil().max(0.0), (cx + half).floor().min((width - 1) as f64));

        // The columns the quad touches in this row: both edge spans and the corners inside the row.
        let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
        for span in [upper_span, lower_span].into_iter().flatten() {
            left = left.min(span.0);
            right = right.max(span.1);
        }
        for corner in corners.iter().filter(|corner| corner[1] > y - 0.5 && corner[1] < y + 0.5) {
            left = left.min(corner[0]);
            right = right.max(corner[0]);
        }
        if left > right { continue; }
        let (first, last) = (((left - 0.5).floor() + 1.0).max(disc_first), ((right + 0.5).ceil() - 1.0).min(disc_last)); // pixels that overlap, not touch
        if first > last { continue; }
        let (first, last) = (first as usize, last as usize);

        // The columns covered whole: the pixel's four corners inside the quad, which is convex.
        let full = match (upper_span, lower_span) {
            (Some(upper), Some(lower)) => {
                let (inner_left, inner_right) = (upper.0.max(lower.0), upper.1.min(lower.1));
                let (whole_first, whole_last) = ((inner_left + 0.5).ceil().max(first as f64), (inner_right - 0.5).floor().min(last as f64));
                (whole_first <= whole_last).then_some((whole_first as usize, whole_last as usize))
            }
            _ => None,
        };
        let (split, left_triangle) = quad.diagonal_split(y, first, last);
        visit(&mut Row { quad: &quad, row, first, last, full, split, left_triangle, sub_spans: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scanned { row: usize, first: usize, last: usize, full: Option<(usize, usize)>, split: usize, left_triangle: usize, edges: Vec<(usize, Option<EdgePixel>)> }

    fn scan(corners: [Point; 4], size: usize) -> Vec<Scanned> {
        let mut rows = Vec::new();
        scan_quad(corners, size, size, SkyArea { x: 0, y: 0, size }, |row| {
            let outline: Vec<usize> = (row.first..=row.last).filter(|&column| !row.is_whole(column)).collect();
            let edges = outline.into_iter().map(|column| (column, row.edge_pixel(column))).collect();
            rows.push(Scanned { row: row.row, first: row.first, last: row.last, full: row.full, split: row.split, left_triangle: row.left_triangle, edges });
        });
        rows
    }
    fn square(low: f64, high: f64) -> [Point; 4] { [[low, low], [high, low], [high, high], [low, high]] }
    fn close(a: f64, b: f64) -> bool { (a - b).abs() < 1e-6 }                              // coverage is f32
    fn edge(row: &Scanned, column: usize) -> EdgePixel { row.edges.iter().find(|(c, _)| *c == column).unwrap().1.unwrap() }

    #[test]
    fn whole_pixels_are_exact_and_outline_pixels_carry_their_covered_part() {
        let [row] = &scan(square(3.5, 4.5), 9)[..] else { panic!() };                     // one pixel exactly
        assert_eq!((row.row, row.first, row.last, row.full), (4, 4, 4, Some((4, 4))));
        assert!(row.edges.is_empty());

        let [row] = &scan(square(3.75, 4.25), 9)[..] else { panic!() };                   // a quarter pixel in the middle of pixel 4
        assert_eq!((row.first, row.last, row.full), (4, 4, None));
        assert_eq!(edge(row, 4), EdgePixel { coverage: 0.25, centre: [4.0, 4.0], size: [0.5, 0.5] });

        let rows = scan(square(3.8, 6.3), 9);                                               // 2.5 pixels wide, from the lower part of row 4 to the upper part of row 6
        assert_eq!(rows.iter().map(|row| (row.row, row.first, row.last, row.full)).collect::<Vec<_>>(), [(4, 4, 6, None), (5, 4, 6, Some((5, 5))), (6, 4, 6, None)]);
        let top_middle = edge(&rows[0], 5);
        assert_eq!(top_middle.coverage, 0.75);                                               // the quad starts at 3.8: three of four sub-rows
        assert!(close(top_middle.centre[1], 4.125) && close(top_middle.size[1], 0.75) && top_middle.size[0] == 1.0);
        let middle_left = edge(&rows[1], 4);
        assert!(close(f64::from(middle_left.coverage), 0.7) && close(middle_left.centre[0], 4.15) && close(middle_left.size[0], 0.7) && middle_left.size[1] == 1.0);
        let middle_right = edge(&rows[1], 6);
        assert!(close(f64::from(middle_right.coverage), 0.8) && close(middle_right.centre[0], 5.9));
        assert_eq!(edge(&rows[2], 5).coverage, 0.75);                                        // the quad ends at 6.3: three sub-rows of row 6 (5.5 to 6.5)
        assert!(close(f64::from(edge(&rows[2], 4).coverage), 0.525));                        // 0.7 wide, 0.75 high
    }

    #[test]
    fn rows_split_at_the_diagonal_whichever_the_winding() {
        let corners = square(4.5, 10.5);                                                    // the diagonal x = y from corner 0 to corner 2, inside a 15-pixel disc
        for (winding, below_diagonal) in [([0, 1, 2, 3], 0), ([0, 3, 2, 1], 1)] {           // corner 1 lies below (x > y) or above the diagonal
            let quad = winding.map(|index| corners[index]);
            let rows = scan(quad, 15);
            assert_eq!(rows.len(), 6);
            for row in rows {
                assert_eq!(row.split, row.row, "{}", row.row);                               // x = y crosses row y at column y
                assert_eq!(row.left_triangle, 1 - below_diagonal);                            // columns left of the crossing have x < y
                assert_eq!((row.first, row.last, row.full), (5, 10, Some((5, 10))));
            }
        }
        let rows = scan([[0.0, 4.0], [4.0, 0.0], [8.0, 4.0], [4.0, 8.0]], 9);
        let middle = rows.iter().find(|row| row.row == 4).unwrap();
        assert_eq!((middle.first, middle.last, middle.split, middle.left_triangle), (0, 8, 9, 0)); // a horizontal diagonal: one run
        let upper = rows.iter().find(|row| row.row == 2).unwrap();                           // the diamond is 4 wide here: whole pixels and the two outline pixels
        assert_eq!((upper.first, upper.last, upper.full), (2, 6, Some((3, 5))));
        assert!(upper.edges.iter().all(|(_, edge)| edge.is_some_and(|edge| edge.coverage > 0.0 && edge.coverage < 1.0)), "{:?}", upper.edges);
    }

    #[test]
    fn pixels_outside_the_frame_and_the_sky_disc_are_skipped() {
        let rows = scan(square(-100.0, 100.0), 9);                                          // corners far outside enclose the frame
        assert_eq!(rows.len(), 9);
        let middle = rows.iter().find(|row| row.row == 4).unwrap();
        assert_eq!((middle.first, middle.last, middle.full), (0, 8, Some((0, 8))));
        let top = rows.iter().find(|row| row.row == 0).unwrap();
        assert_eq!((top.first, top.last, top.full), (4, 4, Some((4, 4))));                   // only the disc's top pixel
        assert!(scan([[20.0, 20.0], [30.0, 20.0], [30.0, 30.0], [20.0, 30.0]], 9).is_empty());
        assert!(scan([[f64::NAN; 2]; 4], 9).is_empty());
    }
}
