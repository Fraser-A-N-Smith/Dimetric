//! Two-dimensional fixed-point vectors.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::angle::Angle;
use crate::fx::{Fx, FxWide};

/// A position, offset or velocity in world space.
///
/// Note which operations widen. Anything that multiplies two components
/// together — [`dot`](Vec2Fx::dot), [`cross`](Vec2Fx::cross),
/// [`length_squared`](Vec2Fx::length_squared) — returns [`FxWide`], because the
/// result leaves `Fx` range for any two points more than 181 units apart.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct Vec2Fx {
    /// Horizontal component.
    pub x: Fx,
    /// Vertical component.
    pub y: Fx,
}

/// Shorthand constructor.
#[inline]
pub const fn vec2(x: Fx, y: Fx) -> Vec2Fx {
    Vec2Fx { x, y }
}

impl Vec2Fx {
    /// The origin.
    pub const ZERO: Vec2Fx = Vec2Fx {
        x: Fx::ZERO,
        y: Fx::ZERO,
    };
    /// `(1, 1)`, the identity for component-wise scaling.
    pub const ONE: Vec2Fx = Vec2Fx {
        x: Fx::ONE,
        y: Fx::ONE,
    };
    /// `(1, 0)`.
    pub const X: Vec2Fx = Vec2Fx {
        x: Fx::ONE,
        y: Fx::ZERO,
    };
    /// `(0, 1)`.
    pub const Y: Vec2Fx = Vec2Fx {
        x: Fx::ZERO,
        y: Fx::ONE,
    };

    /// Construct from components.
    #[inline]
    pub const fn new(x: Fx, y: Fx) -> Vec2Fx {
        Vec2Fx { x, y }
    }

    /// Construct from whole numbers.
    #[inline]
    pub fn from_ints(x: i32, y: i32) -> Vec2Fx {
        Vec2Fx::new(Fx::from_int(x), Fx::from_int(y))
    }

    /// Construct from raw backing integers.
    #[inline]
    pub const fn from_raw(x: i32, y: i32) -> Vec2Fx {
        Vec2Fx::new(Fx::from_raw(x), Fx::from_raw(y))
    }

    /// True when both components are exactly zero.
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.x.is_zero() && self.y.is_zero()
    }

    /// Dot product. Widens.
    #[inline]
    pub fn dot(self, o: Vec2Fx) -> FxWide {
        self.x.wide() * o.x.wide() + self.y.wide() * o.y.wide()
    }

    /// The z component of the 3D cross product. Widens.
    ///
    /// Its sign is which side of `self` the vector `o` falls on, which is how
    /// polygon winding and separating-axis tests are decided.
    #[inline]
    pub fn cross(self, o: Vec2Fx) -> FxWide {
        self.x.wide() * o.y.wide() - self.y.wide() * o.x.wide()
    }

    /// Squared magnitude. Widens.
    ///
    /// Prefer this to [`length`](Vec2Fx::length) whenever you are comparing
    /// against a radius: squaring the radius once is cheaper and exact, while
    /// `length` costs a square root and truncates.
    #[inline]
    pub fn length_squared(self) -> FxWide {
        self.dot(self)
    }

    /// Magnitude, rounded toward zero.
    #[inline]
    pub fn length(self) -> Fx {
        self.length_squared().sqrt()
    }

    /// Squared distance to another point. Widens.
    #[inline]
    pub fn distance_squared(self, o: Vec2Fx) -> FxWide {
        (o - self).length_squared()
    }

    /// Distance to another point.
    #[inline]
    pub fn distance(self, o: Vec2Fx) -> Fx {
        (o - self).length()
    }

    /// A unit vector in the same direction, or zero if `self` is zero.
    ///
    /// The division happens at 32 fractional bits and is then truncated back,
    /// so short vectors do not lose their direction the way a naive
    /// `self / self.length()` would.
    pub fn normalized(self) -> Vec2Fx {
        let len = self.length().to_raw() as i64;
        if len == 0 {
            return Vec2Fx::ZERO;
        }
        Vec2Fx::from_raw(
            (((self.x.to_raw() as i64) << 16) / len) as i32,
            (((self.y.to_raw() as i64) << 16) / len) as i32,
        )
    }

    /// A vector in the same direction with magnitude `len`.
    #[inline]
    pub fn with_length(self, len: Fx) -> Vec2Fx {
        self.normalized() * len
    }

    /// Clamp the magnitude to at most `max`.
    pub fn clamp_length(self, max: Fx) -> Vec2Fx {
        let max_sq = max.wide() * max.wide();
        if self.length_squared() <= max_sq {
            self
        } else {
            self.with_length(max)
        }
    }

    /// Rotated a quarter turn counter-clockwise. Exact, no table lookup.
    #[inline]
    pub fn perp(self) -> Vec2Fx {
        Vec2Fx::new(-self.y, self.x)
    }

    /// Rotated by `angle`.
    pub fn rotated(self, angle: Angle) -> Vec2Fx {
        let (sin, cos) = angle.sin_cos();
        Vec2Fx::new(self.x * cos - self.y * sin, self.x * sin + self.y * cos)
    }

    /// The direction this vector points.
    #[inline]
    pub fn to_angle(self) -> Angle {
        Angle::from_vector(self.x, self.y)
    }

    /// Component-wise product.
    #[inline]
    pub fn mul_components(self, o: Vec2Fx) -> Vec2Fx {
        Vec2Fx::new(self.x * o.x, self.y * o.y)
    }

    /// Component-wise minimum.
    #[inline]
    pub fn min(self, o: Vec2Fx) -> Vec2Fx {
        Vec2Fx::new(self.x.min(o.x), self.y.min(o.y))
    }

    /// Component-wise maximum.
    #[inline]
    pub fn max(self, o: Vec2Fx) -> Vec2Fx {
        Vec2Fx::new(self.x.max(o.x), self.y.max(o.y))
    }

    /// Component-wise absolute value.
    #[inline]
    pub fn abs(self) -> Vec2Fx {
        Vec2Fx::new(self.x.abs(), self.y.abs())
    }

    /// Linear interpolation. `t` is not clamped.
    #[inline]
    pub fn lerp(self, to: Vec2Fx, t: Fx) -> Vec2Fx {
        Vec2Fx::new(self.x.lerp(to.x, t), self.y.lerp(to.y, t))
    }

    /// Reflect across the surface with unit normal `normal`.
    pub fn reflected(self, normal: Vec2Fx) -> Vec2Fx {
        let twice = (self.dot(normal) * FxWide::from_int(2)).narrow_saturating();
        self - normal * twice
    }

    /// The component of `self` that survives sliding along a surface with unit
    /// normal `normal`.
    ///
    /// This is the whole of "slide along the wall" movement: remove the part of
    /// the motion that points into the surface and keep the rest.
    pub fn slid_along(self, normal: Vec2Fx) -> Vec2Fx {
        let into = self.dot(normal).narrow_saturating();
        if into >= Fx::ZERO {
            self
        } else {
            self - normal * into
        }
    }

    /// Components as `f32`, for the render boundary only.
    #[inline]
    pub fn to_f32_pair(self) -> (f32, f32) {
        // I3-exempt: render boundary conversion.
        (self.x.to_f32(), self.y.to_f32())
    }
}

impl Add for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn add(self, o: Vec2Fx) -> Vec2Fx {
        Vec2Fx::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn sub(self, o: Vec2Fx) -> Vec2Fx {
        Vec2Fx::new(self.x - o.x, self.y - o.y)
    }
}
impl Neg for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn neg(self) -> Vec2Fx {
        Vec2Fx::new(-self.x, -self.y)
    }
}
impl Mul<Fx> for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn mul(self, s: Fx) -> Vec2Fx {
        Vec2Fx::new(self.x * s, self.y * s)
    }
}
impl Mul<i32> for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn mul(self, s: i32) -> Vec2Fx {
        Vec2Fx::new(self.x * s, self.y * s)
    }
}
impl Div<Fx> for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn div(self, s: Fx) -> Vec2Fx {
        Vec2Fx::new(self.x / s, self.y / s)
    }
}
impl Div<i32> for Vec2Fx {
    type Output = Vec2Fx;
    #[inline]
    fn div(self, s: i32) -> Vec2Fx {
        Vec2Fx::new(self.x / s, self.y / s)
    }
}
impl AddAssign for Vec2Fx {
    #[inline]
    fn add_assign(&mut self, o: Vec2Fx) {
        *self = *self + o;
    }
}
impl SubAssign for Vec2Fx {
    #[inline]
    fn sub_assign(&mut self, o: Vec2Fx) {
        *self = *self - o;
    }
}
impl Sum for Vec2Fx {
    fn sum<I: Iterator<Item = Vec2Fx>>(iter: I) -> Vec2Fx {
        iter.fold(Vec2Fx::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for Vec2Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}
impl fmt::Debug for Vec2Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vec2({}, {})", self.x, self.y)
    }
}
