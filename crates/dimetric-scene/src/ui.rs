//! Laying out a tree of `Control` nodes.
//!
//! # Why layout is not allowed to know the window size
//!
//! The obvious way to lay out UI is against the window: a button anchored to
//! the right edge sits wherever the right edge currently is. Do that here and
//! the engine's central promise breaks, because a game's UI is not only drawn
//! — it is clicked. If where a button lands depends on how big somebody's
//! window happens to be, then whether a click hit it does too, and two players
//! running the same input log on differently-sized windows play different
//! games.
//!
//! So layout runs against a **canvas**: a fixed virtual resolution the project
//! declares once. The renderer scales that canvas to whatever window it is
//! given, which is what a pixel-art game wants anyway. Layout is then a pure
//! function of the tree and a constant, and is the same on every machine.
//!
//! # Anchors and offsets
//!
//! Each edge of a control is `anchor * parent_extent + offset`. An anchor is a
//! fraction of the parent, so `0` is its left or top edge and `1` its right or
//! bottom; an offset is whole pixels from there. Anchoring both sides to `0`
//! and offsetting them apart gives a fixed-size box pinned to the top-left;
//! anchoring `0` and `1` gives one that stretches with its parent. It is the
//! same scheme Godot uses, for the good reason that it covers both cases
//! without a mode switch.

use std::collections::BTreeMap;

use dimetric_core::{Fx, NodeId, Rect, Vec2Fx};

use crate::tree::Scene;
use crate::value::Value;

/// The virtual resolution UI is laid out against.
///
/// Not the window. See the module note: a layout that moved with the window
/// would make a click's outcome depend on the window, and a replay would stop
/// reproducing on a different monitor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Canvas {
    /// Width in virtual pixels.
    pub width: i32,
    /// Height in virtual pixels.
    pub height: i32,
}

impl Default for Canvas {
    fn default() -> Canvas {
        // 320x180 is 16:9 and scales to 1280x720 and 1920x1080 by whole
        // numbers, which keeps pixel art sharp at the sizes people actually
        // run games at.
        Canvas {
            width: 320,
            height: 180,
        }
    }
}

impl Canvas {
    /// The whole canvas as a rectangle.
    pub fn rect(&self) -> Rect {
        Rect {
            pos: Vec2Fx::ZERO,
            size: Vec2Fx::from_ints(self.width, self.height),
        }
    }
}

/// Where every control ended up.
pub type Layout = BTreeMap<NodeId, Rect>;

/// Lay out every `Control` in the scene against a canvas.
///
/// Parents are laid out before their children, which the depth-first walk
/// gives for free. A control whose parent is not a control is laid out against
/// the canvas itself, so a UI tree can hang anywhere in the scene without
/// needing a root of a special kind.
pub fn layout(scene: &Scene, canvas: Canvas) -> Layout {
    let mut out: Layout = BTreeMap::new();
    // Tracks how far down (or across) each container has filled, so a child of
    // a box is placed after its earlier siblings rather than on top of them.
    let mut filled: BTreeMap<NodeId, Fx> = BTreeMap::new();

    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        if node.base != "Control" {
            continue;
        }
        let parent_id = node.parent();
        let parent = parent_id
            .and_then(|p| out.get(&p).copied())
            .unwrap_or_else(|| canvas.rect());

        // A child of a box is stacked, not anchored. The box owns the axis it
        // stacks along and the child keeps its own extent on the other, which
        // is the split that makes a row of buttons need only a height each.
        let axis = parent_id.and_then(|p| scene.get(p)).and_then(box_axis);
        let rect = match (axis, parent_id) {
            (Some((axis, spacing, padding)), Some(pid)) => {
                let used = filled.entry(pid).or_insert(Fx::ZERO);
                let placed = stacked(node, parent, axis, *used, padding);
                // The next sibling starts past this one and its gap.
                *used = *used
                    + match axis {
                        Axis::Vertical => placed.size.y,
                        Axis::Horizontal => placed.size.x,
                    }
                    + spacing;
                placed
            }
            _ => rect_of(node, parent),
        };
        out.insert(id, rect);
    }
    out
}

/// Which way a container stacks, and by how much, if it is one.
fn box_axis(node: &crate::node::Node) -> Option<(Axis, Fx, Fx)> {
    let axis = match node.kind.as_str() {
        "VBox" => Axis::Vertical,
        "HBox" => Axis::Horizontal,
        _ => return None,
    };
    let scalar = |name: &str| {
        node.get(name)
            .and_then(Value::as_scalar)
            .unwrap_or(Fx::ZERO)
    };
    Some((axis, scalar("spacing"), scalar("padding")))
}

/// The direction a container lays its children out in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    /// Top to bottom.
    Vertical,
    /// Left to right.
    Horizontal,
}

/// One child of a box: placed after its earlier siblings, filling the cross
/// axis.
///
/// A child in a box does not get to choose its position — that is what putting
/// it in a box means — but it keeps its size along the stacking axis, taken
/// from its own offsets. So a `VBox` of buttons needs a height on each button
/// and nothing else, which is the thing a container is supposed to save you.
fn stacked(node: &crate::node::Node, parent: Rect, axis: Axis, used: Fx, padding: Fx) -> Rect {
    let scalar = |name: &str| {
        node.get(name)
            .and_then(Value::as_scalar)
            .unwrap_or(Fx::ZERO)
    };
    let inner = parent.size - Vec2Fx::new(padding + padding, padding + padding);
    match axis {
        Axis::Vertical => {
            let height = (scalar("offset_bottom") - scalar("offset_top")).max(Fx::ZERO);
            Rect {
                pos: Vec2Fx::new(parent.pos.x + padding, parent.pos.y + padding + used),
                size: Vec2Fx::new(inner.x.max(Fx::ZERO), height),
            }
        }
        Axis::Horizontal => {
            let width = (scalar("offset_right") - scalar("offset_left")).max(Fx::ZERO);
            Rect {
                pos: Vec2Fx::new(parent.pos.x + padding + used, parent.pos.y + padding),
                size: Vec2Fx::new(width, inner.y.max(Fx::ZERO)),
            }
        }
    }
}

/// One control's rectangle, given its parent's.
fn rect_of(node: &crate::node::Node, parent: Rect) -> Rect {
    let anchor = |name: &str| {
        node.get(name)
            .and_then(Value::as_scalar)
            .unwrap_or(Fx::ZERO)
    };
    let offset = |name: &str| {
        node.get(name)
            .and_then(Value::as_scalar)
            .unwrap_or(Fx::ZERO)
    };

    let left = parent.pos.x + parent.size.x * anchor("anchor_left") + offset("offset_left");
    let top = parent.pos.y + parent.size.y * anchor("anchor_top") + offset("offset_top");
    let right = parent.pos.x + parent.size.x * anchor("anchor_right") + offset("offset_right");
    let bottom = parent.pos.y + parent.size.y * anchor("anchor_bottom") + offset("offset_bottom");

    // A right edge left of the left edge is a mistake somebody made in a
    // scene, and a negative size would draw a quad inside out. Clamping to
    // zero makes it vanish, which is visible and harmless, rather than
    // rendering something nobody can explain.
    Rect {
        pos: Vec2Fx::new(left, top),
        size: Vec2Fx::new((right - left).max(Fx::ZERO), (bottom - top).max(Fx::ZERO)),
    }
}

/// The topmost control containing a point, if any.
///
/// Later siblings sit on top of earlier ones — the same order they draw in —
/// so this walks the layout backwards and takes the first hit. Ties are broken
/// by tree order rather than by anything about the geometry, which is what
/// makes the answer the same on every machine.
pub fn hit(scene: &Scene, layout: &Layout, point: Vec2Fx) -> Option<NodeId> {
    let mut found = None;
    for id in scene.walk() {
        let Some(rect) = layout.get(&id) else {
            continue;
        };
        let Some(node) = scene.get(id) else { continue };
        if !node.visible {
            continue;
        }
        // A control that does not take input is scenery: a panel behind a row
        // of buttons should not swallow the clicks meant for them.
        let catches = node
            .get("catches_input")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if catches && rect.contains(point) {
            found = Some(id);
        }
    }
    found
}
