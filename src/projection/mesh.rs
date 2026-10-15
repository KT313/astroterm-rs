//! What the equidistant mesh warp shares: screen points, the affine texture mapping of one triangle and the two
//! tests that decide whether four projected tile corners can be drawn as flat pieces at all.
use crate::astro::Vector3;

pub(super) type Point = [f64; 2];

/// Test the small spherical quad against the direction directly behind the camera. Boundary points count as
/// inside: the projection is singular there too. Texels away from the back hemisphere take the quick exit.
pub(super) fn contains_antipode(corners: [Vector3; 4], forward: Vector3) -> bool {
    if corners.iter().any(|corner| corner.dot(forward) >= 0.0) { return false; }
    let centre = corners.into_iter().fold(Vector3::default(), |sum, corner| sum + corner);
    (0..4).all(|i| {
        let normal = corners[i].cross(corners[(i + 1) % 4]);
        normal.dot(forward * -1.0) * normal.dot(centre) >= 0.0
    })
}

pub(super) fn is_convex(corners: [Point; 4]) -> bool {
    // Each consecutive pair of edges must turn in the same direction.
    let turns: [f64; 4] = std::array::from_fn(|index| {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let c = corners[(index + 2) % 4];
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    });
    turns.iter().all(|turn| turn.is_finite())
        && (turns.iter().all(|&turn| turn > 0.0) || turns.iter().all(|&turn| turn < 0.0))
}

/// Affine screen-to-texture mapping for one triangle. Derivatives are calculated once per triangle, not pixel.
pub(super) struct TextureMapping {
    screen_origin: Point,
    texture_origin: Point,
    per_screen_x: Point,
    per_screen_y: Point,
}

impl TextureMapping {
    pub(super) fn new(screen: [Point; 3], texture: [Point; 3]) -> Option<Self> {
        // Invert the triangle's two screen-space edges; a collapsed triangle has no usable mapping.
        let screen_first = [screen[1][0] - screen[0][0], screen[1][1] - screen[0][1]];
        let screen_second = [screen[2][0] - screen[0][0], screen[2][1] - screen[0][1]];
        let determinant = screen_first[0] * screen_second[1] - screen_second[0] * screen_first[1];
        if determinant == 0.0 || !determinant.is_finite() { return None; }

        // Calculate how one screen-pixel step changes the two texture coordinates.
        let mut per_screen_x = [0.0; 2];
        let mut per_screen_y = [0.0; 2];
        for axis in 0..2 {
            let texture_first = texture[1][axis] - texture[0][axis];
            let texture_second = texture[2][axis] - texture[0][axis];
            per_screen_x[axis] = (texture_first * screen_second[1] - texture_second * screen_first[1]) / determinant;
            per_screen_y[axis] = (texture_second * screen_first[0] - texture_first * screen_second[0]) / determinant;
        }
        if per_screen_x.iter().chain(&per_screen_y).any(|value| !value.is_finite()) { return None; }

        Some(Self { screen_origin: screen[0], texture_origin: texture[0], per_screen_x, per_screen_y })
    }

    /// The texture box of a screen rectangle of `size` pixels (the footprint is that of a whole pixel).
    pub(super) fn footprint_of(&self, size: Point) -> Point {
        std::array::from_fn(|axis| self.per_screen_x[axis].abs() * size[0] + self.per_screen_y[axis].abs() * size[1])
    }

    pub(super) fn texture_position(&self, pixel: Point) -> Point {
        // Interpolate inside this triangle instead of repeating sky and camera calculations per texel.
        let delta_x = pixel[0] - self.screen_origin[0];
        let delta_y = pixel[1] - self.screen_origin[1];
        std::array::from_fn(|axis| {
            self.texture_origin[axis] + delta_x * self.per_screen_x[axis] + delta_y * self.per_screen_y[axis]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antipodal_tiles_are_rejected_without_rejecting_nearby_back_facing_tiles() {
        let forward = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let directions = |offset| [(-0.01 + offset, -0.01), (0.01 + offset, -0.01), (0.01 + offset, 0.01), (-0.01 + offset, 0.01)]
            .map(|(x, y)| Vector3 { x, y, z: -1.0 }.normalized());
        assert!(contains_antipode(directions(0.0), forward));
        assert!(!contains_antipode(directions(0.1), forward));
    }

    #[test]
    fn mapping_handles_skew_and_reversed_winding() {
        let screen = [[10.0, 20.0], [18.0, 22.0], [12.0, 26.0]];
        let texture = [[0.0, 0.0], [64.0, 0.0], [0.0, 64.0]];
        for indices in [[0, 1, 2], [0, 2, 1]] {
            let mapping = TextureMapping::new(indices.map(|i| screen[i]), indices.map(|i| texture[i])).unwrap();
            for (point, expected) in screen.into_iter().zip(texture) {
                for (actual, expected) in mapping.texture_position(point).into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
        assert!(TextureMapping::new([[0.0; 2]; 3], texture).is_none());
        assert!(!is_convex([[0.0; 2]; 4]));
        assert!(!is_convex([[0.0, 0.0], [1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]));
    }

}
