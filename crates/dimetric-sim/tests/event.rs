//! What the simulation tells the host.
//!
//! The property every test here defends is that saying something to the host
//! cannot change what the simulation does. If emitting an achievement moved
//! the hash, a build with Steam disabled would diverge from one with it on —
//! which is the sound list's argument, word for word.

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
    let out = dimetric_scene::parse(SCENE, "e.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/actor.lua", script).expect("loads");
    Sim::new(load(), 5, Box::new(host), SimConfig::default())
}

#[test]
fn a_script_can_tell_the_host_something() {
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 0 then
    event.emit("achievement", { id = "first_light" })
  end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let events = sim.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "achievement");
    assert_eq!(events[0].tick.0, 0);
    match &events[0].payload {
        dimetric_scene::Value::Map(m) => {
            assert_eq!(m.get("id").and_then(|v| v.as_str()), Some("first_light"));
        }
        other => panic!("expected a map payload, got {other:?}"),
    }
}

#[test]
fn emitting_does_not_move_the_state_hash() {
    // The guarantee. A build with achievements wired up and one without have
    // to produce identical hashes from the same seed.
    let quiet = {
        let mut sim = sim_with("function on_tick(self) end");
        for _ in 0..4 {
            sim.step(InputFrame::idle(1));
        }
        sim.hash()
    };
    let chatty = {
        let mut sim = sim_with(
            r#"
function on_tick(self)
  event.emit("achievement", { id = "tick_" .. tick.count() })
  event.emit("presence", { where = "crypt" })
end
"#,
        );
        for _ in 0..4 {
            sim.step(InputFrame::idle(1));
        }
        sim.hash()
    };
    assert_eq!(quiet, chatty);
}

#[test]
fn draining_is_not_hashed_either() {
    // Whether the host is listening must not change the game. Two identical
    // runs, one drained every tick and one never drained.
    let mut drained = sim_with(r#"function on_tick(self) event.emit("x", {}) end"#);
    let mut hoarded = sim_with(r#"function on_tick(self) event.emit("x", {}) end"#);
    for _ in 0..5 {
        drained.step(InputFrame::idle(1));
        let _ = drained.take_events();
        hoarded.step(InputFrame::idle(1));
    }
    assert_eq!(drained.hash(), hoarded.hash());
}

#[test]
fn a_rollback_re_emits_rather_than_restoring() {
    // The contract, decided rather than discovered. Events are not
    // snapshotted, so a restore does not put drained ones back, and re-running
    // a tick emits again. A host that needs exactly-once deduplicates.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 1 then event.emit("achievement", { id = "deep" }) end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let before = sim.snapshot();
    assert!(sim.take_events().is_empty());

    sim.step(InputFrame::idle(1));
    assert_eq!(sim.take_events().len(), 1, "emitted the first time");

    // Rewind and run the same tick again.
    sim.restore(before);
    assert!(
        sim.take_events().is_empty(),
        "a restore must not resurrect events the host already acted on"
    );
    sim.step(InputFrame::idle(1));
    assert_eq!(
        sim.take_events().len(),
        1,
        "and emitted again on the re-run"
    );
}

#[test]
fn taking_events_clears_them() {
    let mut sim = sim_with(r#"function on_tick(self) event.emit("x", {}) end"#);
    sim.step(InputFrame::idle(1));
    assert_eq!(sim.take_events().len(), 1);
    assert!(sim.take_events().is_empty(), "a drain is a take");
}

#[test]
fn events_arrive_in_the_order_they_were_emitted() {
    // A settings menu emitting volume then key bindings needs them applied in
    // that order, and ordering is free here only because the list is a Vec.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() ~= 0 then return end
  event.emit("a", {})
  event.emit("b", {})
  event.emit("c", {})
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let kinds: Vec<String> = sim.take_events().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec!["a", "b", "c"]);
}

#[test]
fn the_tick_is_carried_so_a_late_drain_can_still_tell_them_apart() {
    let mut sim = sim_with(r#"function on_tick(self) event.emit("x", {}) end"#);
    for _ in 0..3 {
        sim.step(InputFrame::idle(1));
    }
    let ticks: Vec<u64> = sim.take_events().into_iter().map(|e| e.tick.0).collect();
    assert_eq!(ticks, vec![0, 1, 2]);
}

#[test]
fn a_payload_is_optional() {
    let mut sim = sim_with(r#"function on_tick(self) event.emit("ping") end"#);
    sim.step(InputFrame::idle(1));
    let events = sim.take_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload,
        dimetric_scene::Value::Map(ref m) if m.is_empty()
    ));
}

#[test]
fn an_unnamed_event_is_refused() {
    // A host routes on the kind; an empty one goes nowhere and would look like
    // the channel is broken.
    let mut sim = sim_with(r#"function on_tick(self) event.emit("") end"#);
    sim.step(InputFrame::idle(1));
    let d = sim.take_diagnostics();
    assert!(d.has_errors());
    assert!(d.to_string().contains("DIM0505"), "{d}");
}

#[test]
fn a_runaway_loop_is_reported_rather_than_filling_memory() {
    // A script emitting per entity rather than per happening is a plausible
    // mistake, and an unbounded list fills memory quietly.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  for i = 1, 100000 do event.emit("spam", {}) end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let d = sim.take_diagnostics();
    assert!(d.has_errors());
    assert!(d.to_string().contains("DIM0505"), "{d}");
    assert!(sim.take_events().len() <= dimetric_sim::event::MAX_EVENTS_PER_TICK);
}

#[test]
fn a_list_payload_keeps_its_order() {
    // The `Value` type scripts already store, so no second serialisation and
    // ordered lists stay ordered (I4).
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 0 then
    event.emit("loadout", { slots = { "amber", "jet", "opal" } })
  end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let events = sim.take_events();
    let dimetric_scene::Value::Map(m) = &events[0].payload else {
        panic!("expected a map");
    };
    let Some(dimetric_scene::Value::List(slots)) = m.get("slots") else {
        panic!("expected a list, got {:?}", m.get("slots"));
    };
    let names: Vec<&str> = slots.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(names, vec!["amber", "jet", "opal"]);
}
