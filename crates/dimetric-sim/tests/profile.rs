//! The profile layer, and the guarantee that it is nowhere near the hash.
//!
//! Knowledge, awards and unlocks differ between two players playing the same
//! seed. If they were hashed, those two players' replays would diverge for a
//! reason that has nothing to do with the game.

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
    let out = dimetric_scene::parse(SCENE, "p.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(
    script: &str,
) -> (
    Sim,
    std::rc::Rc<std::cell::RefCell<dimetric_sim::profile::Profile>>,
) {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/actor.lua", script).expect("loads");
    let handle = host.profile_handle();
    (
        Sim::new(load(), 11, Box::new(host), SimConfig::default()),
        handle,
    )
}

#[test]
fn a_script_can_write_and_read_the_profile() {
    let (mut sim, profile) = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 0 then
    profile.put("runs", (profile.get("runs") or 0) + 1)
    profile.put("seen", { "crypt", "vault" })
  end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let p = profile.borrow();
    assert_eq!(p.get("runs").and_then(|v| v.as_int()), Some(1));
    assert!(matches!(
        p.get("seen"),
        Some(dimetric_scene::Value::List(l)) if l.len() == 2
    ));
}

#[test]
fn writing_the_profile_does_not_move_the_state_hash() {
    // The guarantee. Two players with different unlocks must produce the same
    // hashes from the same seed, or replay is not replay.
    let quiet = {
        let (mut sim, _) = sim_with("function on_tick(self) end");
        for _ in 0..4 {
            sim.step(InputFrame::idle(1));
        }
        sim.hash()
    };
    let noisy = {
        let (mut sim, _) = sim_with(
            r#"
function on_tick(self)
  profile.put("awards", tick.count())
  profile.put("name", "somebody")
end
"#,
        );
        for _ in 0..4 {
            sim.step(InputFrame::idle(1));
        }
        sim.hash()
    };
    assert_eq!(quiet, noisy);
}

#[test]
fn a_prepopulated_profile_does_not_move_the_state_hash_either() {
    // The direction that matters more: a player who has unlocked things must
    // hash the same as one who has not, or every replay depends on who is
    // playing.
    let run = |entries: Vec<(&str, i64)>| {
        let mut host = LuaHost::new(60).expect("lua host");
        host.load("scripts/actor.lua", "function on_tick(self) end")
            .expect("loads");
        {
            let handle = host.profile_handle();
            let mut p = handle.borrow_mut();
            for (k, v) in entries {
                p.put(k, dimetric_scene::Value::Int(v));
            }
        }
        let mut sim = Sim::new(load(), 11, Box::new(host), SimConfig::default());
        for _ in 0..3 {
            sim.step(InputFrame::idle(1));
        }
        sim.hash()
    };
    assert_eq!(run(vec![]), run(vec![("knows_fire", 1), ("awards", 7)]));
}

#[test]
fn the_profile_is_not_rewound_by_a_rollback() {
    // It is not simulation state, so a snapshot does not capture it and a
    // restore does not put it back. Asserted because the opposite would mean
    // an award vanishing when a rollback happened to cross the tick that
    // granted it.
    let (mut sim, profile) = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 1 then profile.put("award", "deep") end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let before = sim.snapshot();

    sim.step(InputFrame::idle(1));
    assert!(profile.borrow().get("award").is_some());

    sim.restore(before);
    assert!(
        profile.borrow().get("award").is_some(),
        "a rollback must not take back what a player earned"
    );
}

#[test]
fn a_profile_tracks_whether_it_needs_writing_out() {
    // So a host writes the file on a change rather than sixty times a second,
    // which is how a profile gets caught half-written by a power cut.
    let (mut sim, profile) = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 2 then profile.put("runs", 1) end
end
"#,
    );
    assert!(!profile.borrow().is_dirty());
    sim.step(InputFrame::idle(1));
    assert!(!profile.borrow().is_dirty(), "nothing written yet");

    for _ in 0..2 {
        sim.step(InputFrame::idle(1));
    }
    assert!(profile.borrow().is_dirty());
    profile.borrow_mut().mark_clean();
    assert!(!profile.borrow().is_dirty());
}

#[test]
fn clearing_a_key_removes_it() {
    let (mut sim, profile) = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 0 then profile.put("temp", 1) end
  if tick.count() == 1 then profile.clear("temp") end
end
"#,
    );
    sim.step(InputFrame::idle(1));
    assert!(profile.borrow().get("temp").is_some());
    sim.step(InputFrame::idle(1));
    assert!(profile.borrow().get("temp").is_none());
}

#[test]
fn a_missing_key_reads_as_nil() {
    let (mut sim, _) = sim_with(
        r#"
function on_tick(self)
  self.had = profile.get("never_set") == nil
end
"#,
    );
    sim.step(InputFrame::idle(1));
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    assert_eq!(
        state.vars[&uid].get("had").and_then(|v| v.as_bool()),
        Some(true)
    );
}
