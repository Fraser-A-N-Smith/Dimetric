//! `project.toml`: the settings that change what a recorded run means.
//!
//! The distinction the tests are really guarding is between a preference and a
//! contract. A window size is a preference and belongs wherever the player left
//! it. A tick rate and a UI canvas are a contract: two people running the same
//! input log have to agree on them, so they live in a file the project commits.

use dimetric_host::settings::{Settings, DEFAULT_TICK_RATE};
use dimetric_scene::ui::Canvas;

fn parse(text: &str) -> (Settings, Vec<String>) {
    let (settings, diagnostics) = Settings::parse(text, "project.toml");
    let messages = diagnostics.iter().map(|d| d.to_string()).collect();
    (settings, messages)
}

#[test]
fn an_empty_file_is_every_default() {
    let (settings, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.tick_rate, DEFAULT_TICK_RATE);
}

#[test]
fn a_project_can_declare_its_tick_rate_and_canvas() {
    let (settings, problems) = parse(
        r#"
[sim]
tick_rate = 30

[ui]
canvas = [640, 360]
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings.tick_rate, 30);
    assert_eq!(
        settings.canvas,
        Canvas {
            width: 640,
            height: 360
        }
    );
}

#[test]
fn settings_are_independent_of_each_other() {
    // Declaring one must not reset the other to a default. This is the bug
    // that a struct-of-defaults parser makes easy to write.
    let (settings, _) = parse("[ui]\ncanvas = [160, 90]\n");
    assert_eq!(settings.tick_rate, DEFAULT_TICK_RATE);
    assert_eq!(settings.canvas.width, 160);

    let (settings, _) = parse("[sim]\ntick_rate = 120\n");
    assert_eq!(settings.tick_rate, 120);
    assert_eq!(settings.canvas, Canvas::default());
}

#[test]
fn a_malformed_file_is_reported_rather_than_ignored() {
    // The failure mode worth preventing: a typo silently falling back to the
    // default is how a project spends a week wondering why its recordings
    // drift.
    let (settings, problems) = parse("[sim\ntick_rate = 30");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("DIM0901"), "{problems:?}");
    assert_eq!(settings, Settings::default(), "and it still runs");
}

#[test]
fn a_nonsense_tick_rate_is_rejected() {
    for bad in ["0", "-5", "100000", "\"sixty\"", "59.94"] {
        let (settings, problems) = parse(&format!("[sim]\ntick_rate = {bad}\n"));
        assert_eq!(problems.len(), 1, "{bad} was accepted");
        assert!(problems[0].contains("DIM0902"), "{problems:?}");
        assert_eq!(
            settings.tick_rate, DEFAULT_TICK_RATE,
            "a rejected value must not be half-applied"
        );
    }
}

#[test]
fn a_nonsense_canvas_is_rejected() {
    for bad in [
        "[0, 180]",
        "[320]",
        "[320, 180, 1]",
        "320",
        "[\"a\", \"b\"]",
    ] {
        let (settings, problems) = parse(&format!("[ui]\ncanvas = {bad}\n"));
        assert_eq!(problems.len(), 1, "{bad} was accepted");
        assert!(problems[0].contains("DIM0902"), "{problems:?}");
        assert_eq!(settings.canvas, Canvas::default());
    }
}

#[test]
fn a_missing_file_is_not_an_error() {
    // Every setting has a default that works, so a project with no
    // project.toml runs rather than refusing to start.
    let dir = tempfile::tempdir().expect("tempdir");
    let (settings, diagnostics) = Settings::load(dir.path());
    assert!(!diagnostics.has_errors());
    assert_eq!(settings, Settings::default());
}

#[test]
fn a_file_that_is_there_is_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("project.toml"),
        "[ui]\ncanvas = [512, 288]\n",
    )
    .expect("write");
    let (settings, diagnostics) = Settings::load(dir.path());
    assert!(!diagnostics.has_errors());
    assert_eq!(settings.canvas.width, 512);
}
