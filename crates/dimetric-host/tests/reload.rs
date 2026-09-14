//! Hot reload: what it picks up, what it keeps, and where it is allowed to run.

use dimetric_host::reload::{Change, Reloader};
use dimetric_host::{Project, RunMode};
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_ticker01"
kind = "Node2D"
name = "Ticker"
parent = "n_root0000"
script = "script:scripts/ticker.lua"
"##;

/// Counts up by one a tick and remembers the count in a node variable.
const COUNT_BY_ONE: &str = r#"
function on_tick(self)
  self.count = (self.count or 0) + 1
end
"#;

/// Same state, different arithmetic.
const COUNT_BY_TEN: &str = r#"
function on_tick(self)
  self.count = (self.count or 0) + 10
end
"#;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "dimetric-reload-{name}-{}",
            std::process::id() as u64 * 17 + name.len() as u64
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).expect("temp dir");
        std::fs::write(dir.join("room.dim"), SCENE).expect("scene");
        std::fs::write(dir.join("scripts/ticker.lua"), COUNT_BY_ONE).expect("script");
        Fixture(dir)
    }

    fn open(&self) -> Project {
        let mut project = Project::open(&self.0, 1);
        project.load_scene("room").expect("scene loads");
        project
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
        std::fs::write(path, text).expect("write");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sim_of(project: &mut Project) -> Sim {
    let (scene, diagnostics) = project.runtime_scene().expect("scene resolves");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    project.load_scripts();
    let mut host = LuaHost::new(SimConfig::default().tick_rate).expect("lua");
    for (path, source) in &project.scripts {
        host.load(path, source).expect("script loads");
    }
    Sim::new(scene, 1, Box::new(host), SimConfig::default())
}

fn count(sim: &Sim) -> i64 {
    let state = sim.state();
    let node = state.scene.resolve_path("/Room/Ticker").expect("node");
    let uid = state.scene.get(node).expect("node").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get("count"))
        .and_then(|v| match v {
            dimetric_scene::Value::Int(i) => Some(*i),
            dimetric_scene::Value::Scalar(f) => Some(f.round_int() as i64),
            _ => None,
        })
        .unwrap_or(0)
}

#[test]
fn what_is_on_disk_when_watching_starts_is_not_a_change() {
    let fixture = Fixture::new("baseline");
    let mut project = fixture.open();
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);
    assert_eq!(reloader.poll(&mut project), 0);
}

#[test]
fn an_edited_script_is_picked_up() {
    let fixture = Fixture::new("script");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);
    let log = InputLog::new(1, "test", 1);

    for tick in 0..3 {
        sim.step(log.frame(tick));
    }
    assert_eq!(count(&sim), 3);

    fixture.write("scripts/ticker.lua", COUNT_BY_TEN);
    assert_eq!(reloader.poll(&mut project), 1);
    let (applied, diagnostics) = reloader.apply(&mut project, &mut sim);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(applied.scripts, ["scripts/ticker.lua"]);

    sim.step(log.frame(3));
    // 3 from the old script plus 10 from the new one. The count survived,
    // because script state lives in the simulation rather than in Lua globals —
    // that is the difference between a reload and a restart.
    assert_eq!(count(&sim), 13);
}

#[test]
fn a_script_that_does_not_compile_leaves_the_running_one_alone() {
    let fixture = Fixture::new("broken");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);
    let log = InputLog::new(1, "test", 1);
    sim.step(log.frame(0));

    fixture.write(
        "scripts/ticker.lua",
        "function on_tick(self) this is not lua",
    );
    reloader.poll(&mut project);
    let (applied, diagnostics) = reloader.apply(&mut project, &mut sim);
    assert!(applied.scripts.is_empty());
    assert!(diagnostics.has_errors(), "the failure is reported");

    sim.step(log.frame(1));
    assert_eq!(count(&sim), 2, "the version that compiled keeps running");
}

#[test]
fn a_deleted_script_is_not_torn_out_from_under_the_nodes_using_it() {
    let fixture = Fixture::new("deleted");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);
    let log = InputLog::new(1, "test", 1);
    sim.step(log.frame(0));

    std::fs::remove_file(fixture.0.join("scripts/ticker.lua")).expect("remove");
    reloader.poll(&mut project);
    let (applied, _) = reloader.apply(&mut project, &mut sim);
    assert!(applied.scripts.is_empty());

    sim.step(log.frame(1));
    assert_eq!(count(&sim), 2);
}

#[test]
fn a_changed_scene_is_reported_rather_than_applied() {
    // Reloading the scene mid-run throws away the simulation, which is a
    // restart. The caller is better placed to decide that than this is.
    let fixture = Fixture::new("scene");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);
    let log = InputLog::new(1, "test", 1);
    sim.step(log.frame(0));
    let before = sim.hash();

    fixture.write("room.dim", &SCENE.replace("Ticker", "Counter"));
    reloader.poll(&mut project);
    let (applied, _) = reloader.apply(&mut project, &mut sim);
    assert_eq!(applied.scenes, ["room.dim"]);
    assert_eq!(sim.hash(), before, "the running simulation is untouched");
}

#[test]
fn a_replay_does_not_hot_reload() {
    // A run that picked up an edited script would not reproduce the recording
    // it came from, which is the whole point of a replay.
    let fixture = Fixture::new("replay");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Replay, &mut project);
    let log = InputLog::new(1, "test", 1);

    fixture.write("scripts/ticker.lua", COUNT_BY_TEN);
    assert_eq!(reloader.poll(&mut project), 0, "nothing is even queued");
    let (applied, _) = reloader.apply(&mut project, &mut sim);
    assert!(applied.is_empty());

    for tick in 0..2 {
        sim.step(log.frame(tick));
    }
    assert_eq!(count(&sim), 2, "the recorded script is what ran");
}

#[test]
fn a_new_asset_is_queued_as_an_asset_change() {
    let fixture = Fixture::new("asset");
    let mut project = fixture.open();
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);

    std::fs::create_dir_all(fixture.0.join("assets/sprites")).expect("dir");
    dimetric_assets::encode_png(
        &fixture.0.join("assets/sprites/hero.png"),
        &[255u8, 0, 0, 255].repeat(16),
        4,
        4,
    )
    .expect("png");

    reloader.poll(&mut project);
    let pending: Vec<&Change> = reloader.pending().collect();
    assert_eq!(pending, [&Change::Asset("sprites/hero".to_string())]);
}

#[test]
fn a_reload_that_changes_nothing_is_not_counted_as_one() {
    let fixture = Fixture::new("noop");
    let mut project = fixture.open();
    let mut sim = sim_of(&mut project);
    let mut reloader = Reloader::new(RunMode::Headless, &mut project);

    reloader.poll(&mut project);
    reloader.apply(&mut project, &mut sim);
    assert_eq!(reloader.reloads(), 0);

    fixture.write("scripts/ticker.lua", COUNT_BY_TEN);
    reloader.poll(&mut project);
    reloader.apply(&mut project, &mut sim);
    assert_eq!(reloader.reloads(), 1);
}
