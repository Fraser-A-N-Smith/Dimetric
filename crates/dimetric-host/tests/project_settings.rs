//! What a project declares has to reach what actually runs.
//!
//! Four things start this engine's simulation — the player, the editor's
//! playback, a headless capture, a replay — and each of them used to build its
//! own `SimConfig`. Two read the project and two used `SimConfig::default()`,
//! so `dim frame capture` photographed a differently-configured game from the
//! one the runtime played. Nothing could tell, because a project whose
//! settings happen to equal the defaults is photographed correctly either way:
//! the defect is invisible until the day somebody changes a setting.
//!
//! The renderer had the matching half of it. `[render] resolution` reached
//! `SimConfig.resolution`, which is what a script unprojects a click through,
//! and did not reach `RenderSettings.internal_resolution`, which is what draws
//! — so the number picked against and the number drawn at were independent.

use dimetric_host::Project;

/// A project with the given `project.toml`, and a scene to open.
fn project(toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("project.toml"), toml).expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    dir
}

const DECLARED: &str = r#"
[sim]
tick_rate = 30

[ui]
canvas = [1920, 1080]

[render]
resolution = [960, 540]
"#;

#[test]
fn the_sim_config_is_the_projects_and_not_the_engines() {
    let dir = project(DECLARED);
    let config = Project::open(dir.path(), 0).sim_config();
    assert_eq!(config.tick_rate, 30);
    assert_eq!(config.canvas.width, 1920);
    assert_eq!(config.canvas.height, 1080);
    assert_eq!(config.resolution, (960, 540));
}

#[test]
fn a_project_that_declares_nothing_still_gets_the_defaults() {
    let dir = project("");
    let config = Project::open(dir.path(), 0).sim_config();
    assert_eq!(config, dimetric_sim::SimConfig::default());
}

#[test]
fn the_world_is_drawn_at_the_resolution_the_project_declared() {
    let dir = project(DECLARED);
    let (size, warning) = Project::open(dir.path(), 0).render_resolution(None);
    assert_eq!(size, (960, 540), "this is what `internal_resolution` gets");
    assert!(warning.is_none(), "nothing was overridden");
}

#[test]
fn an_override_is_allowed_and_is_not_silent() {
    // It is allowed because a one-off capture at a size worth looking at is a
    // real thing to want. It is not silent because for as long as the flag is
    // there, picking and drawing disagree — which is the whole defect, just
    // triggered by a flag rather than by a default.
    let dir = project(DECLARED);
    let (size, warning) = Project::open(dir.path(), 0).render_resolution(Some((1920, 1080)));
    assert_eq!(size, (1920, 1080));
    let d = warning.expect("an override says what it costs");
    assert_eq!(d.code, dimetric_core::Code::SETTINGS_OVERRIDDEN);
    assert_eq!(d.severity, dimetric_core::Severity::Warning);
    assert!(d.message.contains("1920x1080"), "{}", d.message);
    assert!(d.message.contains("960x540"), "{}", d.message);
}

#[test]
fn an_override_that_agrees_with_the_project_says_nothing() {
    let dir = project(DECLARED);
    let (size, warning) = Project::open(dir.path(), 0).render_resolution(Some((960, 540)));
    assert_eq!(size, (960, 540));
    assert!(
        warning.is_none(),
        "there is no disagreement to warn about: {warning:?}"
    );
}

/// Copy a directory, so a capture can run against a project of its own.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("readable") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        match entry.file_type().expect("a file type").is_dir() {
            true => copy_dir(&entry.path(), &target),
            false => {
                std::fs::copy(entry.path(), &target).expect("copy");
            }
        }
    }
}

/// The golden fixtures' scenes, under a `project.toml` of this test's choosing.
fn drawable(settings: &str) -> tempfile::TempDir {
    let scenes = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/scenes")
        .canonicalize()
        .expect("the golden scenes exist");
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir(&scenes, dir.path());
    std::fs::write(dir.path().join("project.toml"), settings).expect("settings");
    dir
}

/// Draw `room.dim`, or `None` when this machine has no graphics adapter.
fn draw(root: &std::path::Path) -> Option<Vec<u8>> {
    let mut project = Project::open(root, 0);
    project.load_scene("room").expect("the scene loads");
    let request = dimetric_host::CaptureRequest {
        tick: 1,
        seed: 0,
        input: None,
        size: (480, 270),
        // Deliberately not naming an internal resolution: the point of the
        // test is that the project decides it.
        settings: dimetric_render::RenderSettings {
            internal_resolution: project.render_resolution(None).0,
            ..Default::default()
        },
    };
    match dimetric_host::capture(&mut project, request) {
        Ok(captured) => Some(captured.pixels),
        Err(diagnostics) => {
            // Same contract as the golden tests: CI sets DIMETRIC_REQUIRE_GPU
            // so a runner that lost its driver cannot go green having drawn
            // nothing.
            assert!(
                std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                "DIMETRIC_REQUIRE_GPU is set but rendering failed: {diagnostics}"
            );
            eprintln!("skipping: {diagnostics}");
            None
        }
    }
}

#[test]
fn changing_the_declared_resolution_changes_the_drawn_frame() {
    // The measurement that opened the report: captures at three different
    // resolutions came back byte-identical, because none of the three ever
    // reached anything. Same scene, same tick, same output size — only the
    // project's declared resolution differs.
    let small = drawable("[render]\nresolution = [240, 135]\n");
    let large = drawable("[render]\nresolution = [480, 270]\n");
    let (Some(small), Some(large)) = (draw(small.path()), draw(large.path())) else {
        return;
    };
    assert_ne!(
        small, large,
        "the world was drawn at the same size either way"
    );
}

#[test]
fn a_project_that_declares_nothing_is_drawn_as_it_always_was() {
    // The other half of the guarantee: this change must not move a capture of
    // a project that never asked for anything, which is every golden image.
    let declared = drawable("[render]\nresolution = [480, 270]\n");
    let silent = drawable("");
    let (Some(declared), Some(silent)) = (draw(declared.path()), draw(silent.path())) else {
        return;
    };
    assert_eq!(declared, silent, "the default is still 480x270");
}
