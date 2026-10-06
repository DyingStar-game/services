//! The little linear algebra the model needs: a vector and a rotation matrix, written out so that
//! the conventions are explicit and identical to Godot's `Basis` (column-major, right-handed).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const UP: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 0.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> Vec3 {
        let l = self.length();
        if l == 0.0 {
            Vec3::ZERO
        } else {
            self.scale(1.0 / l)
        }
    }

    pub fn scale(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }

    pub fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    pub fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    pub fn distance_to(self, o: Vec3) -> f64 {
        self.sub(o).length()
    }

    /// Angle between two vectors, radians.
    pub fn angle_to(self, o: Vec3) -> f64 {
        let c = self.dot(o) / (self.length() * o.length());
        c.clamp(-1.0, 1.0).acos()
    }
}

/// A 3×3 rotation matrix, as rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub m: [[f64; 3]; 3],
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };

    /// Rotation about +X by `a` radians (right-handed: +Y turns toward +Z).
    pub fn rx(a: f64) -> Mat3 {
        let (s, c) = a.sin_cos();
        Mat3 { m: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]] }
    }

    /// Rotation about +Y by `a` radians (right-handed: +Z turns toward +X).
    pub fn ry(a: f64) -> Mat3 {
        let (s, c) = a.sin_cos();
        Mat3 { m: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]] }
    }

    /// Rotation about +Z by `a` radians (right-handed: +X turns toward +Y).
    pub fn rz(a: f64) -> Mat3 {
        let (s, c) = a.sin_cos();
        Mat3 { m: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]] }
    }

    pub fn mul(self, o: Mat3) -> Mat3 {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.m[i][k] * o.m[k][j]).sum();
            }
        }
        Mat3 { m: r }
    }

    pub fn apply(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.m[0][0] * v.x + self.m[0][1] * v.y + self.m[0][2] * v.z,
            self.m[1][0] * v.x + self.m[1][1] * v.y + self.m[1][2] * v.z,
            self.m[2][0] * v.x + self.m[2][1] * v.y + self.m[2][2] * v.z,
        )
    }

    /// The inverse of a rotation is its transpose.
    pub fn transposed(self) -> Mat3 {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.m[j][i];
            }
        }
        Mat3 { m: r }
    }

    /// Column `i` of the matrix: where the basis vector i lands.
    pub fn column(self, i: usize) -> Vec3 {
        Vec3::new(self.m[0][i], self.m[1][i], self.m[2][i])
    }
}

/// Fold an angle into [0, 2π).
pub fn normalize_angle(a: f64) -> f64 {
    let x = a % std::f64::consts::TAU;
    if x < 0.0 {
        x + std::f64::consts::TAU
    } else {
        x
    }
}

/// Positive modulo, like GDScript's `fposmod`.
pub fn fposmod(a: f64, b: f64) -> f64 {
    let x = a % b;
    if x < 0.0 {
        x + b
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn ry_turns_z_toward_x_like_godot() {
        let v = Mat3::ry(FRAC_PI_2).apply(Vec3::new(0.0, 0.0, 1.0));
        assert!((v.x - 1.0).abs() < 1e-12 && v.z.abs() < 1e-12);
        let w = Mat3::ry(FRAC_PI_2).apply(Vec3::new(1.0, 0.0, 0.0));
        assert!((w.z + 1.0).abs() < 1e-12);
    }

    #[test]
    fn rz_turns_x_toward_y() {
        let v = Mat3::rz(FRAC_PI_2).apply(Vec3::new(1.0, 0.0, 0.0));
        assert!((v.y - 1.0).abs() < 1e-12 && v.x.abs() < 1e-12);
    }

    #[test]
    fn transpose_inverts_a_rotation() {
        let r = Mat3::ry(0.7).mul(Mat3::rx(0.3)).mul(Mat3::rz(1.1));
        let v = Vec3::new(0.2, -0.5, 0.9);
        let back = r.transposed().apply(r.apply(v));
        assert!(back.distance_to(v) < 1e-12);
    }
}
