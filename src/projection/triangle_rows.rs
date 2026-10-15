//! Exact row ranges for a triangle: full pixels use a cheap path; only its outline needs polygon clipping.
use super::mesh::Point;

struct Edge { lower: Point, upper: Point, slope: f64 }

pub(super) struct TriangleRow {
    pub row: usize,
    pub first: usize,
    pub last: usize,
    whole: Option<(usize, usize)>,
}

impl TriangleRow {
    pub(super) fn is_whole(&self, column: usize) -> bool {
        // Both pixel edges lie inside the triangle at the row's top and bottom.
        self.whole.is_some_and(|(first, last)| first <= column && column <= last)
    }
}

fn span_at(edges: &[Edge; 3], y: f64) -> Option<(f64, f64)> {
    // Intersect the horizontal line with the three triangle edges, accepting either winding.
    let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
    for edge in edges {
        if y < edge.lower[1] || y > edge.upper[1] { continue; }
        if edge.upper[1] == edge.lower[1] {
            left = left.min(edge.lower[0]).min(edge.upper[0]);
            right = right.max(edge.lower[0]).max(edge.upper[0]);
        } else {
            let x = edge.lower[0] + (y - edge.lower[1]) * edge.slope;
            left = left.min(x);
            right = right.max(x);
        }
    }
    (left <= right).then_some((left, right))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn scan_triangle_rows(
    triangle: [Point; 3], left: usize, top: usize, width: usize, height: usize, size: usize,
    mut visit: impl FnMut(TriangleRow),
) {
    // Prepare edge slopes once and clip the vertical extent to the region's screen rectangle.
    let edges = std::array::from_fn(|index| {
        let first = triangle[index];
        let second = triangle[(index + 1) % 3];
        let (lower, upper) = if first[1] <= second[1] { (first, second) } else { (second, first) };
        let slope = if upper[1] > lower[1] { (upper[0] - lower[0]) / (upper[1] - lower[1]) } else { 0.0 };
        Edge { lower, upper, slope }
    });
    let minimum_y = triangle.iter().map(|point| point[1]).fold(f64::INFINITY, f64::min);
    let maximum_y = triangle.iter().map(|point| point[1]).fold(f64::NEG_INFINITY, f64::max);
    let first_row = (minimum_y - 0.5).ceil().max(top as f64);
    let last_row = (maximum_y + 0.5).floor().min((top + height - 1) as f64);
    if first_row > last_row { return; }
    let radius = (size as f64 - 1.0) * 0.5;
    let center = [radius, radius];
    let mut upper = span_at(&edges, first_row - 0.5);

    // Bound each horizontal strip by its two edge spans and any triangle vertices inside the strip.
    for row in first_row as usize..=last_row as usize {
        let y = row as f64;
        let lower = span_at(&edges, y + 0.5);
        let upper_span = upper;
        upper = lower;
        let reach = radius * radius + 1e-9 - (y - center[1]).powi(2);
        if reach < 0.0 { continue; }
        let half_width = reach.sqrt();
        let (mut outside_left, mut outside_right) = (f64::INFINITY, f64::NEG_INFINITY);
        for (start, end) in [upper_span, lower].into_iter().flatten() {
            outside_left = outside_left.min(start);
            outside_right = outside_right.max(end);
        }
        for point in triangle {
            if point[1] >= y - 0.5 && point[1] <= y + 0.5 {
                outside_left = outside_left.min(point[0]);
                outside_right = outside_right.max(point[0]);
            }
        }
        if outside_left > outside_right { continue; }
        let first = (outside_left - 0.5).ceil().max(left as f64).max((center[0] - half_width).ceil());
        let last = (outside_right + 0.5).floor().min((left + width - 1) as f64).min((center[0] + half_width).floor());
        if first > last { continue; }

        // A convex triangle contains a complete pixel when all four pixel corners lie inside it.
        let whole = match (upper_span, lower) {
            (Some(upper), Some(lower)) => {
                let first_whole = (upper.0.max(lower.0) + 0.5).ceil().max(first);
                let last_whole = (upper.1.min(lower.1) - 0.5).floor().min(last);
                (first_whole <= last_whole).then_some((first_whole as usize, last_whole as usize))
            }
            _ => None,
        };
        visit(TriangleRow { row, first: first as usize, last: last as usize, whole });
    }
}
