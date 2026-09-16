//! Measuring text from a script.
//!
//! This is the piece of the UI request worth building regardless of the rest:
//! it is a pure function of a baked font and a string, both of which the
//! engine already has, and it contains no layout policy at all. Without it
//! every piece of text in a game is sized by guessing.

use dimetric_assets::builtin_font::{builtin, BUILTIN_FONT};
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

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
    let out = dimetric_scene::parse(SCENE, "m.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn run(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    let (font, _) = builtin();
    let mut fonts = dimetric_sim::text::Fonts::new();
    fonts.insert(BUILTIN_FONT.to_string(), font);
    host.set_fonts(fonts);
    host.load("scripts/actor.lua", script).expect("loads");
    let mut sim = Sim::new(load(), 3, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let d = sim.take_diagnostics();
    assert!(!d.has_errors(), "{d}");
    sim
}

fn var(sim: &Sim, key: &str) -> Option<i64> {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    state.vars[&uid].get(key).and_then(|v| v.as_int())
}

#[test]
fn a_script_can_measure_a_string() {
    let sim = run(r#"
function on_tick(self)
  local m = ui.measure("builtin", "AB")
  self.w, self.h = m.w, m.h
end
"#);
    // The built-in font is five wide with a one-pixel gap, so two glyphs
    // advance twelve. Arithmetic rather than a recorded figure.
    assert_eq!(var(&sim, "w"), Some(12));
    assert_eq!(var(&sim, "h"), Some(9));
}

#[test]
fn a_longer_string_measures_wider() {
    let sim = run(r#"
function on_tick(self)
  self.short = ui.measure("builtin", "A").w
  self.long = ui.measure("builtin", "AAAAA").w
end
"#);
    assert_eq!(var(&sim, "short"), Some(6));
    assert_eq!(var(&sim, "long"), Some(30));
}

#[test]
fn newlines_stack_and_the_width_is_the_widest_line() {
    // A tooltip is usually more than one line, and sizing it to the last one
    // is the bug this prevents.
    let sim = run(r#"
function on_tick(self)
  local m = ui.measure("builtin", "A\nAAAA\nAA")
  self.w, self.h = m.w, m.h
end
"#);
    assert_eq!(var(&sim, "w"), Some(24), "the widest line is four glyphs");
    assert_eq!(var(&sim, "h"), Some(27), "three lines");
}

#[test]
fn an_empty_string_measures_as_nothing() {
    let sim = run(r#"
function on_tick(self)
  self.w = ui.measure("builtin", "").w
end
"#);
    assert_eq!(var(&sim, "w"), Some(0));
}

#[test]
fn an_unknown_font_measures_as_nothing_rather_than_failing() {
    // A missing font is a project problem, not a reason to stop a tick — the
    // same call a `Label` makes when its font is absent.
    let sim = run(r#"
function on_tick(self)
  local m = ui.measure("fonts/not-here", "AB")
  self.w, self.h = m.w, m.h
end
"#);
    assert_eq!(var(&sim, "w"), Some(0));
    assert_eq!(var(&sim, "h"), Some(0));
}

#[test]
fn measuring_is_the_same_every_time() {
    // It has to be: a control sized to its caption puts the answer into a
    // layout, and layout is hashed.
    let once = run(r#"
function on_tick(self)
  self.w = ui.measure("builtin", "Hamburgefonstiv").w
end
"#);
    let w = var(&once, "w");
    for _ in 0..8 {
        let again = run(r#"
function on_tick(self)
  self.w = ui.measure("builtin", "Hamburgefonstiv").w
end
"#);
        assert_eq!(var(&again, "w"), w);
    }
}
