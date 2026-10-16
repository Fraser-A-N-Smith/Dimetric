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

/// Which rectangle each node is confined to, where something confines it.
///
/// Keyed by node, and an entry only where an ancestor actually clips — so a UI
/// that uses no clipping produces an empty map and pays nothing. The value is
/// the intersection of every clipping ancestor's rectangle, which is what makes
/// nested clips nest: a row inside a scrolled list inside a dialog is confined
/// to the part of the list that is inside the dialog.
pub type Clips = BTreeMap<NodeId, Rect>;
/// Work out what clips what.
///
/// # Why this is a second map and not a field on the layout
///
/// Because the renderer and the hit test both need it and must not compute it
/// separately. That is the lesson of the hidden-panel bug: the UI hit test read
/// a control's own `visible` where the renderer walked every ancestor, the two
/// disagreed, and a click landed on a row nobody could see. [`Scene::is_visible`]
/// is now the only place that chain is walked, and this is the same door for
/// this chain.
///
/// A node's own `clip_children` does **not** clip the node — it clips what is
/// inside it. A panel that clipped itself to itself would be a no-op on a good
/// day and a vanished panel on a bad one.
///
/// Every node is considered, not only controls: a `Label` parented into the UI
/// tree is drawn by the UI pass and is not a control, and a caption that
/// overflowed the button it belongs to is exactly what a clip is for.
pub fn clips(scene: &Scene, layout: &Layout) -> Clips {
    let mut out: Clips = BTreeMap::new();
    // Depth-first, so a parent's answer is already in `out` when its children
    // are reached. The same property `layout` relies on.
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        let Some(parent_id) = node.parent() else {
            continue;
        };
        // What the parent was already confined to, and what the parent itself
        // adds by asking for its children to be clipped.
        let inherited = out.get(&parent_id).copied();
        let added = match scene.get(parent_id).is_some_and(clips_children) {
            true => layout.get(&parent_id).copied(),
            false => None,
        };
        let confined = match (inherited, added) {
            (Some(outer), Some(inner)) => {
                // No overlap at all is an empty rectangle rather than no
                // entry: nothing inside is visible, and "no entry" would mean
                // the opposite.
                Some(outer.intersection(inner).unwrap_or(Rect {
                    pos: outer.pos,
                    size: Vec2Fx::ZERO,
                }))
            }
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        };
        if let Some(rect) = confined {
            out.insert(id, rect);
        }
    }
    out
}

/// Whether this node confines its descendants to its own rectangle.
///
/// Absent reads as false, which is what every scene written before clipping
/// existed means — and is why the property carries no declared default.
fn clips_children(node: &crate::node::Node) -> bool {
    node.get("clip_children")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The topmost control containing a point, if any.
///
/// Later siblings sit on top of earlier ones — the same order they draw in —
/// so this walks the layout backwards and takes the first hit. Ties are broken
/// by tree order rather than by anything about the geometry, which is what
/// makes the answer the same on every machine.
///
/// # Hidden means hidden, all the way up
///
/// This used to read a control's **own** `visible` and nothing else, where the
/// renderer walks every ancestor. The two disagreed, and a game that closed a
/// screen by hiding the screen got rows that were invisible and still on top:
/// a click in the middle of the board landed on a row nobody could see,
/// `ui.hovered` named it, and the board ignored the click. The alternative
/// offered was hiding every child by hand when a panel closes, which is a
/// second copy of the engine's own visibility rule, written in Lua, in every
/// panel.
///
/// [`Scene::is_visible`] is now the only place that chain is walked, so the
/// renderer and this cannot drift apart again.
///
/// # Clipped means unclickable, too
///
/// A control confined by a clipping ancestor is hit only within the part of it
/// that is actually inside — so a row scrolled out of the top of a list is
/// neither drawn nor clickable. This is the same agreement the visibility rule
/// needed and the same bug if it is missing: a row the player cannot see
/// taking the click meant for the board behind it.
pub fn hit(scene: &Scene, layout: &Layout, point: Vec2Fx) -> Option<NodeId> {
    hit_clipped(scene, layout, &clips(scene, layout), point)
}

/// [`hit`], with the clip map already worked out.
///
/// For a caller that has one — the simulation lays out and clips once per tick
/// and would otherwise walk the tree twice for the same answer.
pub fn hit_clipped(scene: &Scene, layout: &Layout, clips: &Clips, point: Vec2Fx) -> Option<NodeId> {
    let mut found = None;
    for id in scene.walk() {
        let Some(rect) = layout.get(&id) else {
            continue;
        };
        let Some(node) = scene.get(id) else { continue };
        if !scene.is_visible(id) {
            continue;
        }
        // A control that does not take input is scenery: a panel behind a row
        // of buttons should not swallow the clicks meant for them.
        let catches = node
            .get("catches_input")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        // Only the part of it that is inside whatever clips it. A control
        // clipped away to nothing is hit nowhere.
        let reachable = match clips.get(&id) {
            Some(clip) => match rect.intersection(*clip) {
                Some(visible) => visible,
                None => continue,
            },
            None => *rect,
        };
        if catches && reachable.contains(point) {
            found = Some(id);
        }
    }
    found
}
