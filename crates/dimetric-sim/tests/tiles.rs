//! Reading and writing a tile grid from a script.
//!
//! The two properties worth guarding are the ones that make a generated floor
//! reproducible: a read during a tick sees the grid as it was when the tick
//! began, and writes land in a defined order at a phase boundary.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

fn load(src: &str) -> Scene {
    let out = dimetric_scene::parse(src, "tiles.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

const FLOOR: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "World"

[[node]]
id = "n_floor000"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"
tileset = "asset:tiles/dungeon"
cell = [16, 16]

[[node]]
id = "n_actor000"
kind = "Node2D"
name = "Actor"
parent = "n_root0000"
script = "script:scripts/actor.lua"
"##;

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/actor.lua", script).expect("loads");
    Sim::new(load(FLOOR), 7, Box::new(host), SimConfig::default())
}

fn step(sim: &mut Sim, n: u32) {
    for _ in 0..n {
        sim.step(InputFrame::idle(1));
    }
    let diagnostics = sim.take_diagnostics();
    assert!(
        !diagnostics.has_errors(),
        "script reported errors: {diagnostics}"
    );
}

fn tile_at(sim: &Sim, x: i32, y: i32) -> u16 {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Floor").expect("layer");
    let uid = state.scene.get(id).expect("layer").uid;
    dimetric_scene::chunk::tile_at(&state.scene.chunks, uid, x, y)
}

#[test]
fn a_script_can_write_a_tile_and_read_it_back_next_tick() {
    let mut sim = sim_with(
        r#"
function on_ready(self)
  self.done = false
end

function on_tick(self)
  local floor = scene.find("/World/Floor")
  if not self.done then
    tiles.set(floor, 3, 4, 7)
    self.done = true
  else
    self.seen = tiles.get(floor, 3, 4)
  end
end
"#,
    );
    step(&mut sim, 1);
    assert_eq!(tile_at(&sim, 3, 4), 7, "the write did not land");
    step(&mut sim, 1);

    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    assert_eq!(
        state.vars[&uid].get("seen").and_then(|v| v.as_int()),
        Some(7)
    );
}

#[test]
fn a_write_is_not_visible_until_the_tick_that_made_it_has_ended() {
    // The property that makes the grid the same for every script in a tick.
    // Without it, whether a monster saw a wall would depend on where it fell
    // in the traversal.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  local floor = scene.find("/World/Floor")
  tiles.set(floor, 1, 1, 9)
  self.immediately = tiles.get(floor, 1, 1)
end
"#,
    );
    step(&mut sim, 1);
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    assert_eq!(
        state.vars[&uid].get("immediately").and_then(|v| v.as_int()),
        Some(0),
        "a read in the same tick must see the grid as it was at the start"
    );
    assert_eq!(tile_at(&sim, 1, 1), 9, "and the write still lands");
}

#[test]
fn an_unpainted_cell_reads_as_empty_rather_than_failing() {
    // A generator checks the neighbours of an edge cell constantly. Making
    // that an error would mean bounds-checking every read.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  local floor = scene.find("/World/Floor")
  self.far = tiles.get(floor, 9999, -9999)
end
"#,
    );
    step(&mut sim, 1);
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    assert_eq!(
        state.vars[&uid].get("far").and_then(|v| v.as_int()),
        Some(0)
    );
}

#[test]
fn a_fill_paints_a_rectangle_and_nothing_outside_it() {
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 0 then
    tiles.fill(scene.find("/World/Floor"), 2, 2, 3, 2, 5)
  end
end
"#,
    );
    step(&mut sim, 1);
    for (x, y) in [(2, 2), (4, 2), (2, 3), (4, 3)] {
        assert_eq!(tile_at(&sim, x, y), 5, "({x}, {y}) should be filled");
    }
    for (x, y) in [(1, 2), (5, 2), (2, 1), (2, 4)] {
        assert_eq!(
            tile_at(&sim, x, y),
            0,
            "({x}, {y}) is outside the rectangle"
        );
    }
}

#[test]
fn a_degenerate_fill_paints_nothing_rather_than_failing() {
    // A generator computing `x1 - x0` for an empty room should get nothing.
    let mut sim = sim_with(
        r#"
function on_tick(self)
  tiles.fill(scene.find("/World/Floor"), 0, 0, -5, -5, 3)
end
"#,
    );
    step(&mut sim, 1);
    assert_eq!(tile_at(&sim, 0, 0), 0);
}

#[test]
fn an_absurd_fill_is_refused_with_a_code() {
    // `rect` comes from a script, and a billion-cell fill is an allocation the
    // process does not survive.
    let mut host = LuaHost::new(60).expect("lua host");
    host.load(
        "scripts/actor.lua",
        r#"
function on_tick(self)
  tiles.fill(scene.find("/World/Floor"), 0, 0, 100000, 100000, 1)
end
"#,
    )
    .expect("loads");
    let mut sim = Sim::new(load(FLOOR), 7, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let diagnostics = sim.take_diagnostics();
    assert!(diagnostics.has_errors());
    assert!(diagnostics.to_string().contains("DIM0505"), "{diagnostics}");
}

#[test]
fn the_tiles_api_refuses_a_node_that_is_not_a_tile_layer() {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load(
        "scripts/actor.lua",
        r#"
function on_tick(self)
  tiles.get(scene.find("/World"), 0, 0)
end
"#,
    )
    .expect("loads");
    let mut sim = Sim::new(load(FLOOR), 7, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let diagnostics = sim.take_diagnostics();
    assert!(diagnostics.has_errors());
    assert!(diagnostics.to_string().contains("DIM0505"), "{diagnostics}");
}

#[test]
fn bounds_reports_where_a_layer_has_storage() {
    let mut sim = sim_with(
        r#"
function on_tick(self)
  local floor = scene.find("/World/Floor")
  if tick.count() == 0 then
    tiles.set(floor, 5, 5, 1)
  else
    local b = tiles.bounds(floor)
    self.bx, self.by, self.bw, self.bh = b.x, b.y, b.w, b.h
  end
end
"#,
    );
    step(&mut sim, 2);
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Actor").expect("actor");
    let uid = state.scene.get(id).expect("actor").uid;
    let get = |k: &str| state.vars[&uid].get(k).and_then(|v| v.as_int());
    // Chunk granularity: one 32x32 chunk at the origin.
    assert_eq!((get("bx"), get("by")), (Some(0), Some(0)));
    assert_eq!((get("bw"), get("bh")), (Some(32), Some(32)));
}

#[test]
fn a_generated_floor_is_the_same_every_time() {
    // The whole point: a generation bug arrives as a seed.
    let generator = r#"
function on_tick(self)
  if tick.count() ~= 0 then return end
  local floor = scene.find("/World/Floor")
  tiles.fill(floor, 0, 0, 24, 24, 1)
  for i = 1, 40 do
    local x = rng.range("map", 0, 23)
    local y = rng.range("map", 0, 23)
    tiles.set(floor, x, y, 2)
  end
end
"#;
    let run = || {
        let mut sim = sim_with(generator);
        step(&mut sim, 2);
        sim.hash()
    };
    let once = run();
    for _ in 0..8 {
        assert_eq!(run(), once, "the same seed produced a different floor");
    }
}

#[test]
fn a_written_tile_moves_the_state_hash() {
    // Tiles are in the scene and the scene is hashed, so this should hold
    // without anything extra — asserted because a grid outside the hash would
    // be a replay that diverges silently.
    let plain = {
        let mut sim = sim_with("function on_tick(self) end");
        step(&mut sim, 2);
        sim.hash()
    };
    let painted = {
        let mut sim = sim_with(
            r#"
function on_tick(self)
  if tick.count() == 0 then
    tiles.set(scene.find("/World/Floor"), 0, 0, 1)
  end
end
"#,
        );
        step(&mut sim, 2);
        sim.hash()
    };
    assert_ne!(plain, painted);
}

#[test]
fn a_snapshot_restores_the_grid() {
    let mut sim = sim_with(
        r#"
function on_tick(self)
  if tick.count() == 1 then
    tiles.set(scene.find("/World/Floor"), 2, 2, 4)
  end
end
"#,
    );
    step(&mut sim, 1);
    let before = sim.snapshot();
    let hash = sim.hash();

    step(&mut sim, 2);
    assert_eq!(tile_at(&sim, 2, 2), 4);

    sim.restore(before);
    assert_eq!(sim.hash(), hash);
    assert_eq!(
        tile_at(&sim, 2, 2),
        0,
        "the rollback did not rewind the grid"
    );
}
