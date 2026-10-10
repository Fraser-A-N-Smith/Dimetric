//! Neither the pointer nor the keyboard lands inside a closed panel.
//!
//! `ui::hit` reading a control's own `visible` and nothing else is the defect
//! `dimetric-scene`'s `hidden_parent` test pins. These are its consequences one
//! layer up, where they are actually felt: `ui.hovered` naming a control nobody
//! can see, and focus staying on a row inside a screen that has just closed.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{
    input::{InputFrame, PlayerInput},
    NoScripts, Sim, SimConfig,
};

/// A full-canvas `Screen` with two focusable rows, over a board that is itself
/// a control and takes the clicks the screen does not.
const SHEET: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Run"

[[node]]
id = "n_board000"
kind = "Panel"
name = "Board"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 1.0

[[node]]
id = "n_screen00"
kind = "Panel"
name = "Screen"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 1.0

[[node]]
id = "n_row40000"
kind = "Button"
name = "Row4"
parent = "n_screen00"
anchor_right = 1.0
anchor_bottom = 1.0
focusable = true

[[node]]
id = "n_row50000"
kind = "Button"
name = "Row5"
parent = "n_screen00"
offset_left = 0.0
offset_top = 0.0
offset_right = 10.0
offset_bottom = 10.0
focusable = true
"##;

fn load(src: &str) -> Scene {
    let out = dimetric_scene::parse(src, "sheet.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim() -> Sim {
    Sim::new(load(SHEET), 1, Box::new(NoScripts), SimConfig::default())
}

/// The pointer in the middle of the canvas, no button down.
fn frame() -> InputFrame {
    let canvas = SimConfig::default().canvas;
    InputFrame {
        players: vec![PlayerInput {
            pointer: Vec2Fx::from_ints(canvas.width / 2, canvas.height / 2),
            ..PlayerInput::default()
        }],
        ..InputFrame::default()
    }
}

fn uid(sim: &Sim, path: &str) -> NodeUid {
    let state = sim.state();
    let id = state.scene.resolve_path(path).expect("node");
    state.scene.get(id).expect("node").uid
}

/// Hide or show a node by path, between ticks, as a script would.
///
/// Through snapshot and restore, which is the engine's own route into state
/// from outside a tick — and incidentally says that none of this is latched
/// anywhere a snapshot would miss.
fn set_visible(sim: &mut Sim, path: &str, on: bool) {
    let mut state = sim.snapshot();
    let id = state
        .scene
        .resolve_path(path)
        .unwrap_or_else(|| panic!("{path}"));
    state.scene.get_mut(id).unwrap().visible = on;
    sim.restore(state);
}

/// Put focus on a node by uid, from outside a tick.
fn focus(sim: &mut Sim, uid: NodeUid) {
    let mut state = sim.snapshot();
    state.ui.focused = Some(uid);
    sim.restore(state);
}

#[test]
fn the_pointer_stops_naming_rows_in_a_closed_screen() {
    let mut sim = sim();
    sim.step(frame());
    assert_eq!(
        sim.state().ui.hovered,
        Some(uid(&sim, "/Run/Screen/Row4")),
        "while the screen is open the row is what is under the pointer"
    );

    // Closed the way a game closes it: the screen's own flag, nothing else.
    set_visible(&mut sim, "/Run/Screen", false);
    sim.step(frame());
    assert_eq!(
        sim.state().ui.hovered,
        Some(uid(&sim, "/Run/Board")),
        "the board underneath should take the pointer back"
    );
}

#[test]
fn focus_does_not_stay_inside_a_screen_that_closes() {
    // A screen closed while one of its rows had focus would otherwise keep the
    // keyboard inside a panel nobody can see.
    let mut sim = sim();
    sim.step(frame());
    let row = uid(&sim, "/Run/Screen/Row4");
    focus(&mut sim, row);
    sim.step(frame());
    assert_eq!(
        sim.state().ui.focused,
        Some(row),
        "still open, still focused"
    );

    set_visible(&mut sim, "/Run/Screen", false);
    sim.step(frame());
    assert_eq!(sim.state().ui.focused, None, "focus let go of it");
}

#[test]
fn the_focus_order_skips_a_closed_screens_rows() {
    let mut sim = sim();
    sim.step(frame());
    let canvas = sim.state().canvas;
    let open = dimetric_sim::ui::focus_order(&sim.state().scene, canvas);
    assert_eq!(open.len(), 2, "both rows are focusable while open");

    set_visible(&mut sim, "/Run/Screen", false);
    let closed = dimetric_sim::ui::focus_order(&sim.state().scene, canvas);
    assert!(
        closed.is_empty(),
        "neither row should be reachable by Tab: {closed:?}"
    );
}

#[test]
fn tabbing_round_a_closed_screen_finds_nothing_rather_than_a_hidden_row() {
    let mut sim = sim();
    sim.step(frame());
    let canvas = sim.state().canvas;
    set_visible(&mut sim, "/Run/Screen", false);
    assert_eq!(
        dimetric_sim::ui::next_focus(&sim.state().scene, canvas, None, 1),
        None
    );
}

#[test]
fn reopening_the_screen_makes_its_rows_reachable_again() {
    // Nothing is latched: the rows' own flags were never touched.
    let mut sim = sim();
    sim.step(frame());
    set_visible(&mut sim, "/Run/Screen", false);
    sim.step(frame());
    set_visible(&mut sim, "/Run/Screen", true);
    sim.step(frame());
    assert_eq!(sim.state().ui.hovered, Some(uid(&sim, "/Run/Screen/Row4")));
    let canvas = sim.state().canvas;
    assert_eq!(
        dimetric_sim::ui::focus_order(&sim.state().scene, canvas).len(),
        2
    );
}
