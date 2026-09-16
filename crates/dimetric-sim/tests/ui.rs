//! Clicking a button, inside the tick.
//!
//! The point of all of this is that none of it needs a renderer. A click is
//! worked out from the pointer in the input frame and the scene's own layout,
//! so a recorded run through a menu replays headlessly like any other.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{
    input::{buttons, InputFrame, PlayerInput},
    LuaHost, NoScripts, Sim, SimConfig, PHASE_ORDER,
};

fn load(src: &str) -> Scene {
    let out = dimetric_scene::parse(src, "ui.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

const MENU: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Menu"

[[node]]
id = "n_start000"
kind = "Panel"
name = "Start"
parent = "n_root0000"
offset_left = 10.0
offset_top = 10.0
offset_right = 90.0
offset_bottom = 30.0
focusable = true

[[node]]
id = "n_quit0000"
kind = "Panel"
name = "Quit"
parent = "n_root0000"
offset_left = 10.0
offset_top = 40.0
offset_right = 90.0
offset_bottom = 60.0
focusable = true
"##;

fn menu_sim() -> Sim {
    Sim::new(load(MENU), 1, Box::new(NoScripts), SimConfig::default())
}

/// One player's frame, with the pointer somewhere and maybe a button down.
fn frame(x: i32, y: i32, down: bool) -> InputFrame {
    InputFrame {
        players: vec![PlayerInput {
            buttons: if down { buttons::FIRE } else { 0 },
            pointer: Vec2Fx::from_ints(x, y),
            ..PlayerInput::default()
        }],
    }
}

fn uid(sim: &Sim, path: &str) -> NodeUid {
    let state = sim.state();
    let id = state.scene.resolve_path(path).expect("node");
    state.scene.get(id).expect("node").uid
}

#[test]
fn the_ui_phase_runs_after_input_and_before_scripts() {
    // A script asking whether its button was clicked has to be asking about
    // this tick's pointer. Getting this backwards makes every menu feel a
    // frame late, which is the kind of bug people describe as "sluggish" and
    // never find.
    let names: Vec<&str> = PHASE_ORDER.iter().map(|p| p.name()).collect();
    let input = names.iter().position(|n| *n == "input").unwrap();
    let ui = names.iter().position(|n| *n == "ui update").unwrap();
    let scripts = names.iter().position(|n| *n == "scripts on_tick").unwrap();
    assert!(input < ui && ui < scripts, "{names:?}");
}

#[test]
fn the_pointer_hovers_what_it_is_over() {
    let mut sim = menu_sim();
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, false));
    assert_eq!(sim.state().ui.hovered, Some(start));

    sim.step(frame(200, 150, false));
    assert_eq!(sim.state().ui.hovered, None);
}

#[test]
fn a_press_and_a_release_on_the_same_control_is_a_click() {
    let mut sim = menu_sim();
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.clicked.is_empty(), "nothing yet");

    sim.step(frame(50, 20, true));
    assert_eq!(sim.state().ui.pressed, Some(start));
    assert!(
        sim.state().ui.clicked.is_empty(),
        "a press is not yet a click"
    );

    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.was_clicked(start));
    assert_eq!(sim.state().ui.pressed, None);
}

#[test]
fn a_click_is_cancelled_by_sliding_off_before_letting_go() {
    // The convention every toolbar has followed for thirty years. Violating it
    // reads as a bug rather than as a design.
    let mut sim = menu_sim();
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, true));
    assert_eq!(sim.state().ui.pressed, Some(start));

    sim.step(frame(200, 150, false));
    assert!(
        sim.state().ui.clicked.is_empty(),
        "releasing off the control must not click it"
    );
}

#[test]
fn releasing_over_a_control_that_did_not_take_the_press_clicks_nothing() {
    // The other half of press capture: pressing on empty space and releasing
    // over a button is not a click on that button.
    let mut sim = menu_sim();

    sim.step(frame(200, 150, true));
    assert_eq!(sim.state().ui.pressed, None);

    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.clicked.is_empty());
}

#[test]
fn a_click_lasts_exactly_one_tick() {
    // Otherwise a menu that starts a game would start it again on every tick
    // the player left the mouse alone.
    let mut sim = menu_sim();
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, true));
    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.was_clicked(start));

    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.clicked.is_empty(), "the click did not clear");
}

#[test]
fn the_capture_flag_says_when_a_click_belongs_to_the_ui() {
    let mut sim = menu_sim();
    sim.step(frame(50, 20, true));
    assert!(sim.state().ui.captured, "the pointer is over a button");

    sim.step(frame(200, 150, true));
    assert!(!sim.state().ui.captured, "and now it is over the world");
}

#[test]
fn ui_state_is_part_of_the_hash() {
    // If it were not, a rollback could land with a button still held and the
    // hash would call that identical.
    let mut a = menu_sim();
    let mut b = menu_sim();
    a.step(frame(50, 20, true));
    b.step(frame(200, 150, true));
    assert_ne!(a.hash(), b.hash(), "a held button must change the hash");
}

#[test]
fn a_snapshot_restores_a_press_in_progress() {
    let mut sim = menu_sim();
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, true));
    let snapshot = sim.snapshot();
    let hash = sim.hash();

    // Let go somewhere else, which cancels the click.
    sim.step(frame(200, 150, false));
    assert!(sim.state().ui.clicked.is_empty());

    // Rewind to mid-press and let go properly this time.
    sim.restore(snapshot);
    assert_eq!(sim.hash(), hash);
    assert_eq!(sim.state().ui.pressed, Some(start));
    sim.step(frame(50, 20, false));
    assert!(sim.state().ui.was_clicked(start));
}

#[test]
fn the_same_pointer_log_replays_to_the_same_hashes() {
    // The whole reason this lives in the simulation.
    let path: Vec<InputFrame> = (0..40)
        .map(|t| frame(20 + t % 80, 20, (t / 5) % 2 == 0))
        .collect();

    let run = |frames: &[InputFrame]| {
        let mut sim = menu_sim();
        frames
            .iter()
            .map(|f| {
                sim.step(f.clone());
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(&path), run(&path));
}

#[test]
fn focus_walks_the_focusable_controls_and_wraps() {
    let sim = menu_sim();
    let (start, quit) = (uid(&sim, "/Menu/Start"), uid(&sim, "/Menu/Quit"));
    let canvas = sim.state().canvas;
    let scene = &sim.state().scene;

    let next = |from| dimetric_sim::ui::next_focus(scene, canvas, from, 1);
    assert_eq!(next(None), Some(start), "first press lands on the first");
    assert_eq!(next(Some(start)), Some(quit));
    assert_eq!(next(Some(quit)), Some(start), "and wraps");

    let prev = |from| dimetric_sim::ui::next_focus(scene, canvas, from, -1);
    assert_eq!(prev(None), Some(quit), "stepping back starts at the end");
    assert_eq!(prev(Some(start)), Some(quit));
}

const CLICK_SCRIPT: &str = r#"
function on_ready(self)
  self.hits = 0
  self.hovered = false
end

function on_tick(self)
  if ui.clicked(self) then
    self.hits = self.hits + 1
  end
  if ui.hovered(self) then
    self.hovered = true
  end
end
"#;

#[test]
fn a_script_can_ask_whether_its_button_was_clicked() {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/btn.lua", CLICK_SCRIPT).expect("loads");
    let scene = load(&MENU.replace(
        r#"name = "Start"
parent = "n_root0000""#,
        r#"name = "Start"
parent = "n_root0000"
script = "script:scripts/btn.lua""#,
    ));
    let mut sim = Sim::new(scene, 1, Box::new(host), SimConfig::default());
    let start = uid(&sim, "/Menu/Start");

    sim.step(frame(50, 20, true));
    sim.step(frame(50, 20, false));

    let state = sim.state();
    let vars = state.vars.get(&start).expect("vars");
    assert_eq!(
        vars.get("hits").and_then(|v| v.as_int()),
        Some(1),
        "the script did not see the click"
    );
    assert_eq!(vars.get("hovered").and_then(|v| v.as_bool()), Some(true));
}

const BUTTON: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Menu"

[[node]]
id = "n_go000000"
kind = "Button"
name = "Go"
parent = "n_root0000"
offset_left = 10.0
offset_top = 10.0
offset_right = 90.0
offset_bottom = 30.0
"##;

fn button_sim() -> Sim {
    Sim::new(load(BUTTON), 1, Box::new(NoScripts), SimConfig::default())
}

fn state_of(sim: &Sim, path: &str) -> i64 {
    let state = sim.state();
    let id = state.scene.resolve_path(path).expect("node");
    state
        .scene
        .get(id)
        .and_then(|n| n.get("state"))
        .and_then(|v| v.as_int())
        .expect("state property")
}

#[test]
fn a_button_publishes_what_the_pointer_is_doing_to_it() {
    // The route from the simulation to the screen is a node property, because
    // the render crate must not need to know a simulation exists. This is that
    // property being written.
    use dimetric_sim::ui::{STATE_HOVERED, STATE_IDLE, STATE_PRESSED};
    let mut sim = button_sim();

    sim.step(frame(200, 150, false));
    assert_eq!(state_of(&sim, "/Menu/Go"), STATE_IDLE);

    sim.step(frame(50, 20, false));
    assert_eq!(state_of(&sim, "/Menu/Go"), STATE_HOVERED);

    sim.step(frame(50, 20, true));
    assert_eq!(state_of(&sim, "/Menu/Go"), STATE_PRESSED);

    // Sliding off while held goes back to idle rather than staying pressed:
    // the button is no longer the thing under the pointer, and showing it
    // held would promise a click that will not happen.
    sim.step(frame(200, 150, true));
    assert_eq!(state_of(&sim, "/Menu/Go"), STATE_IDLE);
}

#[test]
fn a_buttons_published_state_is_part_of_the_hash() {
    // It is written into the scene, so it is hashed with the scene. Worth
    // asserting because it is the difference between a menu that rolls back
    // correctly and one that does not.
    let mut idle = button_sim();
    let mut hovered = button_sim();
    idle.step(frame(200, 150, false));
    hovered.step(frame(50, 20, false));
    assert_ne!(idle.hash(), hovered.hash());
}
