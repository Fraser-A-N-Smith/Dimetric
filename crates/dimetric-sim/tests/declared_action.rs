//! A script reads a verb the engine has never heard of.
//!
//! `input.pressed("undo")` has to resolve a name that is not one of the
//! engine's five buttons, and resolve it to the *same* bit the runtime set when
//! the key was pressed. The mapping is the action's position in the list the
//! project declared, and `dimetric_sim::input::action_button` is the one place
//! it is made — so the two sides cannot disagree about what bit 5 means.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::{action_button, buttons, InputFrame, PlayerInput};
use dimetric_sim::{LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "World"

[[node]]
id = "n_actor000"
kind = "Node2D"
name = "Actor"
parent = "n_root0000"
script = "script:scripts/actor.lua"
"##;

fn load() -> Scene {
    let out = dimetric_scene::parse(SCENE, "a.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// A simulation whose project declares `actions`.
fn sim_with(actions: &[&str], script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    let problems = host.set_actions(actions.iter().map(|s| s.to_string()).collect());
    assert!(problems.is_empty(), "{problems:?}");
    host.load("scripts/actor.lua", script).expect("loads");
    Sim::new(load(), 7, Box::new(host), SimConfig::default())
}

/// One frame with the given buttons held.
fn frame(bits: u32) -> InputFrame {
    InputFrame {
        players: vec![PlayerInput {
            buttons: bits,
            ..PlayerInput::default()
        }],
    }
}

/// What the script wrote into `self.saw`.
fn saw(sim: &Sim) -> String {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get("saw"))
        .and_then(dimetric_scene::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

const WATCH: &str = r#"
function on_ready(self)
  self.saw = ""
end

function on_tick(self)
  if input.pressed("undo") then self.saw = self.saw .. "U" end
  if input.pressed("screens") then self.saw = self.saw .. "S" end
  if input.pressed("fire") then self.saw = self.saw .. "F" end
end
"#;

#[test]
fn a_script_reads_a_verb_the_project_declared() {
    let mut sim = sim_with(&["undo", "screens"], WATCH);
    let undo = action_button("undo", &["undo".into(), "screens".into()]).expect("a bit");
    let screens = action_button("screens", &["undo".into(), "screens".into()]).expect("a bit");

    sim.step(frame(0));
    sim.step(frame(undo));
    sim.step(frame(0));
    sim.step(frame(screens));
    sim.step(frame(0));
    sim.step(frame(buttons::FIRE));
    assert_eq!(saw(&sim), "USF");
}

#[test]
fn an_undeclared_verb_raises_rather_than_reading_nothing() {
    // A script asking for a name the project did not declare is a bug in the
    // script, and silently answering false would hide it for ever.
    let mut sim = sim_with(
        &["undo"],
        r#"
function on_tick(self)
  self.saw = tostring(input.pressed("undu"))
end
"#,
    );
    sim.step(frame(0));
    let diagnostics = sim.take_diagnostics();
    assert!(
        diagnostics.iter().any(|d| d.message.contains("undu")),
        "{diagnostics}"
    );
    // And the message says what this project does have, not only the engine's.
    assert!(
        diagnostics.iter().any(|d| d.message.contains("undo")),
        "{diagnostics}"
    );
}

#[test]
fn a_declared_verb_is_hashed_like_any_button() {
    // It travels in the same bitfield as `fire`, so a run that undid and one
    // that did not have to hash differently — otherwise an undo could not be
    // recorded at all.
    let hashes = |bits: u32| {
        let mut sim = sim_with(&["undo"], WATCH);
        sim.step(frame(0));
        sim.step(frame(bits));
        sim.hash()
    };
    let undo = action_button("undo", &["undo".into()]).expect("a bit");
    assert_ne!(hashes(0), hashes(undo));
    assert_eq!(
        hashes(undo),
        hashes(undo),
        "and the same input, the same hash"
    );
}

#[test]
fn the_same_input_replays_the_same_whatever_the_names_are() {
    // The dictionary is not in the hash: two projects whose declared lists
    // differ in *name* but agree in shape read the same bits the same way. The
    // scripts differ too in practice, which is why this is safe — what must
    // not happen is the name reaching the state.
    let run = |actions: &[&str], script: &str| {
        let mut sim = sim_with(actions, script);
        let bit = 1 << buttons::CUSTOM_FIRST;
        sim.step(frame(0));
        sim.step(frame(bit));
        sim.hash()
    };
    let one = run(
        &["undo"],
        r#"function on_tick(self) self.n = input.pressed("undo") and 1 or 0 end"#,
    );
    let other = run(
        &["rewind"],
        r#"function on_tick(self) self.n = input.pressed("rewind") and 1 or 0 end"#,
    );
    assert_eq!(one, other);
}
