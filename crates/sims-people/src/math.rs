//! Small vector and rotation math for drawing. Presentation only: ordinary
//! `f32`, never fed back into the simulation.

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The vector scaled to length 1, or zero for a zero vector.
    pub fn normalize(self) -> Self {
        let l = self.length();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            Self::ZERO
        }
    }

    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// A rotation as a unit quaternion (Hamilton convention: `a * b` rotates
/// by `b`, then by `a`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// A rotation by `angle` radians about the vertical (Y) axis, from its
    /// sine and cosine.
    pub fn about_y(sin: f32, cos: f32) -> Self {
        // Half-angle identities, so no trigonometry is needed here.
        let c = ((1.0 + cos) * 0.5).max(0.0).sqrt();
        let s = ((1.0 - cos) * 0.5).max(0.0).sqrt();
        Self::new(0.0, if sin < 0.0 { -s } else { s }, 0.0, c)
    }

    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// The quaternion scaled to length 1, or the identity for zero.
    pub fn normalize(self) -> Self {
        let l = self.dot(self).sqrt();
        if l > 0.0 {
            Self::new(self.x / l, self.y / l, self.z / l, self.w / l)
        } else {
            Self::IDENTITY
        }
    }

    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Normalized linear interpolation along the shorter way.
    pub fn nlerp(self, o: Self, t: f32) -> Self {
        let o = if self.dot(o) < 0.0 {
            Self::new(-o.x, -o.y, -o.z, -o.w)
        } else {
            o
        };
        Self::new(
            self.x + (o.x - self.x) * t,
            self.y + (o.y - self.y) * t,
            self.z + (o.z - self.z) * t,
            self.w + (o.w - self.w) * t,
        )
        .normalize()
    }
}

impl Mul for Quat {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::new(
            self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        )
    }
}

/// A rotation then a translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    pub fn point(&self, p: Vec3) -> Vec3 {
        self.rotation.rotate(p) + self.translation
    }

    pub fn vector(&self, v: Vec3) -> Vec3 {
        self.rotation.rotate(v)
    }
}

impl Mul for Transform {
    type Output = Self;
    /// `a * b` applies `b`, then `a`.
    fn mul(self, o: Self) -> Self {
        Self {
            rotation: (self.rotation * o.rotation).normalize(),
            translation: self.point(o.translation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn a_quarter_turn_about_y_takes_x_to_minus_z() {
        let q = Quat::about_y(1.0, 0.0);
        assert!(close(
            q.rotate(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(0.0, 0.0, -1.0)
        ));
        let back = Quat::about_y(-1.0, 0.0);
        assert!(close(
            (q * back).rotate(Vec3::new(1.0, 2.0, 3.0)),
            Vec3::new(1.0, 2.0, 3.0)
        ));
    }

    #[test]
    fn transforms_compose_child_after_parent() {
        let parent = Transform {
            rotation: Quat::about_y(1.0, 0.0),
            translation: Vec3::new(0.0, 1.0, 0.0),
        };
        let child = Transform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(1.0, 0.0, 0.0),
        };
        let world = parent * child;
        assert!(close(world.point(Vec3::ZERO), Vec3::new(0.0, 1.0, -1.0)));
    }

    #[test]
    fn nlerp_takes_the_short_way() {
        let a = Quat::IDENTITY;
        let b = Quat::new(0.0, 0.0, 0.0, -1.0);
        assert_eq!(a.nlerp(b, 0.5), Quat::IDENTITY);
    }
}
