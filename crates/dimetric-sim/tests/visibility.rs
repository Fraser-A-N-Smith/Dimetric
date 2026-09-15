//! A script that hides a collider mid-tick.
//!
//! An invisible node is not a body, so hiding one has to reach the sweep in
//! the same tick rather than the next. The scripts and the sweep read the
//! scene at two different points in a tick, and anything that caches the
//! first for the second gets this wrong by exactly one tick — which is
//! invisible in a stress run and obvious in a game.

use dimetric_core::Vec2Fx;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

const ROOM: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"
script = "script:scripts/room.lua"

[[node]]
id = "n_mover000"
kind = "Collider"
name = "Mover"
parent = "n_root0000"
pos = [0.0, 0.0]
shape = "Circle"
radius = 4.0
collision_layer = 1
collision_mask = 1

[[node]]
id = "n_wall0000"
kind = "Collider"
name = "Wall"
parent = "n_root0000"
pos = [20.0, 0.0]
shape = "AABB"
size = [8.0, 40.0]
is_static = true
collision_layer = 1
collision_mask = 1
"##;

fn scene_of(text: &str) -> Scene {
    let out = dimetric_scene::parse(text, "test.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// Run `script` on the room's root for `ticks` ticks and report where the
/// mover ended up.
fn mover_x_after(script: &str, ticks: u64) -> Vec2Fx {
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/room.lua", script).expect("script loads");
    let mut sim = Sim::new(scene_of(ROOM), 1, Box::new(host), SimConfig::default());
    let log = InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
    let state = sim.state();
    let mover = state
        .scene
        .walk()
        .into_iter()
        .find(|id| state.scene.get(*id).is_some_and(|n| n.name == "Mover"))
        .expect("the mover exists");
    state.scene.get(mover).unwrap().transform.pos
}

/// Drive the mover at the wall: ten units a tick, from x=0 at a wall whose
/// near face is at x=16. It is stopped on the second tick, or would be.
const DRIVE: &str = r#"
function on_ready(self)
  self:find("Mover"):set_velocity(vec2(600, 0))
end
"#;

#[test]
fn a_wall_stops_the_mover() {
    let pos = mover_x_after(DRIVE, 2);
    assert!(
        pos.x < dimetric_core::Fx::from_int(13),
        "stopped against the wall, found {}",
        pos.x
    );
}

#[test]
fn hiding_the_wall_reaches_the_same_tick() {
    // Hidden on the tick it would have blocked on. The scripts queried a world
    // built before they ran, and the sweep reuses it; a stale one still has the
    // wall in it and stops the mover a tick after it stopped existing.
    let script = format!(
        "{DRIVE}\nlocal n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n == 2 then self:find(\"Wall\").visible = false end\nend\n"
    );
    let pos = mover_x_after(&script, 2);
    assert!(
        pos.x >= dimetric_core::Fx::from_int(19),
        "passed through the hidden wall, found {}",
        pos.x
    );
}
