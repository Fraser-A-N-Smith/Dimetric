//! Replays every fixture under `tests/replay/` and checks it against the
//! hashes checked in beside it.
//!
//! This is the gate the design document puts before the renderer: determinism
//! added later is determinism never achieved. CI runs it on all three target
//! platforms, and a fixture whose hash differs is a build failure rather than a
//! code-review comment.

use std::path::{Path, PathBuf};

use dimetric_host::replay::{parse_probes, HashLog};
use dimetric_host::{Project, Replay};
use dimetric_sim::{InputLog, LuaHost, SimConfig};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/replay")
        .canonicalize()
        .expect("tests/replay exists")
}

fn fixtures() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("tests/replay is readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("scene.dim").exists())
        .collect();
    found.sort();
    found
}

/// Replay one fixture and return its report.
fn replay(dir: &Path) -> dimetric_host::ReplayReport {
    let mut project = Project::open(dir, 0);
    project
        .load_scene("scene")
        .unwrap_or_else(|d| panic!("{}: {d}", dir.display()));

    let log = InputLog::parse(&read(dir, "run.input")).expect("input log parses");
    let probes = match dir.join("run.probes").exists() {
        true => parse_probes(&read(dir, "run.probes")).expect("probes parse"),
        false => Vec::new(),
    };
    let recorded = HashLog::parse(&read(dir, "run.hashes")).expect("hash log parses");

    let (scene, mut diags) = project
        .runtime_scene()
        .unwrap_or_else(|d| panic!("{}: {d}", dir.display()));
    diags.extend(project.load_scripts());
    let mut host = LuaHost::new(60).expect("lua host");
    for (path, source) in &project.scripts {
        host.load(path, source)
            .unwrap_or_else(|d| panic!("{}: {d}", dir.display()));
    }
    assert!(!diags.has_errors(), "{}: {diags}", dir.display());

    Replay {
        log: &log,
        ticks: Some(recorded.hashes.len() as u64),
        expected: Some(&recorded.hashes),
        probes: &probes,
        clips: project.clips(),
        templates: project.templates().0,
    }
    .run(scene, Box::new(host), SimConfig::default())
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_else(|e| panic!("{}/{name}: {e}", dir.display()))
}

#[test]
fn there_are_fixtures_to_replay() {
    // A suite that silently finds nothing passes forever and tests nothing.
    assert!(
        !fixtures().is_empty(),
        "no fixtures found under {}",
        fixtures_dir().display()
    );
}

#[test]
fn every_fixture_replays_to_its_recorded_hashes() {
    for dir in fixtures() {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let report = replay(&dir);

        if let Some(divergence) = &report.divergence {
            panic!(
                "{name}: {}\n\nEither you changed behaviour on purpose — in which case \
                 re-record the fixture and read the diff — or determinism broke.",
                dimetric_host::replay::describe(divergence)
            );
        }
        let failed: Vec<String> = report
            .probes
            .iter()
            .filter(|p| !p.passed)
            .map(|p| p.to_string())
            .collect();
        assert!(
            failed.is_empty(),
            "{name}: {} probes failed:\n  {}",
            failed.len(),
            failed.join("\n  ")
        );
        assert!(
            !report.diagnostics.has_errors(),
            "{name}: {}",
            report.diagnostics
        );
    }
}

#[test]
fn replaying_a_fixture_twice_gives_the_same_hashes() {
    // Guards the case a recorded log cannot: a run reproducible against its own
    // recording but not against itself, which is what a stray HashMap iteration
    // looks like from the outside.
    for dir in fixtures() {
        let first = replay(&dir);
        let second = replay(&dir);
        assert_eq!(
            first.hashes,
            second.hashes,
            "{} is not reproducible within one process",
            dir.display()
        );
    }
}

/// The example project, replayed with the exact files the README tells people
/// to use.
///
/// Without this the README rots quietly: the commands keep parsing, the example
/// keeps loading, and the recorded run stops matching without anyone noticing
/// until they copy a command out of the documentation and it fails.
#[test]
fn the_example_project_replays_as_the_readme_says_it_does() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sorcerer")
        .canonicalize()
        .expect("examples/sorcerer exists");

    let mut project = Project::open(&dir, 0);
    project
        .load_scene("arena01")
        .unwrap_or_else(|d| panic!("{d}"));

    let log = InputLog::parse(&read(&dir, "tests/full-run.input")).expect("input log");
    let recorded = HashLog::parse(&read(&dir, "tests/arena01.hashes")).expect("hash log");
    let probes = parse_probes(&read(&dir, "tests/arena01.probes")).expect("probes");

    let (scene, mut diags) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
    diags.extend(project.load_scripts());
    let mut host = LuaHost::new(60).expect("lua host");
    for (path, source) in &project.scripts {
        host.load(path, source)
            .unwrap_or_else(|d| panic!("{path}: {d}"));
    }
    assert!(!diags.has_errors(), "{diags}");

    let report = Replay {
        log: &log,
        ticks: Some(recorded.hashes.len() as u64),
        expected: Some(&recorded.hashes),
        probes: &probes,
        clips: project.clips(),
        templates: project.templates().0,
    }
    .run(scene, Box::new(host), SimConfig::default());

    if let Some(divergence) = &report.divergence {
        panic!("{}", dimetric_host::replay::describe(divergence));
    }
    let failed: Vec<String> = report
        .probes
        .iter()
        .filter(|p| !p.passed)
        .map(|p| p.to_string())
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n  "));
    assert!(!report.diagnostics.has_errors(), "{}", report.diagnostics);
}
