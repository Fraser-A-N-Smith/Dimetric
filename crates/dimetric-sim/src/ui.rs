//! UI interaction, inside the tick.
//!
//! The geometry half of this lives in `dimetric_scene::ui`: anchors, offsets,
//! and a layout computed against a fixed canvas. This is the half that decides
//! what the player is *doing* to it.
//!
//! # Why this is in the simulation at all
//!
//! It would be less work to hit-test at the render boundary, where the pointer
//! and the rectangles both already exist, and hand the simulation the answer.
//! That is the wrong side of the line for the same reason the layout is:
//! a click on a button is an action a replay has to reproduce. If the
//! renderer decides what was clicked, then a replay run headlessly — which is
//! how the fixtures run — has nothing to decide it with, and a recorded run
//! through a menu cannot be played back at all.
//!
//! So the pointer crosses the boundary as data ([`crate::PlayerInput::pointer`],
//! in canvas pixels), the simulation lays the UI out itself, and what it
//! concludes is state like any other: snapshotted, hashed, and restored by a
//! rollback.
//!
//! # Press capture
//!
//! A click is a press and a release on the *same* control. Pressing a button
//! and sliding off it before letting go is a cancelled click, which is a
//! convention old enough that violating it reads as a bug. That requires
//! remembering which control took the press, so [`UiState::pressed`] outlives
//! the tick it started in.

use dimetric_core::{HashState, NodeUid, StateHasher, Vec2Fx};
use dimetric_scene::ui::{hit, layout, Canvas};
use dimetric_scene::{Scene, Value};

use crate::input::{buttons, InputFrame};

/// What the player is doing to the UI.
///
/// Hashed and snapshotted: a menu is part of the game, and a rollback that
/// landed with a button still visually held would be a rollback that did not
/// restore the game.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct UiState {
    /// The control under the pointer.
    pub hovered: Option<NodeUid>,
    /// The control the pointer went down on, until it comes back up.
    ///
    /// Held across ticks on purpose: a click belongs to whatever took the
    /// press, and letting go somewhere else cancels it rather than clicking
    /// the control that happens to be under the pointer at the time.
    pub pressed: Option<NodeUid>,
    /// Controls clicked this tick, cleared at the start of the next.
    ///
    /// A list rather than an `Option` because a script may ask about several
    /// buttons, and because two players may click two different controls on
    /// the same tick.
    pub clicked: Vec<NodeUid>,
    /// The control with keyboard focus, if any.
    ///
    /// Moved by scripts rather than by the engine. Which key walks a menu is a
    /// game's decision — binding it here would mean the engine deciding that
    /// pressing Down in a menu must never also move the player, which is not
    /// the engine's call to make.
    pub focused: Option<NodeUid>,
    /// True when the pointer is over a control that catches input.
    ///
    /// The flag a game checks before firing a weapon at a click that was meant
    /// for a button. Reported rather than enforced: the engine does not
    /// silently swallow the input, because a script that never asked would
    /// then have no way to find out why its fire button stopped working.
    pub captured: bool,
}

/// Nothing is happening to this control.
pub const STATE_IDLE: i64 = 0;
/// The pointer is over it.
pub const STATE_HOVERED: i64 = 1;
/// It is being held.
pub const STATE_PRESSED: i64 = 2;

impl UiState {
    /// True when this control was clicked on this tick.
    pub fn was_clicked(&self, node: NodeUid) -> bool {
        self.clicked.contains(&node)
    }
}

impl HashState for UiState {
    fn hash_state(&self, h: &mut StateHasher) {
        h.tag("ui");
        match self.hovered {
            Some(uid) => h.bool(true).node_uid(uid),
            None => h.bool(false),
        };
        match self.pressed {
            Some(uid) => h.bool(true).node_uid(uid),
            None => h.bool(false),
        };
        match self.focused {
            Some(uid) => h.bool(true).node_uid(uid),
            None => h.bool(false),
        };
        h.bool(self.captured).len(self.clicked.len());
        for uid in &self.clicked {
            h.node_uid(*uid);
        }
    }
}

/// Recompute hover, press and click for this tick.
///
/// Runs after input is latched and before scripts do, so a script asking
/// whether its button was clicked is asking about this tick's pointer rather
/// than the last one's.
pub fn update(
    scene_mut: &mut Scene,
    ui: &mut UiState,
    input: &InputFrame,
    previous: &InputFrame,
    canvas: Canvas,
) {
    ui.clicked.clear();

    // Player zero drives the UI. A second pointer wants a second cursor and a
    // rule for what happens when two of them press the same button, and
    // inventing that before there is a game with two cursors is how an input
    // system grows a feature nobody asked for.
    let now = input.player(0);
    let before = previous.player(0);

    let rects = layout(scene_mut, canvas);
    let over = hit(scene_mut, &rects, now.pointer);
    ui.hovered = over.and_then(|id| scene_mut.get(id)).map(|n| n.uid);
    ui.captured = over.is_some();

    let down = now.held(buttons::FIRE);
    let was_down = before.held(buttons::FIRE);

    if down && !was_down {
        // The press is captured by whatever is under the pointer, including
        // nothing: pressing on empty space and releasing over a button must
        // not click it.
        ui.pressed = ui.hovered;
    } else if !down && was_down {
        if let Some(pressed) = ui.pressed {
            if ui.hovered == Some(pressed) {
                ui.clicked.push(pressed);
            }
        }
        ui.pressed = None;
    }

    // Publish what the pointer is doing onto the controls themselves, so the
    // renderer can colour a button without the render crate needing to know
    // that a simulation exists. Same route every other sim decision takes to
    // the screen: a node property.
    let ids: Vec<_> = rects.keys().copied().collect();
    for id in ids {
        let Some(node) = scene_mut.get_mut(id) else {
            continue;
        };
        let want = if ui.pressed == Some(node.uid) && ui.hovered == Some(node.uid) {
            STATE_PRESSED
        } else if ui.hovered == Some(node.uid) {
            STATE_HOVERED
        } else {
            STATE_IDLE
        };
        // Written only on change: every write is a scene mutation, and a menu
        // sitting still should not churn the tree sixty times a second.
        if node.get("state").and_then(Value::as_int) != Some(want) {
            node.set("state", Value::Int(want));
        }
    }

    // A control that vanished mid-press — a menu closing under the pointer —
    // cannot be clicked, and holding its uid would let a later node reusing
    // that slot inherit the press.
    if let Some(pressed) = ui.pressed {
        if !alive(scene_mut, pressed) {
            ui.pressed = None;
        }
    }
    if let Some(focused) = ui.focused {
        if !alive(scene_mut, focused) {
            ui.focused = None;
        }
    }
}

/// Whether a uid still names a node in the tree.
fn alive(scene: &Scene, uid: NodeUid) -> bool {
    scene.contains_uid(uid)
}

/// Every focusable control, in tree order.
///
/// Tree order rather than anything geometric: it is the order the scene
/// declares, it is stable when a control moves, and it is the order a reader
/// of the `.dim` file would predict.
pub fn focus_order(scene: &Scene, canvas: Canvas) -> Vec<NodeUid> {
    let rects = layout(scene, canvas);
    let mut out = Vec::new();
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        if !rects.contains_key(&id) || !node.visible {
            continue;
        }
        if node
            .get("focusable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            out.push(node.uid);
        }
    }
    out
}

/// The focusable control after `from`, wrapping at the end.
pub fn next_focus(
    scene: &Scene,
    canvas: Canvas,
    from: Option<NodeUid>,
    step: i32,
) -> Option<NodeUid> {
    let order = focus_order(scene, canvas);
    if order.is_empty() {
        return None;
    }
    let index = match from.and_then(|uid| order.iter().position(|o| *o == uid)) {
        Some(i) => i as i32 + step,
        // Nothing focused yet: stepping forward lands on the first control and
        // stepping back on the last, which is what a player pressing Up as
        // their first action expects.
        None if step >= 0 => 0,
        None => order.len() as i32 - 1,
    };
    let len = order.len() as i32;
    Some(order[index.rem_euclid(len) as usize])
}

/// Where the pointer is, for a script that wants the raw value.
pub fn pointer(input: &InputFrame) -> Vec2Fx {
    input.player(0).pointer
}
