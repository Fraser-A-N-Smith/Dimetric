//! Saving a run and putting it back exactly.
//!
//! The property every test here is really about: a resumed run has to be the
//! *same* run. A save that restores a nearly-identical state is worse than no
//! save, because the difference surfaces later as a divergence nobody can
//! trace back to the load.

use dimetric_core::Vec2Fx;
use dimetric_host::savefile::{self, SaveFile, SAVE_FORMAT, SAVE_VERSION};
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
id = "n_floor000"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"
tileset = "asset:tiles/dungeon"
cell = [16, 16]

[[node]]
id = "n_hero0000"
kind = "Collider"
name = "Hero"
parent = "n_root0000"
pos = [4.0, 8.0]
size = [16.0, 16.0]
script = "script:scripts/hero.lua"
"##;

const HERO: &str = r#"
function on_ready(self)
  self.hp = 20
  self.stones = { "amber", "jet" }
end

function on_tick(self)
  self.hp = self.hp - 1
  self.roll = rng.range("loot", 1, 100)
  self:set_velocity(vec2(fx.new(1), fx.new(0)))
  if tick.count() == 2 then
    tiles.set(scene.find("/World/Floor"), 3, 3, 9)
  end
end
"#;

fn registry() -> KindRegistry {
    KindRegistry::with_builtins()
}

fn load_scene() -> Scene {
    let out = dimetric_scene::parse(SCENE, "save.dim", &registry());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim() -> Sim {
    sim_with(SimConfig::default())
}

/// A project's settings, rather than the engine's.
///
/// Every test above this line used the defaults, which is how a hashed field
/// went unsaved without anything noticing: a field that happens to equal its
/// default round-trips whether it is written or not.
fn configured() -> SimConfig {
    SimConfig {
        tick_rate: 60,
        canvas: dimetric_scene::ui::Canvas {
            width: 1920,
            height: 1080,
        },
        resolution: (1920, 1080),
    }
}

fn sim_with(config: SimConfig) -> Sim {
    let mut host = LuaHost::new(config.tick_rate).expect("lua host");
    host.load("scripts/hero.lua", HERO).expect("loads");
    Sim::new(load_scene(), 4242, Box::new(host), config)
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(InputFrame::idle(1));
    }
}

#[test]
fn a_saved_run_restores_to_the_same_hash() {
    // The one that matters. Everything else is a way this can go wrong.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 6);
    let want = original.hash();

    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");
    let (restored, _) = savefile::load(dir.path(), &registry()).expect("load");
    assert_eq!(restored.hash(), want);
}

#[test]
fn a_restored_run_continues_identically() {
    // Restoring the same hash is necessary and not sufficient: the run has to
    // carry on the same way, which means the RNG streams are where they were
    // rather than merely looking like it.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 6);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let mut straight_through = original.snapshot();
    let expected: Vec<_> = {
        let mut host = LuaHost::new(60).expect("host");
        host.load("scripts/hero.lua", HERO).expect("loads");
        let mut s = Sim::new(load_scene(), 4242, Box::new(host), SimConfig::default());
        s.restore(std::mem::replace(
            &mut straight_through,
            original.snapshot(),
        ));
        (0..5)
            .map(|_| {
                s.step(InputFrame::idle(1));
                s.hash()
            })
            .collect()
    };

    let (state, _) = savefile::load(dir.path(), &registry()).expect("load");
    let mut host = LuaHost::new(60).expect("host");
    host.load("scripts/hero.lua", HERO).expect("loads");
    let mut resumed = Sim::new(load_scene(), 4242, Box::new(host), SimConfig::default());
    resumed.restore(state);
    let actual: Vec<_> = (0..5)
        .map(|_| {
            resumed.step(InputFrame::idle(1));
            resumed.hash()
        })
        .collect();

    assert_eq!(
        actual, expected,
        "the resumed run diverged from the original"
    );
}

#[test]
fn script_variables_survive_including_lists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 3);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let (state, _) = savefile::load(dir.path(), &registry()).expect("load");
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    let uid = state.scene.get(id).expect("hero").uid;
    let vars = state.vars.get(&uid).expect("vars restored");
    assert_eq!(vars.get("hp").and_then(|v| v.as_int()), Some(17));
    assert!(
        matches!(vars.get("stones"), Some(dimetric_scene::Value::List(l)) if l.len() == 2),
        "an ordered list has to come back ordered"
    );
}

#[test]
fn tiles_a_script_painted_survive() {
    // The grid lives in the scene, and the scene is written as `.dim` text, so
    // this is really asserting that the save reuses the writer I2 guarantees.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 5);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let (state, _) = savefile::load(dir.path(), &registry()).expect("load");
    let id = state.scene.resolve_path("/World/Floor").expect("floor");
    let uid = state.scene.get(id).expect("floor").uid;
    assert_eq!(
        dimetric_scene::chunk::tile_at(&state.scene.chunks, uid, 3, 3),
        9
    );
}

#[test]
fn velocity_and_position_survive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 4);
    let before = {
        let s = original.state();
        let id = s.scene.resolve_path("/World/Hero").expect("hero");
        let uid = s.scene.get(id).expect("hero").uid;
        (
            s.scene.get(id).expect("hero").transform.pos,
            s.velocity[&uid],
        )
    };
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let (state, _) = savefile::load(dir.path(), &registry()).expect("load");
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    let uid = state.scene.get(id).expect("hero").uid;
    assert_eq!(state.scene.get(id).expect("hero").transform.pos, before.0);
    assert_eq!(state.velocity[&uid], before.1);
    assert_ne!(before.1, Vec2Fx::ZERO, "the test should have moved it");
}

#[test]
fn the_save_is_text_a_person_can_read() {
    // A binary savefile would be the one place this engine's bet — that a bug
    // arrives as a seed and a diff — stopped being true.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 2);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let state_text = std::fs::read_to_string(dir.path().join("state.toml")).expect("state");
    assert!(state_text.contains("dimetric-save"));
    assert!(state_text.contains("seed = 4242"));

    let scene_text = std::fs::read_to_string(dir.path().join("scene.dim")).expect("scene");
    assert!(scene_text.starts_with("format = \"dimetric\""));
    assert!(scene_text.contains("name = \"Hero\""));
}

#[test]
fn a_save_from_another_engine_version_is_refused() {
    // A warning would be wrong here. An input log that replays wrong
    // announces itself as a divergence; a save that restores wrong just keeps
    // playing.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 1);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let path = dir.path().join("state.toml");
    let text = std::fs::read_to_string(&path).expect("state");
    let doctored = text.replace(
        &format!("engine = \"{}\"", env!("CARGO_PKG_VERSION")),
        "engine = \"0.0.1-ancient\"",
    );
    assert_ne!(doctored, text, "the version line should be in the file");
    std::fs::write(&path, doctored).expect("write");

    let err = savefile::load(dir.path(), &registry()).expect_err("should refuse");
    assert_eq!(err.code.0, "DIM1002");
    assert!(err.message.contains("0.0.1-ancient"), "{}", err.message);
}

#[test]
fn a_save_from_a_future_format_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim();
    run(&mut original, 1);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let path = dir.path().join("state.toml");
    let text = std::fs::read_to_string(&path).expect("state");
    std::fs::write(
        &path,
        text.replace(
            &format!("version = {SAVE_VERSION}"),
            &format!("version = {}", SAVE_VERSION + 1),
        ),
    )
    .expect("write");

    let err = savefile::load(dir.path(), &registry()).expect_err("should refuse");
    assert_eq!(err.code.0, "DIM1002");
}

#[test]
fn a_file_that_is_not_a_save_is_refused_by_name() {
    let mut file = SaveFile::capture(&sim().state(), "scene");
    file.format = "something-else".to_string();
    let err = file.restore(load_scene()).expect_err("should refuse");
    assert_eq!(err.code.0, "DIM1001");
    assert_eq!(file.format, "something-else");
    assert_eq!(SAVE_FORMAT, "dimetric-save");
}

#[test]
fn a_missing_save_says_which_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = savefile::load(dir.path(), &registry()).expect_err("nothing there");
    assert_eq!(err.code.0, "DIM1001");
    assert!(err.message.contains("state.toml"), "{}", err.message);
}

#[test]
fn a_run_on_a_projects_own_settings_restores_to_the_same_hash() {
    // The generalisation of the test at the top of this file, and the one that
    // would have caught `resolution` going unsaved: every other round-trip
    // here runs on `SimConfig::default()`, where a dropped field restores to
    // the value it had anyway.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim_with(configured());
    run(&mut original, 6);
    let want = original.hash();

    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");
    let (restored, _) = savefile::load(dir.path(), &registry()).expect("load");
    assert_eq!(
        restored.hash(),
        want,
        "a resumed run is not the run that was saved"
    );
}

#[test]
fn the_save_carries_every_hashed_setting() {
    // Named individually, so the next field added to the hash and forgotten
    // here fails on the field rather than on an opaque hash mismatch.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut original = sim_with(configured());
    run(&mut original, 2);
    savefile::save(dir.path(), &original.state(), "scene", &registry()).expect("save");

    let (state, _) = savefile::load(dir.path(), &registry()).expect("load");
    assert_eq!(state.canvas.width, 1920);
    assert_eq!(state.canvas.height, 1080);
    assert_eq!(state.resolution, (1920, 1080));
}
