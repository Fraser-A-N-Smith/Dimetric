//! Cosmetic tweens: squash, flash, drift.
//!
//! # Why these are simulation state
//!
//! "Cosmetic" describes what a tween is *for* — the juice a game needs and the
//! physics does not — rather than where it lives. A tween writes to a node
//! property, and node properties are simulation state, so a tween is too: it is
//! snapshotted, it is hashed, and it advances on ticks.
//!
//! The alternative, a presentation-side tween outside the hash, sounds tidier
//! and is not. It could not be snapshotted, so a rollback would leave every
//! tween mid-flight writing to positions that had just been rewound.
//!
//! # Everything here is fixed point
//!
//! Easing included. A curve evaluated in floating point would put a
//! platform-dependent number into a hashed property, which is exactly the
//! failure mode I3 exists to prevent.

use std::collections::BTreeMap;

use dimetric_core::{Angle, Fx, NodeUid, StateHasher, Vec2Fx};
use dimetric_scene::{Color, Value};

/// How a tween gets from one value to the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Easing {
    /// Constant rate.
    #[default]
    Linear,
    /// Slow at the start.
    EaseIn,
    /// Slow at the end.
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
}

impl Easing {
    /// Parse the name a script uses.
    pub fn parse(name: &str) -> Option<Easing> {
        Some(match name {
            "linear" => Easing::Linear,
            "ease_in" => Easing::EaseIn,
            "ease_out" => Easing::EaseOut,
            "ease_in_out" => Easing::EaseInOut,
            _ => return None,
        })
    }

    /// The name a script uses.
    pub fn name(self) -> &'static str {
        match self {
            Easing::Linear => "linear",
            Easing::EaseIn => "ease_in",
            Easing::EaseOut => "ease_out",
            Easing::EaseInOut => "ease_in_out",
        }
    }

    /// Shape a `0..=1` progress value. Fixed point throughout.
    pub fn shape(self, t: Fx) -> Fx {
        let t = t.clamp(Fx::ZERO, Fx::ONE);
        let inverse = Fx::ONE - t;
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => Fx::ONE - inverse * inverse,
            Easing::EaseInOut => {
                let half = Fx::ONE / 2;
                if t < half {
                    Fx::from_int(2) * t * t
                } else {
                    Fx::ONE - Fx::from_int(2) * inverse * inverse
                }
            }
        }
    }
}

/// One property on its way from one value to another.
#[derive(Clone, PartialEq, Debug)]
pub struct Tween {
    /// Property being written.
    pub property: String,
    /// Where it started.
    pub from: Value,
    /// Where it is going.
    pub to: Value,
    /// Ticks elapsed.
    pub elapsed: u32,
    /// Ticks the whole thing takes.
    pub ticks: u32,
    /// The curve.
    pub easing: Easing,
}

impl Tween {
    /// Progress as a fixed-point fraction.
    pub fn progress(&self) -> Fx {
        if self.ticks == 0 {
            return Fx::ONE;
        }
        Fx::from_int(self.elapsed.min(self.ticks) as i32) / self.ticks as i32
    }

    /// The value now.
    pub fn value(&self) -> Value {
        interpolate(&self.from, &self.to, self.easing.shape(self.progress()))
    }

    /// Whether it has arrived.
    pub fn is_done(&self) -> bool {
        self.elapsed >= self.ticks
    }

    /// Feed into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.str(&self.property)
            .u64(self.elapsed as u64)
            .u64(self.ticks as u64)
            .str(self.easing.name());
        self.from.hash_state(h);
        self.to.hash_state(h);
    }
}

/// Every tween a node is running, newest last.
pub type Tweens = BTreeMap<NodeUid, Vec<Tween>>;

/// Move every tween on by one tick and write the results into the scene.
///
/// Finished tweens are removed after their final value is written, so a tween
/// always lands exactly on its target rather than near it.
pub fn advance(scene: &mut dimetric_scene::Scene, tweens: &mut Tweens) {
    let uids: Vec<NodeUid> = tweens.keys().copied().collect();
    for uid in uids {
        let Some(list) = tweens.get_mut(&uid) else {
            continue;
        };
        for tween in list.iter_mut() {
            tween.elapsed = tween.elapsed.saturating_add(1);
        }
        let writes: Vec<(String, Value)> = list
            .iter()
            .map(|t| (t.property.clone(), t.value()))
            .collect();
        list.retain(|t| !t.is_done());
        if list.is_empty() {
            tweens.remove(&uid);
        }
        let Some(id) = scene.by_uid(uid) else {
            // The node was destroyed mid-tween. Dropping the tween rather than
            // resurrecting anything.
            tweens.remove(&uid);
            continue;
        };
        for (property, value) in writes {
            write(scene, id, &property, value);
        }
    }
}

/// Write one tweened property, reserved keys included.
fn write(
    scene: &mut dimetric_scene::Scene,
    id: dimetric_core::NodeId,
    property: &str,
    value: Value,
) {
    let Some(node) = scene.get_mut(id) else {
        return;
    };
    let moved = match (property, &value) {
        ("pos", Value::Vec2(v)) => {
            node.transform.pos = *v;
            true
        }
        ("scale", Value::Vec2(v)) => {
            node.transform.scale = *v;
            true
        }
        ("rot", Value::Angle(a)) => {
            node.transform.rot = *a;
            true
        }
        _ => {
            node.props.insert(property.to_string(), value);
            false
        }
    };
    // Writing through the node bypasses the setters that invalidate, so the
    // cached world transforms have to be told by hand.
    if moved {
        scene.mark_subtree_dirty(id);
    }
}

/// Whether two values are the same shape, and therefore tweenable between.
pub fn can_tween(from: &Value, to: &Value) -> bool {
    matches!(
        (from, to),
        (Value::Scalar(_), Value::Scalar(_))
            | (Value::Vec2(_), Value::Vec2(_))
            | (Value::Angle(_), Value::Angle(_))
            | (Value::Color(_), Value::Color(_))
    )
}

/// Interpolate between two values of the same shape.
///
/// Anything else holds `from` until the tween ends and then snaps: a tween
/// between a string and a number has no sensible midpoint, and refusing to
/// invent one is better than inventing a wrong one.
pub fn interpolate(from: &Value, to: &Value, t: Fx) -> Value {
    match (from, to) {
        (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(a.lerp(*b, t)),
        (Value::Vec2(a), Value::Vec2(b)) => {
            Value::Vec2(Vec2Fx::new(a.x.lerp(b.x, t), a.y.lerp(b.y, t)))
        }
        (Value::Angle(a), Value::Angle(b)) => Value::Angle(lerp_angle(*a, *b, t)),
        (Value::Color(a), Value::Color(b)) => Value::Color(lerp_color(*a, *b, t)),
        _ if t >= Fx::ONE => to.clone(),
        _ => from.clone(),
    }
}

/// Interpolate around the short way.
///
/// Turning 350° to 10° is twenty degrees, not three hundred and forty. Binary
/// angles wrap on their own, so the short way is what the arithmetic already
/// does — the cast through `i16` is what makes it signed.
fn lerp_angle(a: Angle, b: Angle, t: Fx) -> Angle {
    let delta = b.to_bam().wrapping_sub(a.to_bam()) as i16;
    let moved = (Fx::from_int(delta as i32) * t).round_int();
    Angle::from_bam(a.to_bam().wrapping_add(moved as u16))
}

fn lerp_color(a: Color, b: Color, t: Fx) -> Color {
    let channel = |x: u8, y: u8| -> u8 {
        let moved = Fx::from_int(x as i32).lerp(Fx::from_int(y as i32), t);
        moved.round_int().clamp(0, 255) as u8
    };
    Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: channel(a.a, b.a),
    }
}
