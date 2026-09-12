//! Swept collision against axis-aligned boxes.
//!
//! Swept rather than discrete because a discrete test is what lets a fast
//! projectile pass through a wall between two ticks, and "just raise the tick
//! rate" is not a fix — it changes the answer instead of correcting it.

use dimetric_core::{Fx, Vec2Fx};

/// When and how a sweep hit something.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hit {
    /// Fraction of the motion travelled before contact, in `0.0 ..= 1.0`.
    pub toi: Fx,
    /// Unit surface normal, pointing back toward the mover.
    pub normal: Vec2Fx,
}

/// How far off a surface a resolved mover is left.
///
/// Landing exactly on a surface makes the next tick's overlap test a coin
/// flip on the rounding of one bit. Backing off by a hair makes it decisive.
pub const SKIN: Fx = Fx::from_raw(8);

/// Sweep a box from `start` along `motion` against a static box.
///
/// Both boxes are given by centre and half extents. Returns `None` when the
/// motion does not reach the target within this step.
pub fn sweep_aabb(
    start: Vec2Fx,
    half: Vec2Fx,
    motion: Vec2Fx,
    target: Vec2Fx,
    target_half: Vec2Fx,
) -> Option<Hit> {
    // Minkowski sum: grow the target by the mover's half extents and sweep a
    // point instead of a box.
    let grown = half + target_half;
    let min = target - grown;
    let max = target + grown;

    let (entry_x, exit_x) = axis_times(start.x, motion.x, min.x, max.x)?;
    let (entry_y, exit_y) = axis_times(start.y, motion.y, min.y, max.y)?;

    let entry = entry_x.max(entry_y);
    let exit = exit_x.min(exit_y);

    if entry > exit || entry >= Fx::ONE || exit <= Fx::ZERO {
        return None;
    }

    // A negative entry time means the boxes already overlap. Report contact at
    // the start of the step and let the caller depenetrate.
    let toi = entry.max(Fx::ZERO);
    let normal = if entry_x > entry_y {
        Vec2Fx::new(opposing(motion.x), Fx::ZERO)
    } else {
        Vec2Fx::new(Fx::ZERO, opposing(motion.y))
    };
    Some(Hit { toi, normal })
}

/// Entry and exit times for one axis, or `None` when the axis never overlaps.
fn axis_times(start: Fx, motion: Fx, min: Fx, max: Fx) -> Option<(Fx, Fx)> {
    if motion.is_zero() {
        // No movement on this axis: either it already overlaps for the whole
        // step, or it never will.
        if start <= min || start >= max {
            return None;
        }
        return Some((Fx::MIN, Fx::MAX));
    }
    // Saturating division on purpose: a tiny motion against a large distance
    // produces a time far outside 0..1, which is exactly the right answer
    // ("not within this step") and must not trip the overflow assertion.
    let t1 = (min - start).saturating_div(motion);
    let t2 = (max - start).saturating_div(motion);
    Some((t1.min(t2), t1.max(t2)))
}

/// The unit normal opposing motion along one axis.
fn opposing(motion: Fx) -> Fx {
    if motion > Fx::ZERO {
        Fx::NEG_ONE
    } else {
        Fx::ONE
    }
}
