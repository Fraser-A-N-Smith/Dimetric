//! A tileset's `[[tile]]` block reaches the atlas, and never reaches the hash.
//!
//! Two halves, and the second is the one that matters. A tile that animates by
//! stepping the map would work, look right, and quietly make the state hash of
//! a recorded run depend on the art: change a `.meta` and a replay that passed
//! yesterday diverges today. So the proof this feature needs is not that water
//! ripples — the renderer's own tests cover that — but that a project whose
//! tileset ripples hashes exactly like one whose tileset does not.

use dimetric_host::render::build_atlas;
use dimetric_host::Project;

/// The tileset's own sidecar, with and without a declared cycle.
const ANIMATED: &str =
    "id = \"a_lu48nofm\"\n\n[[tile]]\nid = 1\nframes = [1, 2, 4]\nframe_ms = 100\n";
const STILL: &str = "id = \"a_lu48nofm\"\n";

const PROJECT: &str = "
[sim]
tick_rate = 60
";

/// A scene with a one-cell floor on the tileset below.
const SCENE: &str = r#"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_floor000"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"
tileset = "asset:tilesets/water"
cell = [16, 16]

[[chunk]]
layer = "n_floor000"
at = [0, 0]
data = "1:1 1023:0"
"#;

/// A project whose tileset carries `meta`.
fn project(meta: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("project.toml"), PROJECT).expect("settings");
    std::fs::write(dir.path().join("main.dim"), SCENE).expect("scene");
    let tilesets = dir.path().join("assets").join("tilesets");
    std::fs::create_dir_all(&tilesets).expect("assets");
    // Four 16x16 slices side by side, so tile ids 1 to 4.
    let pixels = [0x40u8, 0x80, 0xC0, 0xFF].repeat(64 * 16);
    dimetric_assets::encode_png(&tilesets.join("water.png"), &pixels, 64, 16).expect("png");
    std::fs::write(tilesets.join("water.png.meta"), meta).expect("meta");
    dir
}

#[test]
fn a_declared_cycle_arrives_in_the_atlas_already_in_ticks() {
    // The whole path: a sidecar on disk, through the import cache, onto the
    // atlas the renderer is handed. Milliseconds went in and ticks came out,
    // converted against this project's declared tick rate.
    let dir = project(ANIMATED);
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, diagnostics) = project.runtime_scene().expect("the scene opens");
    assert!(!diagnostics.has_errors(), "{diagnostics}");

    let (atlas, diagnostics) = build_atlas(&project, &scene);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let animations = atlas
        .tile_animations("tilesets/water")
        .expect("the sheet's cycles are on the atlas");
    let cycle = animations.cycle(1).expect("tile 1 animates");
    assert_eq!(cycle.frames, vec![1, 2, 4]);
    assert_eq!(cycle.ticks, 6, "100ms at the project's 60Hz");
    assert_eq!(animations.tiles().collect::<Vec<_>>(), vec![1]);
}

#[test]
fn a_tileset_that_declares_nothing_puts_nothing_on_the_atlas() {
    let dir = project(STILL);
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    let (atlas, _) = build_atlas(&project, &scene);
    assert!(
        atlas.tile_animations("tilesets/water").is_none(),
        "the map is empty for almost every project"
    );
}

#[test]
fn a_tick_rate_change_changes_the_ticks_and_not_the_timing() {
    // The conversion happens at import, against the project's rate, for the
    // reason `ms_to_ticks` gives: resolved at runtime, a ripple's speed would
    // depend on whatever rate that session was configured with.
    let dir = project(ANIMATED);
    std::fs::write(
        dir.path().join("project.toml"),
        "\n[sim]\ntick_rate = 120\n",
    )
    .expect("settings");
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    let (atlas, _) = build_atlas(&project, &scene);
    assert_eq!(
        atlas
            .tile_animations("tilesets/water")
            .and_then(|a| a.cycle(1))
            .map(|c| c.ticks),
        Some(12),
        "the same 100ms is twice the ticks at twice the rate"
    );
}

#[test]
fn a_sidecar_that_does_not_describe_a_cycle_fails_the_import_and_is_left_alone() {
    // The rule the whole sidecar follows: malformed means an opinion that did
    // not survive parsing, so the import fails naming the file and the file
    // stays exactly as it is. An invalid cycle deserves to be rejected; it
    // does not deserve to be corrected by deletion.
    let broken = "id = \"a_lu48nofm\"\n\n[[tile]]\nid = 1\nframes = [1, 0]\n";
    let dir = project(broken);
    let mut project = Project::open(dir.path(), 0);
    let imported = project.import_assets();
    assert!(
        imported
            .failures
            .iter()
            .any(|(name, _)| name == "tilesets/water"),
        "the tileset's import failed: {:?}",
        imported.failures
    );
    assert_eq!(
        std::fs::read_to_string(
            dir.path()
                .join("assets")
                .join("tilesets")
                .join("water.png.meta")
        )
        .expect("the sidecar is still there"),
        broken,
        "the file somebody wrote survived to be looked at"
    );
}

/// The state hash at each tick of a short run of the project at `dir`.
fn hashes(dir: &std::path::Path) -> Vec<dimetric_core::StateHash> {
    let mut project = Project::open(dir, 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, diagnostics) = project.runtime_scene().expect("the scene opens");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let config = project.sim_config();
    let host = project.script_host().expect("a script host");
    let mut sim = dimetric_sim::Sim::new(scene, 7, Box::new(host), config);
    let log = dimetric_sim::InputLog::new(7, env!("CARGO_PKG_VERSION"), 1);
    (0..12)
        .map(|tick| {
            sim.step(log.frame(tick));
            sim.hash()
        })
        .collect()
}

#[test]
fn animating_a_tileset_does_not_move_the_state_hash() {
    // The point of the whole shape. The two projects differ only in whether
    // the tileset's sidecar declares a cycle, and a run of each has to agree
    // at every tick — otherwise an art change breaks a recorded run, and the
    // one invariant this engine is for is gone.
    let animated = project(ANIMATED);
    let still = project(STILL);
    assert_eq!(
        hashes(animated.path()),
        hashes(still.path()),
        "a ripple is not state"
    );
}

#[test]
fn a_run_over_a_declared_tileset_leaves_the_map_as_it_was_painted() {
    // The other way of saying it: if animation ever became a map edit, the
    // chunk would have moved on. Checked against the authored id rather than
    // against a snapshot, so a change that stepped every cell by zero would
    // not pass either.
    let dir = project(ANIMATED);
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    let config = project.sim_config();
    let host = project.script_host().expect("a script host");
    let mut sim = dimetric_sim::Sim::new(scene, 7, Box::new(host), config);
    let log = dimetric_sim::InputLog::new(7, env!("CARGO_PKG_VERSION"), 1);
    for tick in 0..30 {
        sim.step(log.frame(tick));
    }
    let state = sim.state();
    let chunk = state
        .scene
        .chunks
        .first()
        .expect("the floor still has its chunk");
    assert_eq!(chunk.get(0, 0), Some(1), "the id an author painted");
}
