//! M2's acceptance criteria: a run is reproducible from its seed and input log,
//! snapshots restore exactly, and the phase order is the one that was agreed.

use dimetric_core::{Fx, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{
    input::{buttons, InputFrame, InputLog, PlayerInput},
    LuaHost, NoScripts, Phase, Sim, SimConfig, PHASE_ORDER,
};

fn load(src: &str) -> Scene {
    let out = dimetric_scene::parse(src, "test.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// A mover between two walls, for the sweep tests.
const CORRIDOR: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena"

[[node]]
id = "n_player00"
kind = "Collider"
name = "Player"
parent = "n_root0000"
pos = [0.0, 0.0]
size = [16.0, 16.0]

[[node]]
id = "n_wallrt01"
kind = "Collider"
name = "WallRight"
parent = "n_root0000"
pos = [100.0, 0.0]
size = [16.0, 200.0]
is_static = true

[[node]]
id = "n_walldn01"
kind = "Collider"
name = "WallDown"
parent = "n_root0000"
pos = [0.0, 100.0]
size = [200.0, 16.0]
is_static = true
"##;

fn corridor_sim() -> Sim {
    Sim::new(
        load(CORRIDOR),
        1234,
        Box::new(NoScripts),
        SimConfig::default(),
    )
}

#[test]
fn the_phase_order_is_the_one_that_was_agreed() {
    // Reordering these silently invalidates every recorded replay, so the
    // order is asserted rather than assumed.
    assert_eq!(
        PHASE_ORDER,
        &[
            Phase::Input,
            Phase::ScriptsTick,
            Phase::PhysicsIntegrate,
            Phase::CollisionBroadphase,
            Phase::CollisionResolve,
            Phase::CollisionCallbacks,
            // Animation and tweens advance after collisions have settled: a
            // squash tween that ran before the sweep would be overwritten by
            // it, and a hitbox frame should open against final positions.
            Phase::Advance,
            Phase::ScriptsPostTick,
            Phase::SignalFlush,
            Phase::TickIncrement,
        ]
    );
}

#[test]
fn a_body_stops_at_a_wall_instead_of_passing_through_it() {
    let mut sim = corridor_sim();
    let player = sim.state().scene.resolve_path("/Arena/Player").unwrap();
    let uid = sim.state().scene.get(player).unwrap().uid;
    // 600 units/second for one tick is 10 units; run it far enough to reach
    // the wall at x = 100.
    sim.set_velocity(uid, Vec2Fx::from_ints(600, 0));
    for _ in 0..60 {
        sim.step(InputFrame::idle(1));
    }
    let x = sim.state().scene.get(player).unwrap().transform.pos.x;
    // The wall's near face is at 100 - 8 = 92; the player's half width is 8.
    assert!(
        x > Fx::from_int(80) && x < Fx::from_int(85),
        "stopped at {x}, expected just short of the wall"
    );
}

#[test]
fn a_body_slides_along_a_wall_rather_than_sticking_to_it() {
    let mut sim = corridor_sim();
    let player = sim.state().scene.resolve_path("/Arena/Player").unwrap();
    let uid = sim.state().scene.get(player).unwrap().uid;
    // Push diagonally into the right-hand wall. The x component should be
    // blocked and the y component should survive.
    sim.set_velocity(uid, Vec2Fx::from_ints(600, 300));
    for _ in 0..60 {
        sim.step(InputFrame::idle(1));
    }
    let pos = sim.state().scene.get(player).unwrap().transform.pos;
    assert!(
        pos.x < Fx::from_int(85),
        "x should be blocked, is {}",
        pos.x
    );
    assert!(
        pos.y > Fx::from_int(50),
        "y should keep moving while sliding, is {}",
        pos.y
    );
}

#[test]
fn a_very_fast_body_does_not_tunnel_through_a_thin_wall() {
    let mut sim = corridor_sim();
    let player = sim.state().scene.resolve_path("/Arena/Player").unwrap();
    let uid = sim.state().scene.get(player).unwrap().uid;
    // 12000 units/second is 200 units per tick — far enough to jump the wall
    // entirely in one step if the test were discrete rather than swept.
    sim.set_velocity(uid, Vec2Fx::from_ints(12000, 0));
    for _ in 0..4 {
        sim.step(InputFrame::idle(1));
    }
    let x = sim.state().scene.get(player).unwrap().transform.pos.x;
    assert!(
        x < Fx::from_int(95),
        "body tunnelled through the wall, ended at {x}"
    );
}

#[test]
fn the_same_seed_and_inputs_produce_the_same_state_hash() {
    let run = |seed: u64| {
        let mut sim = Sim::new(
            load(CORRIDOR),
            seed,
            Box::new(NoScripts),
            SimConfig::default(),
        );
        let player = sim.state().scene.resolve_path("/Arena/Player").unwrap();
        let uid = sim.state().scene.get(player).unwrap().uid;
        sim.set_velocity(uid, Vec2Fx::from_ints(240, 180));
        let mut hashes = Vec::new();
        for tick in 0..120u64 {
            let mut frame = InputFrame::idle(1);
            frame.players[0] = PlayerInput {
                buttons: if tick % 7 == 0 { buttons::FIRE } else { 0 },
                move_dir: Vec2Fx::new(Fx::HALF, Fx::ZERO),
                aim: dimetric_core::Angle::from_bam((tick * 512) as u16),
            };
            sim.step(frame);
            hashes.push(sim.hash());
        }
        hashes
    };
    assert_eq!(run(99), run(99), "identical runs must hash identically");
    assert_ne!(
        run(99).last(),
        run(100).last(),
        "a different seed should reach a different state"
    );
}

#[test]
fn a_snapshot_restores_the_run_exactly() {
    let mut sim = corridor_sim();
    let player = sim.state().scene.resolve_path("/Arena/Player").unwrap();
    let uid = sim.state().scene.get(player).unwrap().uid;
    sim.set_velocity(uid, Vec2Fx::from_ints(300, 150));

    for _ in 0..20 {
        sim.step(InputFrame::idle(1));
    }
    let snapshot = sim.snapshot();
    let expected: Vec<_> = (0..30)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();

    // Rewind and replay: every tick must land on the same hash.
    sim.restore(snapshot);
    let actual: Vec<_> = (0..30)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();
    assert_eq!(expected, actual);
}

#[test]
fn the_state_hash_notices_a_single_changed_bit() {
    let mut a = corridor_sim();
    let mut b = corridor_sim();
    let player = a.state().scene.resolve_path("/Arena/Player").unwrap();
    let uid = a.state().scene.get(player).unwrap().uid;
    a.set_velocity(uid, Vec2Fx::from_ints(100, 0));
    b.set_velocity(uid, Vec2Fx::from_raw(100 * 65536 + 1, 0));
    a.step(InputFrame::idle(1));
    b.step(InputFrame::idle(1));
    assert_ne!(
        a.hash(),
        b.hash(),
        "one raw unit of difference must show up"
    );
}

#[test]
fn input_logs_round_trip_through_text() {
    let mut log = InputLog::new(4242, "0.0.1", 2);
    for tick in 0..10u64 {
        log.push(InputFrame {
            players: vec![
                PlayerInput {
                    buttons: (tick as u32) & 0xf,
                    move_dir: Vec2Fx::new(Fx::HALF, -Fx::HALF),
                    aim: dimetric_core::Angle::from_degrees_str("45.0").unwrap(),
                },
                PlayerInput::default(),
            ],
        });
    }
    let text = log.to_text();
    let back = InputLog::parse(&text).expect("round trip");
    assert_eq!(back, log);
    assert_eq!(back.to_text(), text);
}

#[test]
fn a_log_with_a_missing_tick_line_is_refused() {
    let broken = "dimetric-input 1\nseed 1\nplayers 1\n0 0000 0.0 0.0 0.0\n2 0000 0.0 0.0 0.0\n";
    // Accepting this would shift every input by one tick and produce a replay
    // that diverges for no visible reason.
    assert!(matches!(
        InputLog::parse(broken),
        Err(dimetric_sim::input::LogError::OutOfOrder { .. })
    ));
}

// -- scripting ----------------------------------------------------------

const SCRIPTED: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena"

[[node]]
id = "n_walker01"
kind = "Node2D"
name = "Walker"
parent = "n_root0000"
script = "script:scripts/walker.lua"
pos = [0.0, 0.0]

[[node]]
id = "n_spawnr01"
kind = "Node"
name = "Spawner"
parent = "n_root0000"
script = "script:scripts/spawner.lua"

[[connect]]
from = "n_walker01"
signal = "stepped"
to = "n_spawnr01"
method = "on_stepped"
"##;

const WALKER_LUA: &str = r#"
function on_ready(self)
  self.steps = 0
  self.speed = fx.new(2)
end

function on_tick(self)
  self.pos = self.pos + vec2(self.speed, fx.new(0))
  self.steps = self.steps + 1
  if self.steps == 5 then
    self:emit("stepped", { count = self.steps })
  end
end
"#;

const SPAWNER_LUA: &str = r#"
function on_ready(self)
  self.heard = 0
end

function on_stepped(self, from, name, payload)
  self.heard = self.heard + 1
  self.last_count = payload.count
  self.roll = rng.range("spawn", 1, 100)
end
"#;

fn scripted_sim(seed: u64) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/walker.lua", WALKER_LUA)
        .expect("walker loads");
    host.load("scripts/spawner.lua", SPAWNER_LUA)
        .expect("spawner loads");
    Sim::new(load(SCRIPTED), seed, Box::new(host), SimConfig::default())
}

#[test]
fn a_lua_driven_node_moves_and_keeps_its_state_in_rust() {
    let mut sim = scripted_sim(7);
    for _ in 0..10 {
        sim.step(InputFrame::idle(1));
    }
    assert!(sim.diagnostics().is_empty(), "{}", sim.diagnostics());

    let walker = sim.state().scene.resolve_path("/Arena/Walker").unwrap();
    let state = sim.state();
    let uid = state.scene.get(walker).unwrap().uid;
    assert_eq!(
        state.scene.get(walker).unwrap().transform.pos.x,
        Fx::from_int(20),
        "ten ticks at two units each"
    );
    // `self.steps` lives in the simulation, not in a Lua global, which is what
    // makes it survive a snapshot.
    assert_eq!(
        state
            .var(uid, "steps")
            .and_then(dimetric_scene::Value::as_int),
        Some(10)
    );
}

#[test]
fn a_signal_reaches_its_connection() {
    let mut sim = scripted_sim(7);
    for _ in 0..8 {
        sim.step(InputFrame::idle(1));
    }
    assert!(sim.diagnostics().is_empty(), "{}", sim.diagnostics());
    let spawner = sim.state().scene.resolve_path("/Arena/Spawner").unwrap();
    let state = sim.state();
    let uid = state.scene.get(spawner).unwrap().uid;
    assert_eq!(
        state
            .var(uid, "heard")
            .and_then(dimetric_scene::Value::as_int),
        Some(1),
        "the stepped signal should have arrived exactly once"
    );
    assert_eq!(
        state
            .var(uid, "last_count")
            .and_then(dimetric_scene::Value::as_int),
        Some(5),
        "the payload should come through"
    );
}

#[test]
fn a_scripted_run_replays_identically() {
    let run = || {
        let mut sim = scripted_sim(31);
        let mut hashes = Vec::new();
        for _ in 0..30 {
            sim.step(InputFrame::idle(1));
            hashes.push(sim.hash());
        }
        assert!(sim.diagnostics().is_empty(), "{}", sim.diagnostics());
        hashes
    };
    assert_eq!(run(), run());
}

#[test]
fn a_scripted_run_survives_a_rollback() {
    let mut sim = scripted_sim(31);
    for _ in 0..10 {
        sim.step(InputFrame::idle(1));
    }
    let snapshot = sim.snapshot();
    let expected: Vec<_> = (0..10)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();
    sim.restore(snapshot);
    let actual: Vec<_> = (0..10)
        .map(|_| {
            sim.step(InputFrame::idle(1));
            sim.hash()
        })
        .collect();
    assert_eq!(
        expected, actual,
        "Lua state must not leak outside the snapshot"
    );
}

#[test]
fn the_sandbox_withholds_the_things_that_break_determinism() {
    for (name, source) in [
        ("os", "function on_tick(self) local t = os.time() end"),
        ("io", "function on_tick(self) io.open('x') end"),
        ("require", "function on_tick(self) require('x') end"),
        (
            "math.random",
            "function on_tick(self) local x = math.random() end",
        ),
        (
            "math.sin",
            "function on_tick(self) local x = math.sin(1) end",
        ),
    ] {
        let mut host = LuaHost::new(60).unwrap();
        host.load("scripts/walker.lua", source).unwrap();
        host.load("scripts/spawner.lua", "").unwrap();
        let mut sim = Sim::new(load(SCRIPTED), 1, Box::new(host), SimConfig::default());
        sim.step(InputFrame::idle(1));
        assert!(
            !sim.diagnostics().is_empty(),
            "{name} should not be reachable from a script"
        );
    }
}

#[test]
fn a_script_error_is_reported_with_a_code_and_does_not_stop_the_tick() {
    let mut host = LuaHost::new(60).unwrap();
    host.load(
        "scripts/walker.lua",
        "function on_tick(self) error('boom') end",
    )
    .unwrap();
    host.load("scripts/spawner.lua", "").unwrap();
    let mut sim = Sim::new(load(SCRIPTED), 1, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let diags = sim.take_diagnostics();
    assert_eq!(diags.len(), 1);
    let d = diags.iter().next().unwrap();
    assert_eq!(d.code, dimetric_core::Code::SCRIPT_RUNTIME);
    assert!(d.message.contains("boom"), "{d}");
    // The tick still completed.
    assert_eq!(sim.state().tick.0, 1);
}

#[test]
fn a_syntax_error_is_reported_when_the_script_is_loaded() {
    let mut host = LuaHost::new(60).unwrap();
    let err = host
        .load(
            "scripts/bad.lua",
            "function on_tick(self) this is not lua end",
        )
        .unwrap_err();
    assert_eq!(err.code, dimetric_core::Code::SCRIPT_SYNTAX);
}

#[test]
fn a_destroyed_node_leaves_no_state_behind() {
    let mut host = LuaHost::new(60).unwrap();
    host.load(
        "scripts/walker.lua",
        "function on_tick(self) if tick.count() == 3 then self:destroy() end end",
    )
    .unwrap();
    host.load("scripts/spawner.lua", "").unwrap();
    let mut sim = Sim::new(load(SCRIPTED), 1, Box::new(host), SimConfig::default());
    let walker = sim.state().scene.resolve_path("/Arena/Walker").unwrap();
    let uid = sim.state().scene.get(walker).unwrap().uid;
    for _ in 0..6 {
        sim.step(InputFrame::idle(1));
    }
    let state = sim.state();
    assert!(state.scene.by_uid(uid).is_none(), "the node should be gone");
    assert!(
        !state.vars.contains_key(&uid),
        "its variables should be gone too"
    );
    assert!(!state.velocity.contains_key(&uid));
}
