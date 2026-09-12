//! Collision shapes.

use dimetric_core::{Fx, FxWide, Rect, Vec2Fx};

/// A collision shape in a body's local space.
///
/// Deliberately not a general shape library. These are the shapes a top-down
/// action game needs: a box for characters and walls, a circle for projectiles
/// and aggro ranges, and a convex hull for the occasional angled prop.
#[derive(Clone, PartialEq, Debug)]
pub enum Shape {
    /// A box, given by its half extents.
    Aabb {
        /// Half width and half height.
        half: Vec2Fx,
    },
    /// A circle centred on the body's origin.
    Circle {
        /// Radius.
        radius: Fx,
    },
    /// A convex hull, counter-clockwise, in local space.
    Polygon {
        /// Hull vertices.
        points: Vec<Vec2Fx>,
    },
}

impl Shape {
    /// A box from full extents.
    pub fn box_of(size: Vec2Fx) -> Shape {
        Shape::Aabb { half: size / 2 }
    }

    /// The bounding box of this shape placed at `pos`.
    pub fn bounds_at(&self, pos: Vec2Fx) -> Rect {
        match self {
            Shape::Aabb { half } => Rect::from_center(pos, *half),
            Shape::Circle { radius } => Rect::from_center(pos, Vec2Fx::new(*radius, *radius)),
            Shape::Polygon { points } => {
                if points.is_empty() {
                    return Rect::new(pos, Vec2Fx::ZERO);
                }
                let mut min = points[0];
                let mut max = points[0];
                for p in &points[1..] {
                    min = min.min(*p);
                    max = max.max(*p);
                }
                Rect::from_corners(pos + min, pos + max)
            }
        }
    }

    /// The half extents of this shape's bounding box.
    pub fn bounds_half(&self) -> Vec2Fx {
        self.bounds_at(Vec2Fx::ZERO).half_size()
    }

    /// True when `point` is inside the shape placed at `pos`.
    pub fn contains(&self, pos: Vec2Fx, point: Vec2Fx) -> bool {
        let local = point - pos;
        match self {
            Shape::Aabb { half } => local.x.abs() <= half.x && local.y.abs() <= half.y,
            Shape::Circle { radius } => local.length_squared() <= radius.wide() * radius.wide(),
            Shape::Polygon { points } => {
                if points.len() < 3 {
                    return false;
                }
                // Inside a counter-clockwise convex hull means left of every
                // edge. One sign test per edge, no square roots.
                points.iter().enumerate().all(|(i, p)| {
                    let q = points[(i + 1) % points.len()];
                    (q - *p).cross(local - *p) >= FxWide::ZERO
                })
            }
        }
    }
}

/// A contact between two bodies.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Contact {
    /// Unit vector pointing out of the second body, toward the first.
    pub normal: Vec2Fx,
    /// How far the two shapes overlap along the normal.
    pub depth: Fx,
}

/// Test two shapes for overlap and, when they overlap, by how much.
///
/// Returns the minimum translation that separates `a` from `b`.
pub fn overlap(a: &Shape, a_pos: Vec2Fx, b: &Shape, b_pos: Vec2Fx) -> Option<Contact> {
    match (a, b) {
        (Shape::Aabb { half: ha }, Shape::Aabb { half: hb }) => aabb_aabb(a_pos, *ha, b_pos, *hb),
        (Shape::Circle { radius: ra }, Shape::Circle { radius: rb }) => {
            circle_circle(a_pos, *ra, b_pos, *rb)
        }
        (Shape::Circle { radius }, Shape::Aabb { half }) => {
            circle_aabb(a_pos, *radius, b_pos, *half)
        }
        (Shape::Aabb { half }, Shape::Circle { radius }) => {
            circle_aabb(b_pos, *radius, a_pos, *half).map(flip)
        }
        // Polygons fall back to their bounding boxes. Exact convex-hull
        // separation is a separating-axis pass that nothing in the vertical
        // slice needs yet; when a shipped game demands it, it goes here and
        // nothing above this function changes.
        _ => aabb_aabb(a_pos, a.bounds_half(), b_pos, b.bounds_half()),
    }
}

fn flip(c: Contact) -> Contact {
    Contact {
        normal: -c.normal,
        depth: c.depth,
    }
}

fn aabb_aabb(a_pos: Vec2Fx, a_half: Vec2Fx, b_pos: Vec2Fx, b_half: Vec2Fx) -> Option<Contact> {
    let delta = a_pos - b_pos;
    let overlap_x = (a_half.x + b_half.x) - delta.x.abs();
    if overlap_x <= Fx::ZERO {
        return None;
    }
    let overlap_y = (a_half.y + b_half.y) - delta.y.abs();
    if overlap_y <= Fx::ZERO {
        return None;
    }
    // Separate along whichever axis needs the least movement.
    if overlap_x < overlap_y {
        Some(Contact {
            normal: Vec2Fx::new(sign_or_positive(delta.x), Fx::ZERO),
            depth: overlap_x,
        })
    } else {
        Some(Contact {
            normal: Vec2Fx::new(Fx::ZERO, sign_or_positive(delta.y)),
            depth: overlap_y,
        })
    }
}

fn circle_circle(a: Vec2Fx, ra: Fx, b: Vec2Fx, rb: Fx) -> Option<Contact> {
    let delta = a - b;
    let sum = ra + rb;
    if delta.length_squared() >= sum.wide() * sum.wide() {
        return None;
    }
    let distance = delta.length();
    if distance.is_zero() {
        // Concentric: pick a fixed axis rather than a random one, so two
        // perfectly overlapping bodies separate the same way every replay.
        return Some(Contact {
            normal: Vec2Fx::Y,
            depth: sum,
        });
    }
    Some(Contact {
        normal: delta.normalized(),
        depth: sum - distance,
    })
}

fn circle_aabb(c: Vec2Fx, radius: Fx, b: Vec2Fx, half: Vec2Fx) -> Option<Contact> {
    let closest = Rect::from_center(b, half).closest_point(c);
    let delta = c - closest;
    let dist_sq = delta.length_squared();
    if dist_sq >= radius.wide() * radius.wide() {
        return None;
    }
    if dist_sq == FxWide::ZERO {
        // The centre is inside the box: push out along the shallowest face.
        let local = c - b;
        let dx = half.x - local.x.abs();
        let dy = half.y - local.y.abs();
        return Some(if dx < dy {
            Contact {
                normal: Vec2Fx::new(sign_or_positive(local.x), Fx::ZERO),
                depth: dx + radius,
            }
        } else {
            Contact {
                normal: Vec2Fx::new(Fx::ZERO, sign_or_positive(local.y)),
                depth: dy + radius,
            }
        });
    }
    let distance = delta.length();
    Some(Contact {
        normal: delta.normalized(),
        depth: radius - distance,
    })
}

/// The sign of `v`, treating exactly zero as positive.
///
/// A zero here means two bodies are exactly aligned on an axis. Something has
/// to break the tie, and it has to break it the same way on every machine.
fn sign_or_positive(v: Fx) -> Fx {
    if v < Fx::ZERO {
        Fx::NEG_ONE
    } else {
        Fx::ONE
    }
}
