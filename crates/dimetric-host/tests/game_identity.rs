//! A game names its own window and gives it an icon.
//!
//! The window is created before any of the project has been read and no script
//! can reach it, so neither of these can come from the game at runtime. With
//! `--single` the manifest is folded into the executable at build time, so no
//! post-build step can edit it either. They have to be declared and carried.
//!
//! Both are host presentation, on the same side of the line as audio: they
//! reach a window manager and nothing else. Nothing below the runtime reads
//! them, so nothing here can reach the state hash.

use dimetric_host::package::{self, PackageRequest};
use dimetric_host::settings::Settings;
use dimetric_host::Project;

/// Write a small PNG, through the engine's own encoder.
fn write_png(path: &std::path::Path) {
    let pixels: Vec<u8> = (0..16 * 16)
        .flat_map(|_| [0x3a, 0x2c, 0x5e, 0xff])
        .collect();
    dimetric_assets::encode_png(path, &pixels, 16, 16).expect("the icon encodes");
}

/// A project in `<tempdir>/confluence`, so the directory name is distinct from
/// anything a `[game]` section might say.
fn project(dir: &std::path::Path, settings: &str) -> std::path::PathBuf {
    let root = dir.join("confluence");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(root.join("project.toml"), settings).expect("settings");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    root
}

struct Built {
    manifest: String,
    files: Vec<String>,
    warnings: Vec<String>,
}

fn build(root: &std::path::Path, name: Option<&str>) -> Built {
    let mut project = Project::open(root, 0);
    let staged = package::stage(
        &mut project,
        PackageRequest {
            platform: package::platform("linux").expect("linux"),
            scene: "main.dim".to_string(),
            seed: dimetric_host::package::BootSeed::Fixed(0),
            out: Some(root.join("out")),
            runtime: None,
            name: name.map(str::to_string),
        },
    )
    .expect("the project stages");
    Built {
        manifest: std::fs::read_to_string(staged.out.join(package::MANIFEST)).expect("manifest"),
        warnings: staged
            .diagnostics
            .0
            .iter()
            .map(|d| format!("{d}"))
            .collect(),
        files: staged.files,
    }
}

#[test]
fn a_declared_name_and_icon_reach_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(
        dir.path(),
        "[game]\nname = \"Confluence\"\nicon = \"icon.png\"\n",
    );
    write_png(&root.join("icon.png"));

    let built = build(&root, None);
    assert_eq!(
        package::game_name(&built.manifest).as_deref(),
        Some("Confluence")
    );
    assert_eq!(
        package::game_icon(&built.manifest).as_deref(),
        Some("icon.png")
    );
    assert!(
        built.files.iter().any(|f| f == "icon.png"),
        "the icon did not ship: {:?}",
        built.files
    );
}

#[test]
fn a_project_that_says_nothing_gets_what_it_always_got() {
    // The directory's name, and no icon. The point of the fallback: a project
    // built before any of this existed builds the same way afterwards.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(dir.path(), "[sim]\ntick_rate = 60\n");
    let built = build(&root, None);
    assert_eq!(
        package::game_name(&built.manifest).as_deref(),
        Some("confluence")
    );
    assert_eq!(package::game_icon(&built.manifest), None);
    // Nothing about the window. These tests stage without a runtime, which has
    // a warning of its own.
    assert!(
        !built.warnings.iter().any(|w| w.contains("DIM1201")),
        "{:?}",
        built.warnings
    );
}

#[test]
fn the_name_flag_wins_over_the_project_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(dir.path(), "[game]\nname = \"Confluence\"\n");
    let built = build(&root, Some("Confluence Demo"));
    assert_eq!(
        package::game_name(&built.manifest).as_deref(),
        Some("Confluence Demo")
    );
}

#[test]
fn an_icon_under_assets_is_not_staged_twice() {
    // `assets/` is already copied wholesale, so the icon is in the staged game
    // before anything looks for it. Copying it again would double-count the
    // bytes and list the file twice.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(dir.path(), "[game]\nicon = \"assets/icon.png\"\n");
    std::fs::create_dir_all(root.join("assets")).expect("assets");
    write_png(&root.join("assets/icon.png"));

    let built = build(&root, None);
    assert_eq!(
        built
            .files
            .iter()
            .filter(|f| *f == "assets/icon.png")
            .count(),
        1,
        "{:?}",
        built.files
    );
    assert_eq!(
        package::game_icon(&built.manifest).as_deref(),
        Some("assets/icon.png")
    );
}

#[test]
fn an_icon_that_is_not_there_is_a_warning_and_not_a_failed_build() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(dir.path(), "[game]\nicon = \"icon.png\"\n");
    let built = build(&root, None);
    assert!(
        built.warnings.iter().any(|w| w.contains("DIM1201")),
        "{:?}",
        built.warnings
    );
    // Omitted rather than written: there is one runtime behaviour for a game
    // with no icon, and the build already said which happened.
    assert_eq!(package::game_icon(&built.manifest), None);
}

#[test]
fn an_icon_that_is_not_a_png_is_reported_at_build_time() {
    // At build time, where the file is in front of the person who chose it —
    // not at launch, on somebody else's machine, where nothing can be done.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(dir.path(), "[game]\nicon = \"icon.png\"\n");
    std::fs::write(root.join("icon.png"), b"this is not a png").expect("icon");
    let built = build(&root, None);
    assert!(
        built.warnings.iter().any(|w| w.contains("DIM1201")),
        "{:?}",
        built.warnings
    );
    assert_eq!(package::game_icon(&built.manifest), None);
}

#[test]
fn an_icon_from_outside_the_project_is_refused() {
    // A staged build carries the files the project contains. An icon from
    // somewhere else on the machine would be the one thing staging is for not
    // doing — and it would not be there on anybody else's.
    let dir = tempfile::tempdir().expect("tempdir");
    write_png(&dir.path().join("elsewhere.png"));
    let root = project(dir.path(), "[game]\nicon = \"../elsewhere.png\"\n");
    let built = build(&root, None);
    assert!(
        built.warnings.iter().any(|w| w.contains("DIM1201")),
        "{:?}",
        built.warnings
    );
    assert!(
        !built.files.iter().any(|f| f.contains("elsewhere")),
        "{:?}",
        built.files
    );
}

#[test]
fn a_game_section_of_the_wrong_shape_is_reported() {
    let (settings, diagnostics) = Settings::parse("[game]\nname = 7\n", "project.toml");
    assert_eq!(settings.game.name, None);
    assert!(
        diagnostics
            .0
            .iter()
            .any(|d| d.code == dimetric_core::Code::SETTINGS_INVALID),
        "{diagnostics}"
    );

    let (settings, diagnostics) = Settings::parse("[game]\nicon = []\n", "project.toml");
    assert_eq!(settings.game.icon, None);
    assert!(
        diagnostics
            .0
            .iter()
            .any(|d| d.code == dimetric_core::Code::SETTINGS_INVALID),
        "{diagnostics}"
    );
}

#[test]
fn an_empty_name_is_reported_rather_than_shown() {
    // A window with a blank title bar looks like a bug in the engine. Saying so
    // beats showing it.
    let (settings, diagnostics) = Settings::parse("[game]\nname = \"  \"\n", "project.toml");
    assert_eq!(settings.game.name, None);
    assert!(diagnostics
        .0
        .iter()
        .any(|d| d.code == dimetric_core::Code::SETTINGS_INVALID));
}

#[test]
fn the_game_section_is_not_part_of_the_replay_contract() {
    // Structural, not a convention: everything a run depends on is a field on
    // `Settings`, and these two are behind `Settings::game`. A change here
    // cannot reach the simulation's configuration at all.
    let named = Settings::parse(
        "[game]\nname = \"Confluence\"\nicon = \"icon.png\"\n[sim]\ntick_rate = 30\n",
        "project.toml",
    )
    .0;
    let bare = Settings::parse("[sim]\ntick_rate = 30\n", "project.toml").0;
    assert_eq!(named.tick_rate, bare.tick_rate);
    assert_eq!(named.canvas, bare.canvas);
    assert_eq!(named.resolution, bare.resolution);
    assert_eq!(named.bindings, bare.bindings);
    assert_ne!(named.game, bare.game);
}
