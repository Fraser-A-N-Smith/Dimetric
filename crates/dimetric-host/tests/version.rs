//! An input log records the engine that wrote it. This is that field being read.
//!
//! It had never been. The field was written into every log, `DIM0703` was
//! defined as "recorded against a different scene or engine version", and
//! nothing compared them — so a log recorded by one version replayed against
//! another said nothing at all. 0.1.0 is the release where that starts to
//! matter, because it is the first moment there are two versions in the world.
//!
//! A warning rather than a refusal: a log from another version usually still
//! replays, and refusing would make every version bump a wall. What it buys is
//! the difference between "your change broke this" and "this was recorded by a
//! different engine", which is otherwise an afternoon.

use dimetric_host::Replay;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{InputLog, NoScripts, SimConfig};

const ROOM: &str = r#"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"
"#;

fn scene() -> Scene {
    let out = dimetric_scene::parse(ROOM, "test.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn replay_recorded_by(engine: &str) -> dimetric_host::ReplayReport {
    let log = InputLog::new(1, engine, 1);
    Replay {
        log: &log,
        ticks: Some(4),
        expected: None,
        probes: &[],
        clips: Default::default(),
        templates: Default::default(),
    }
    .run(scene(), Box::new(NoScripts), SimConfig::default())
}

#[test]
fn a_log_from_another_engine_version_says_so() {
    let report = replay_recorded_by("0.0.1");
    let warning = report
        .diagnostics
        .iter()
        .find(|d| d.code == dimetric_core::Code::LOG_MISMATCH)
        .expect("the mismatch is reported");
    assert!(warning.message.contains("0.0.1"), "{}", warning.message);
    assert!(
        warning.message.contains(env!("CARGO_PKG_VERSION")),
        "{}",
        warning.message
    );

    // A warning, so the replay still ran and still passed.
    assert!(!report.diagnostics.has_errors(), "{}", report.diagnostics);
    assert!(report.passed());
    assert_eq!(report.ticks, 4);
}

#[test]
fn a_log_from_this_engine_version_is_quiet() {
    let report = replay_recorded_by(env!("CARGO_PKG_VERSION"));
    assert!(
        report.diagnostics.iter().next().is_none(),
        "{}",
        report.diagnostics
    );
}

/// A log with no engine recorded is older than the field, not from another
/// version. Warning about it would be warning about nothing.
#[test]
fn a_log_with_no_engine_recorded_is_quiet() {
    let report = replay_recorded_by("");
    assert!(
        report.diagnostics.iter().next().is_none(),
        "{}",
        report.diagnostics
    );
}
