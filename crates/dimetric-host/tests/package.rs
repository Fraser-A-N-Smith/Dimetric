//! Staging a project into something you can hand to somebody.

use dimetric_host::package::{self, PackageRequest};
use dimetric_host::Project;

fn sorcerer() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sorcerer")
        .canonicalize()
        .expect("examples/sorcerer exists")
}

/// A scratch directory of this test's own, removed first so a previous run
/// cannot be mistaken for this one's output.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dimetric-package-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn stage_sorcerer(out: &std::path::Path, runtime: Option<std::path::PathBuf>) -> package::Staged {
    let mut project = Project::open(sorcerer(), 0);
    package::stage(
        &mut project,
        PackageRequest {
            platform: package::platform("linux").expect("linux is a target"),
            scene: "arena01.dim".to_string(),
            seed: 42,
            out: Some(out.to_path_buf()),
            runtime,
        },
    )
    .unwrap_or_else(|d| panic!("{d}"))
}

#[test]
fn a_staged_game_carries_what_it_runs_on() {
    let out = scratch("contents");
    let staged = stage_sorcerer(&out, None);

    for expected in [
        "arena01.dim",
        "kinds.toml",
        "scripts/arena.lua",
        "prefabs/skeleton.dim",
        "dimetric.toml",
    ] {
        assert!(
            staged.files.iter().any(|f| f == expected),
            "{expected} was not staged"
        );
        assert!(out.join(expected).exists(), "{expected} is not on disk");
    }
    assert!(staged.bytes > 0);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_staged_game_carries_its_import_cache() {
    // Without it the first frame re-imports every asset, which on a player's
    // machine looks like the game hanging on startup.
    let out = scratch("cache");
    let staged = stage_sorcerer(&out, None);
    assert!(
        staged.files.iter().any(|f| f.starts_with(".import/")),
        "no import cache: {:?}",
        staged.files
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_manifest_says_what_to_boot() {
    let out = scratch("manifest");
    stage_sorcerer(&out, None);
    let manifest = std::fs::read_to_string(out.join(package::MANIFEST)).expect("a manifest");
    assert_eq!(
        package::boot_scene(&manifest).as_deref(),
        Some("arena01.dim")
    );
    assert_eq!(package::boot_seed(&manifest), Some(42));
    assert!(manifest.contains("x86_64-unknown-linux-gnu"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_second_build_does_not_keep_the_first_ones_leftovers() {
    // A file deleted from the project has to disappear from the build, or it
    // ships forever.
    let out = scratch("leftovers");
    stage_sorcerer(&out, None);
    let stale = out.join("deleted-last-week.dim");
    std::fs::write(&stale, "leftover").expect("write");
    stage_sorcerer(&out, None);
    assert!(!stale.exists(), "a previous build's file survived");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn staging_without_a_runtime_warns_rather_than_failing() {
    // The data is still worth having, and `dim` does not compile Rust — so
    // this is a thing the caller has to do, not a thing that went wrong.
    let out = scratch("no-runtime");
    let staged = stage_sorcerer(&out, None);
    assert!(staged.runtime.is_none());
    assert!(!staged.diagnostics.has_errors(), "{}", staged.diagnostics);
    assert!(
        staged
            .diagnostics
            .iter()
            .any(|d| d.message.contains("runtime")),
        "no warning about the missing runtime"
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_runtime_is_copied_in_under_the_projects_name() {
    let out = scratch("runtime");
    let fake = scratch("runtime-src").join("dim-play");
    std::fs::create_dir_all(fake.parent().unwrap()).expect("mkdir");
    std::fs::write(&fake, b"#!/bin/sh\ntrue\n").expect("write");
    let staged = stage_sorcerer(&out, Some(fake.clone()));
    let shipped = staged.runtime.expect("a runtime was copied");
    assert_eq!(shipped.file_name().unwrap(), "sorcerer");
    assert!(shipped.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let from = std::fs::metadata(&fake).unwrap().permissions().mode();
        let to = std::fs::metadata(&shipped).unwrap().permissions().mode();
        assert_eq!(
            from & 0o111,
            to & 0o111,
            "the executable bit did not survive"
        );
    }
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(fake.parent().unwrap());
}

#[test]
fn a_windows_build_names_its_executable_with_an_extension() {
    let out = scratch("windows");
    let fake = scratch("windows-src").join("dim-play.exe");
    std::fs::create_dir_all(fake.parent().unwrap()).expect("mkdir");
    std::fs::write(&fake, b"MZ").expect("write");
    let mut project = Project::open(sorcerer(), 0);
    let staged = package::stage(
        &mut project,
        PackageRequest {
            platform: package::platform("windows").expect("windows is a target"),
            scene: "arena01.dim".to_string(),
            seed: 0,
            out: Some(out.clone()),
            runtime: Some(fake.clone()),
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(staged.runtime.unwrap().file_name().unwrap(), "sorcerer.exe");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(fake.parent().unwrap());
}

#[test]
fn a_scene_the_project_does_not_have_is_refused_before_anything_is_copied() {
    let out = scratch("no-scene");
    let mut project = Project::open(sorcerer(), 0);
    let result = package::stage(
        &mut project,
        PackageRequest {
            platform: package::platform("linux").unwrap(),
            scene: "nope.dim".to_string(),
            seed: 0,
            out: Some(out.clone()),
            runtime: None,
        },
    );
    assert!(result.is_err());
    assert!(!out.exists(), "it made a directory anyway");
}

#[test]
fn a_target_is_named_either_way() {
    assert_eq!(
        package::platform("macos").map(|p| p.triple),
        Some("aarch64-apple-darwin")
    );
    assert_eq!(
        package::platform("aarch64-apple-darwin").map(|p| p.name),
        Some("macos")
    );
    assert!(package::platform("playstation").is_none());
}

#[test]
fn the_file_list_is_the_same_on_every_machine() {
    // Two builds of one commit should be comparable, and directory iteration
    // order is not something to rely on.
    let a = scratch("order-a");
    let b = scratch("order-b");
    let first = stage_sorcerer(&a, None);
    let second = stage_sorcerer(&b, None);
    assert_eq!(first.files, second.files);
    assert_eq!(first.bytes, second.bytes);
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}
