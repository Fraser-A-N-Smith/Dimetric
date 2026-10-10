//! A script can start a stream over, and a replay replays it.
//!
//! Run codes: six letters that name a run, so a player can share one or play
//! one again. Every draw a run makes is on a named stream, and a run with a
//! code draws from `floor#KQPRMX` instead of `floor`, so the same code is the
//! same dice. A stream is created from the run seed the first time it is asked
//! for and kept for the rest of the session, though — so the first time a code
//! is played in a launch it is exactly its run, and the second time it
//! continues where the first left off and is a different run while the screen
//! says they are the same.
//!
//! A reset is a change to simulation state and is hashed like a draw, which is
//! what makes it replayable rather than a back door around the log.

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
    let out = dimetric_scene::parse(SCENE, "r.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/actor.lua", script).expect("loads");
    Sim::new(load(), 7, Box::new(host), SimConfig::default())
}

/// A run that draws three numbers from `floor#KQPRMX` and writes them into its
/// own state, optionally resetting the stream before each run begins.
///
/// Two runs in one session, which is the situation: play a code, come home,
/// play the same code again.
fn two_runs(reset: &str) -> Vec<i32> {
    let mut sim = sim_with(&format!(
        r#"
function on_ready(self)
  self.rolls = ""
end

function on_tick(self)
  local t = tick.count()
  -- A run begins at tick 0 and again at tick 4.
  if t == 0 or t == 4 then
    {reset}
  end
  if t < 3 or (t >= 4 and t < 7) then
    self.rolls = self.rolls .. tostring(rng.range("floor#KQPRMX", 0, 1000)) .. ","
  end
end
"#
    ));
    for _ in 0..8 {
        sim.step(InputFrame::idle(1));
    }
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    let rolls = state
        .vars
        .get(&uid)
        .and_then(|vars| vars.get("rolls"))
        .and_then(dimetric_scene::Value::as_str)
        .expect("the script wrote its rolls")
        .to_string();
    rolls
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("a number"))
        .collect()
}

#[test]
fn without_a_reset_the_second_play_of_a_code_is_a_different_run() {
    // The defect, through Lua, which is the layer the game sees it from.
    let rolls = two_runs("-- nothing");
    assert_eq!(rolls.len(), 6);
    assert_ne!(
        rolls[..3],
        rolls[3..],
        "the second play has to differ, or there is no bug to fix"
    );
}

#[test]
fn a_reset_makes_the_second_play_the_same_run() {
    let rolls = two_runs(r#"rng.reset("floor#KQPRMX")"#);
    assert_eq!(rolls.len(), 6);
    assert_eq!(
        rolls[..3],
        rolls[3..],
        "the same code has to be the same dice"
    );
}

#[test]
fn an_explicit_seed_does_the_same_and_does_not_depend_on_the_session() {
    // What makes a code portable between launches: `reset` reconstructs from
    // the session seed, and this does not depend on it at all.
    let rolls = two_runs(r#"rng.seed("floor#KQPRMX", 12345)"#);
    assert_eq!(rolls[..3], rolls[3..]);

    // A run seeded by hand is the same run under a different session seed,
    // which is the claim a shared run code actually rests on.
    let seeded = |session: u64| {
        let mut host = LuaHost::new(60).expect("lua host");
        host.load(
            "scripts/actor.lua",
            r#"
function on_ready(self)
  rng.seed("floor", 12345)
  self.roll = rng.range("floor", 0, 1000000)
end
"#,
        )
        .expect("loads");
        let mut sim = Sim::new(load(), session, Box::new(host), SimConfig::default());
        sim.step(InputFrame::idle(1));
        let state = sim.state();
        let id = state.scene.resolve_path("/World/Actor").expect("actor");
        let uid = state.scene.get(id).expect("actor").uid;
        state
            .vars
            .get(&uid)
            .and_then(|vars| vars.get("roll"))
            .and_then(dimetric_scene::Value::as_int)
            .expect("a roll")
    };
    assert_eq!(seeded(7), seeded(999_999));
}

#[test]
fn a_reset_moves_the_hash_because_it_is_a_change_to_state() {
    // Not a back door around the log. If a reset were invisible to the hash, a
    // replay could diverge from the run it recorded and nothing would notice.
    let hashes = |script: &str| {
        let mut sim = sim_with(script);
        (0..4)
            .map(|_| {
                sim.step(InputFrame::idle(1));
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    let plain = hashes(
        r#"
function on_tick(self)
  self.roll = rng.range("floor", 0, 1000)
end
"#,
    );
    let with_reset = hashes(
        r#"
function on_tick(self)
  if tick.count() == 2 then rng.reset("floor") end
  self.roll = rng.range("floor", 0, 1000)
end
"#,
    );
    assert_eq!(plain[..2], with_reset[..2], "nothing differs before it");
    assert_ne!(plain[2], with_reset[2], "and the reset is in the hash");
}

#[test]
fn a_run_that_resets_replays_to_the_same_hashes() {
    // The property that matters: a reset is a draw like any other, so the same
    // seed and the same input produce the same state.
    let script = r#"
function on_tick(self)
  local t = tick.count()
  if t == 0 or t == 4 then rng.reset("floor#KQPRMX") end
  self.roll = rng.range("floor#KQPRMX", 0, 1000)
end
"#;
    let run = || {
        let mut sim = sim_with(script);
        (0..8)
            .map(|_| {
                sim.step(InputFrame::idle(1));
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        run(),
        run(),
        "two runs of one seed must agree tick for tick"
    );
}

#[test]
fn a_reset_survives_a_snapshot_and_restore() {
    // A rollback must not undo or re-apply one. The stream's whole state is in
    // the snapshot, so a reset is restored like anything else.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 1 then rng.reset("floor") end
  self.roll = rng.range("floor", 0, 1000)
end
"#,
    );
    sim.step(InputFrame::idle(1));
    sim.step(InputFrame::idle(1));
    let after_reset = sim.snapshot();
    let expected: Vec<_> = (0..3)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();

    sim.restore(after_reset);
    let again: Vec<_> = (0..3)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();
    assert_eq!(expected, again);
}

#[test]
fn resetting_one_stream_leaves_the_rest_of_the_run_alone() {
    // The whole point of naming streams. A run resets its seven and the
    // session's other draws do not shift.
    let mut sim = sim_with(
        r#"
function on_ready(self)
  self.damage = ""
end

function on_tick(self)
  if tick.count() == 2 then rng.reset("floor") end
  self.damage = self.damage .. tostring(rng.range("damage", 0, 1000)) .. ","
  self.floor = rng.range("floor", 0, 1000)
end
"#,
    );
    for _ in 0..5 {
        sim.step(InputFrame::idle(1));
    }
    let damaged = {
        let state = sim.state();
        let id = state.scene.resolve_path("/World/Actor").expect("actor");
        let uid = state.scene.get(id).expect("actor").uid;
        state
            .vars
            .get(&uid)
            .and_then(|v| v.get("damage"))
            .and_then(dimetric_scene::Value::as_str)
            .expect("damage rolls")
            .to_string()
    };

    // The same run with no reset at all: `damage` has to read identically.
    let mut control = sim_with(
        r#"
function on_ready(self)
  self.damage = ""
end

function on_tick(self)
  self.damage = self.damage .. tostring(rng.range("damage", 0, 1000)) .. ","
  self.floor = rng.range("floor", 0, 1000)
end
"#,
    );
    for _ in 0..5 {
        control.step(InputFrame::idle(1));
    }
    let control_damage = {
        let state = control.state();
        let id = state.scene.resolve_path("/World/Actor").expect("actor");
        let uid = state.scene.get(id).expect("actor").uid;
        state
            .vars
            .get(&uid)
            .and_then(|v| v.get("damage"))
            .and_then(dimetric_scene::Value::as_str)
            .expect("damage rolls")
            .to_string()
    };
    assert_eq!(damaged, control_damage, "resetting `floor` moved `damage`");
}
