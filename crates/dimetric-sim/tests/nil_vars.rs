//! `self.thing = nil` clears a script variable.
//!
//! `SimState.vars` holds a `Value`, and `Value` has no nil variant, so the
//! write used to store the nearest thing it had: `false`. The read-back was
//! then `false`, the standard `if self.thing ~= nil` guard passed, and the
//! next line indexed a boolean.
//!
//! It cost a real bug: a hub with no finished run behind it set its summary to
//! nil, the guard passed, and the draw indexed a boolean. The engine reported
//! that correctly and the per-tick log swallowed it, so the only symptom was a
//! caption that would not hide.
//!
//! Substituting a different value that then fails the standard nil check is
//! the one behaviour that surprises. Nil removes the key, which is what it
//! does to every other table in Lua and needs no new variant in a value type
//! the scene format also uses.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

fn scene() -> Scene {
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
         script = \"script:main.lua\"\n";
    let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn step(source: &str) -> (Sim, String) {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "script did not load: {load:?}");
    let mut sim = Sim::new(scene(), 1, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let complaints = sim
        .take_diagnostics()
        .0
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    (sim, complaints)
}

fn var(sim: &Sim, key: &str) -> Option<dimetric_scene::Value> {
    let state = sim.state();
    let id = state.scene.resolve_path("/World").expect("world");
    let uid = state.scene.get(id).expect("world").uid;
    state.var(uid, key).cloned()
}

#[test]
fn a_variable_set_to_nil_reads_back_as_nil() {
    // The defect, in one line of Lua. `saw` records what the guard saw.
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 self.thing = 7\n\
         \x20 self.thing = nil\n\
         \x20 self.saw = (self.thing == nil) and \"nil\" or tostring(self.thing)\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(
        var(&sim, "saw").and_then(|v| v.as_str().map(str::to_string)),
        Some("nil".to_string()),
        "the standard nil check did not see a nil"
    );
    assert_eq!(var(&sim, "thing"), None, "the variable is gone, not false");
}

#[test]
fn clearing_the_last_variable_leaves_the_state_as_it_started() {
    // `vars.len()` is hashed, so a node left holding an empty table would make
    // "cleared the only variable" hash differently from "never had one" —
    // two states a script cannot tell apart.
    let (cleared, _) = step(
        "function on_tick(self)\n\
         \x20 self.thing = 7\n\
         \x20 self.thing = nil\n\
         end\n",
    );
    let (never, _) = step("function on_tick(self)\nend\n");
    assert_eq!(
        cleared.hash(),
        never.hash(),
        "an emptied table is not the same as no table"
    );
}

#[test]
fn clearing_one_variable_leaves_the_others() {
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 self.a = 1\n\
         \x20 self.b = 2\n\
         \x20 self.a = nil\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(var(&sim, "a"), None);
    assert_eq!(var(&sim, "b").and_then(|v| v.as_int()), Some(2));
}

#[test]
fn clearing_something_that_was_never_there_is_not_an_error() {
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 self.never = nil\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(var(&sim, "never"), None);
}

#[test]
fn nil_anywhere_else_is_refused_rather_than_stored_as_false() {
    // A kind property has a type and a default and cannot be cleared, so a
    // nil there is an argument that was not supplied. It used to become
    // `false`, which for `modulate` meant a node that stopped drawing.
    let (_, out) = step(
        "function on_tick(self)\n\
         \x20 self:set(\"anything\", nil)\n\
         end\n",
    );
    assert!(out.contains("nil is not a value"), "{out}");
    assert!(
        out.contains("self.key = nil"),
        "it names the one that is: {out}"
    );
}
