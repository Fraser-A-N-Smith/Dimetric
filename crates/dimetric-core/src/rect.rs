//! Axis-aligned rectangles.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::fx::Fx;
use crate::vec::Vec2Fx;

/// An axis-aligned rectangle, stored as a corner and a size.
///
/// Width and height are expected to be non-negative; [`Rect::normalized`] fixes
/// a rectangle built from a drag that went up and to the left.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Minimum corner.
    pub pos: Vec2Fx,
    /// Extent from `pos`.
    pub size: Vec2Fx,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Rect = Rect {
        pos: Vec2Fx::ZERO,
        size: Vec2Fx::ZERO,
    };

    /// Construct from a corner and a size.
    #[inline]
    pub const fn new(pos: Vec2Fx, size: Vec2Fx) -> Rect {
        Rect { pos, size }
    }

    /// Construct from two opposite corners, in either order.
    #[inline]
    pub fn from_corners(a: Vec2Fx, b: Vec2Fx) -> Rect {
        let min = a.min(b);
        Rect::new(min, a.max(b) - min)
    }

    /// Construct from a centre point and half-extents.
    #[inline]
    pub fn from_center(center: Vec2Fx, half: Vec2Fx) -> Rect {
        Rect::new(center - half, half * 2)
    }

    /// Minimum corner.
    #[inline]
    pub fn min(self) -> Vec2Fx {
        self.pos
    }
    /// Maximum corner.
    #[inline]
    pub fn max(self) -> Vec2Fx {
        self.pos + self.size
    }
    /// Centre point.
    #[inline]
    pub fn center(self) -> Vec2Fx {
        self.pos + self.size / 2
    }
    /// Half the size.
    #[inline]
    pub fn half_size(self) -> Vec2Fx {
        self.size / 2
    }
    /// Width.
    #[inline]
    pub fn width(self) -> Fx {
        self.size.x
    }
    /// Height.
    #[inline]
    pub fn height(self) -> Fx {
        self.size.y
    }

    /// True when either dimension is zero or negative.
    #[inline]
    pub fn is_empty(self) -> bool {
        self.size.x <= Fx::ZERO || self.size.y <= Fx::ZERO
    }

    /// The same region with non-negative extents.
    #[inline]
    pub fn normalized(self) -> Rect {
        Rect::from_corners(self.pos, self.pos + self.size)
    }

    /// True when `p` is inside, treating the minimum edges as inside and the
    /// maximum edges as outside.
    ///
    /// Half-open on purpose: tiled regions built this way tessellate without
    /// double-counting a point on a shared border.
    #[inline]
    pub fn contains(self, p: Vec2Fx) -> bool {
        let max = self.max();
        p.x >= self.pos.x && p.y >= self.pos.y && p.x < max.x && p.y < max.y
    }

    /// True when the two rectangles share any interior area.
    #[inline]
    pub fn intersects(self, o: Rect) -> bool {
        let (a_max, b_max) = (self.max(), o.max());
        self.pos.x < b_max.x && o.pos.x < a_max.x && self.pos.y < b_max.y && o.pos.y < a_max.y
    }

    /// The overlapping region, or `None` when they do not overlap.
    pub fn intersection(self, o: Rect) -> Option<Rect> {
        if !self.intersects(o) {
            return None;
        }
        let min = self.pos.max(o.pos);
        Some(Rect::new(min, self.max().min(o.max()) - min))
    }

    /// The smallest rectangle containing both.
    pub fn union(self, o: Rect) -> Rect {
        let min = self.pos.min(o.pos);
        Rect::new(min, self.max().max(o.max()) - min)
    }

    /// Grown by `amount` on every side. A negative amount shrinks.
    #[inline]
    pub fn expanded(self, amount: Fx) -> Rect {
        let d = Vec2Fx::new(amount, amount);
        Rect::new(self.pos - d, self.size + d * 2)
    }

    /// Moved by `offset`.
    #[inline]
    pub fn translated(self, offset: Vec2Fx) -> Rect {
        Rect::new(self.pos + offset, self.size)
    }

    /// The point inside the rectangle closest to `p`.
    #[inline]
    pub fn closest_point(self, p: Vec2Fx) -> Vec2Fx {
        p.max(self.min()).min(self.max())
    }
}

impl fmt::Debug for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rect({}, {}, {}, {})",
            self.pos.x, self.pos.y, self.size.x, self.size.y
        )
    }
}
