//! Creating nodes from a script, sensors that move, and the queries that make
//! a busy scene affordable.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene, Value};
use dimetric_sim::{InputFrame, InputLog, LuaHost, Sim, SimConfig};

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
id = "n_marker00"
kind = "Collider"
name = "Marker"
parent = "n_root0000"
pos = [40.0, 0.0]
tags = ["enemy"]
shape = "Circle"
radius = 6.0
collision_layer = 2
collision_mask = 3
"##;

const BULLET: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_bullet00"

[[node]]
id = "n_bullet00"
kind = "Area"
name = "Bullet"
tags = ["bullet"]
shape = "Circle"
radius = 4.0
collision_layer = 4
collision_mask = 2

[[node]]
id = "n_bultrail"
kind = "Sprite2D"
name = "Trail"
parent = "n_bullet00"
texture = "asset:sprites/bolt"
"##;

fn registry() -> KindRegistry {
    KindRegistry::with_builtins()
}

fn scene_of(text: &str) -> Scene {
    let out = dimetric_scene::parse(text, "test.dim", &registry());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// A simulation over `ROOM`, with `BULLET` available to spawn.
fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/room.lua", script).expect("script loads");
    let mut templates = dimetric_sim::spawn::Templates::new();
    templates.insert("prefabs/bullet".to_string(), scene_of(BULLET));
    Sim::new(scene_of(ROOM), 1, Box::new(host), SimConfig::default()).with_templates(templates)
}

fn run(sim: &mut Sim, ticks: u64) {
    let log = InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
}

fn count_named(sim: &Sim, tag: &str) -> usize {
    let state = sim.state();
    state
        .scene
        .walk()
        .into_iter()
        .filter_map(|id| state.scene.get(id))
        .filter(|n| n.has_tag(tag))
        .count()
}

// -- spawning -----------------------------------------------------------

#[test]
fn a_script_can_create_a_node() {
    let mut sim = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/bullet\", vec2(1, 2)) end\nend\n",
    );
    run(&mut sim, 1);
    assert_eq!(count_named(&sim, "bullet"), 1);
}

#[test]
fn a_spawn_appears_at_the_end_of_the_tick_rather_than_inside_it() {
    // Inserting mid-tick would put a node into a tree another script may be
    // walking, and which of them saw it would depend on traversal order.
    let script = "local done = false\nfunction on_tick(self)\n  if not done then\n    done = true\n    scene.spawn(\"prefabs/bullet\", vec2(1, 2))\n    self.seen_immediately = #scene.tagged(\"bullet\")\n  else\n    self.seen_next_tick = #scene.tagged(\"bullet\")\n  end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("seen_immediately"), Some(&Value::Int(0)));
    assert_eq!(vars.get("seen_next_tick"), Some(&Value::Int(1)));
}

#[test]
fn a_spawned_prefab_brings_its_children() {
    let mut sim = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/bullet\", vec2(0, 0)) end\nend\n",
    );
    run(&mut sim, 1);
    let state = sim.state();
    let bullet = state
        .scene
        .walk()
        .into_iter()
        .find(|id| state.scene.get(*id).is_some_and(|n| n.has_tag("bullet")))
        .expect("the bullet exists");
    assert_eq!(
        state.scene.children(bullet).count(),
        1,
        "the trail came too"
    );
}

#[test]
fn a_spawn_lands_where_it_was_put() {
    let mut sim = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/bullet\", vec2(7, -3)) end\nend\n",
    );
    run(&mut sim, 1);
    let state = sim.state();
    let bullet = state
        .scene
        .walk()
        .into_iter()
        .find(|id| state.scene.get(*id).is_some_and(|n| n.has_tag("bullet")))
        .unwrap();
    assert_eq!(
        state.scene.get(bullet).unwrap().transform.pos,
        Vec2Fx::from_ints(7, -3)
    );
}

#[test]
fn the_id_a_spawn_returns_is_the_id_the_node_gets() {
    // Returned before the node exists, because it is derived rather than drawn
    // — which is what lets a script hold it and find the node next tick.
    let script = "local done = false\nfunction on_tick(self)\n  if not done then\n    done = true\n    self.promised = scene.spawn(\"prefabs/bullet\", vec2(0, 0))\n  else\n    local found = scene.by_id(self.promised)\n    self.found_it = found ~= nil\n  end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("found_it"), Some(&Value::Bool(true)));
}

#[test]
fn spawning_many_gives_every_one_its_own_id_and_name() {
    let mut sim = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then\n    done = true\n    for i = 1, 20 do scene.spawn(\"prefabs/bullet\", vec2(i, 0)) end\n  end\nend\n",
    );
    run(&mut sim, 1);
    assert_eq!(count_named(&sim, "bullet"), 20, "none of them collided");
}

#[test]
fn the_same_run_spawns_the_same_ids_every_time() {
    // Ids are derived from the spawn counter rather than drawn from the RNG, so
    // they reproduce — and spawning one fewer bullet does not shift every
    // gameplay roll after it.
    let script = "local done = false\nfunction on_tick(self)\n  if not done then\n    done = true\n    for i = 1, 5 do scene.spawn(\"prefabs/bullet\", vec2(i, 0)) end\n  end\nend\n";
    let ids = |sim: &Sim| -> Vec<String> {
        let state = sim.state();
        let mut out: Vec<String> = state
            .scene
            .walk()
            .into_iter()
            .filter_map(|id| state.scene.get(id))
            .filter(|n| n.has_tag("bullet"))
            .map(|n| n.uid.to_text())
            .collect();
        out.sort();
        out
    };
    let mut a = sim_with(script);
    let mut b = sim_with(script);
    run(&mut a, 3);
    run(&mut b, 3);
    assert_eq!(ids(&a), ids(&b));
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn spawning_is_in_the_state_hash_so_a_replay_notices_it() {
    let mut quiet = sim_with("function on_tick(self) end\n");
    let mut busy = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/bullet\", vec2(0, 0)) end\nend\n",
    );
    run(&mut quiet, 2);
    run(&mut busy, 2);
    assert_ne!(quiet.hash(), busy.hash());
}

#[test]
fn a_spawn_survives_a_snapshot_and_restore() {
    let script = "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/bullet\", vec2(3, 3)) end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 4);
    let snapshot = sim.snapshot();
    let hash = sim.hash();
    run(&mut sim, 4);
    sim.restore(snapshot);
    assert_eq!(sim.hash(), hash);
    assert_eq!(count_named(&sim, "bullet"), 1);
}

#[test]
fn spawning_a_prefab_that_does_not_exist_is_a_diagnostic_rather_than_a_crash() {
    let mut sim = sim_with(
        "local done = false\nfunction on_tick(self)\n  if not done then done = true; scene.spawn(\"prefabs/nonexistent\", vec2(0, 0)) end\nend\n",
    );
    run(&mut sim, 2);
    assert!(
        sim.diagnostics().to_string().contains("nonexistent"),
        "{}",
        sim.diagnostics()
    );
}

#[test]
fn a_spawned_node_can_be_destroyed_like_any_other() {
    let script = "local n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n == 1 then scene.spawn(\"prefabs/bullet\", vec2(0, 0)) end\n  if n == 3 then\n    local bullets = scene.tagged(\"bullet\")\n    if bullets[1] then bullets[1]:destroy() end\n  end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    assert_eq!(count_named(&sim, "bullet"), 1);
    run(&mut sim, 2);
    assert_eq!(count_named(&sim, "bullet"), 0);
}

// -- sensors that move --------------------------------------------------

#[test]
fn an_area_moves_when_it_is_given_a_velocity() {
    // It used not to: areas were skipped by the sweep entirely, so a projectile
    // authored as one sat where it spawned holding a velocity it could not use.
    let script = "local done = false\nfunction on_tick(self)\n  if not done then\n    done = true\n    scene.spawn(\"prefabs/bullet\", vec2(0, 0))\n  else\n    local b = scene.tagged(\"bullet\")[1]\n    if b then b:set_velocity(vec2(fx.new(60), fx.new(0))) end\n  end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 6);
    let state = sim.state();
    let bullet = state
        .scene
        .walk()
        .into_iter()
        .find(|id| state.scene.get(*id).is_some_and(|n| n.has_tag("bullet")))
        .expect("the bullet exists");
    let x = state.scene.get(bullet).unwrap().transform.pos.x;
    assert!(x > Fx::ZERO, "the sensor never moved: {x:?}");
}

#[test]
fn a_moving_sensor_reports_what_it_passes_through_without_being_pushed_out() {
    // The marker sits at x = 40. A sensor driven into it should report the
    // contact and keep going, rather than being resolved out of it.
    let script = "local n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n == 1 then scene.spawn(\"prefabs/bullet\", vec2(0, 0)) end\n  local b = scene.tagged(\"bullet\")[1]\n  if b then b:set_velocity(vec2(fx.new(240), fx.new(0))) end\nend\n\nfunction on_collide(self, other, normal, trigger)\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 30);
    let state = sim.state();
    let bullet = state
        .scene
        .walk()
        .into_iter()
        .find(|id| state.scene.get(*id).is_some_and(|n| n.has_tag("bullet")));
    let x = state
        .scene
        .get(bullet.expect("the bullet exists"))
        .unwrap()
        .transform
        .pos
        .x;
    assert!(
        x > Fx::from_int(60),
        "a sensor should pass through, not be stopped at {x:?}"
    );
}

// -- spatial queries ----------------------------------------------------

#[test]
fn a_script_can_ask_what_is_near_it() {
    let script = "function on_tick(self)\n  self.near_origin = #scene.near(vec2(0, 0), fx.new(10))\n  self.near_marker = #scene.near(vec2(40, 0), fx.new(10))\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("near_origin"), Some(&Value::Int(0)));
    assert_eq!(vars.get("near_marker"), Some(&Value::Int(1)));
}

#[test]
fn a_query_can_be_filtered_by_tag() {
    let script = "function on_tick(self)\n  self.enemies = #scene.near(vec2(40, 0), fx.new(20), \"enemy\")\n  self.bullets = #scene.near(vec2(40, 0), fx.new(20), \"bullet\")\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("enemies"), Some(&Value::Int(1)));
    assert_eq!(vars.get("bullets"), Some(&Value::Int(0)));
}

#[test]
fn nearest_finds_the_closest_and_nothing_when_the_radius_is_too_small() {
    let script = "function on_tick(self)\n  local n = scene.nearest(vec2(38, 0), fx.new(20), \"enemy\")\n  self.found = n and n:name() or \"none\"\n  local far = scene.nearest(vec2(0, 0), fx.new(5), \"enemy\")\n  self.far = far and far:name() or \"none\"\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("found"), Some(&Value::Str("Marker".to_string())));
    assert_eq!(vars.get("far"), Some(&Value::Str("none".to_string())));
}

// -- edge-triggered input -----------------------------------------------

#[test]
fn a_button_is_pressed_on_the_tick_it_goes_down_and_not_after() {
    let script = "function on_tick(self)\n  if input.pressed(\"fire\") then self.presses = (self.presses or 0) + 1 end\n  if input.held(\"fire\") then self.holds = (self.holds or 0) + 1 end\nend\n";
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/room.lua", script).expect("script loads");
    let mut sim = Sim::new(scene_of(ROOM), 1, Box::new(host), SimConfig::default());

    let mut down = InputFrame::idle(1);
    down.players[0].buttons = dimetric_sim::input::buttons::FIRE;

    // Held for five ticks: one press, five holds.
    for _ in 0..5 {
        sim.step(down.clone());
    }
    sim.step(InputFrame::idle(1));
    for _ in 0..3 {
        sim.step(down.clone());
    }

    let state = sim.state();
    let vars = state
        .vars
        .get(&NodeUid::parse("n_root0000").unwrap())
        .unwrap();
    assert_eq!(vars.get("presses"), Some(&Value::Int(2)), "two down-edges");
    assert_eq!(vars.get("holds"), Some(&Value::Int(8)));
}

#[test]
fn an_edge_survives_a_rollback() {
    // The previous tick's input is state, not something re-derived from the
    // log: a rollback that forgot it would fire every edge-triggered action
    // again on the tick it landed on.
    let script = "function on_tick(self)\n  if input.pressed(\"fire\") then self.presses = (self.presses or 0) + 1 end\nend\n";
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/room.lua", script).expect("script loads");
    let mut sim = Sim::new(scene_of(ROOM), 1, Box::new(host), SimConfig::default());

    let mut down = InputFrame::idle(1);
    down.players[0].buttons = dimetric_sim::input::buttons::FIRE;

    sim.step(down.clone());
    let snapshot = sim.snapshot();
    sim.step(down.clone());
    let after = sim.hash();

    sim.restore(snapshot);
    sim.step(down.clone());
    assert_eq!(sim.hash(), after, "the rollback re-fired the edge");
}
