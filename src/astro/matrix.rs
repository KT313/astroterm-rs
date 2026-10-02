//! Small f64 matrices used at frame boundaries; row-major, acting on column vectors.
use super::Vector3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix3(pub [[f64; 3]; 3]);

impl Matrix3 {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn apply(self, v: Vector3) -> Vector3 {
        let [x, y, z] = self.0.map(|row| row[0] * v.x + row[1] * v.y + row[2] * v.z);
        Vector3 { x, y, z }
    }

    pub fn transpose(self) -> Self {
        Self(std::array::from_fn(|i| std::array::from_fn(|j| self.0[j][i])))
    }

    pub fn compose(self, right: Self) -> Self {
        Self(std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                self.0[i][0] * right.0[0][j] + self.0[i][1] * right.0[1][j] + self.0[i][2] * right.0[2][j]
            })
        }))
    }

    /// Passive rotation about +Z, as in ERFA R3.
    pub fn rotate_z(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
    }
}
