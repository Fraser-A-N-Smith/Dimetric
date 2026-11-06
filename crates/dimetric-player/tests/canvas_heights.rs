//! A project can use the whole screen of a 16:10 handheld.
//!
//! `project.toml` fixed the canvas and the engine scaled it to the window with
//! letterboxing, so a 16:9 interface on a 1280x800 screen was drawn at 1280x720
//! with bars above and below. A game could not learn the window's shape or
//! choose a canvas height at start-up, so its layout could not grow into the
//! eighty rows it was not allowed to use.
//!
//! The canvas becomes a fact about the **session**, which is why the recording
//! carries it: layout decides which control a click lands on, so a replay that
//! laid out against a different canvas would disagree about which button was
//! pressed. The last test is that one.

use std::path::Path;

use dimetric_host::Project;
use dimetric_player::{Session, SessionConfig};
use dimetric_sim::PlayerInput;

/// A project declaring two canvas heights for one width, with a rail anchored
/// to the bottom so the chosen height is visible in the layout.
fn write_project(root: &Path, heights: &str) {
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(
        root.join("project.toml"),
        format!("[game]\nname = \"Wide\"\n\n[ui]\ncanvas = [1920, 1080]\n{heights}"),
    )
    .expect("settings");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Screen\"\n\
         script = \"script:scripts/rails.lua\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\n\
         [[node]]\nid = \"n_rail0000\"\nkind = \"Panel\"\nname = \"Rail\"\n\
         parent = \"n_root0000\"\n\
         anchor_top = 1.0\nanchor_right = 1.0\nanchor_bottom = 1.0\n\
         offset_top = -120.0\n",
    )
    .expect("scene");
    // The script is the oracle: it checks that the rail it anchored to the
    // bottom actually sits 120 rows above whatever canvas it was given, and a
    // failed `assert` becomes a script-runtime diagnostic the session reports.
    // That needs no accessor on `Session` that only a test would want.
    std::fs::write(
        root.join("scripts/rails.lua"),
        "function on_tick(self)\n\
         \x20 local c = ui.canvas()\n\
         \x20 assert(c.w == 1920, \"width: \" .. tostring(c.w))\n\
         \x20 local top = ui.rect(scene.find(\"/Screen/Rail\")).y\n\
         \x20 assert(\n\
         \x20   tostring(top) == tostring(c.h - 120.0),\n\
         \x20   \"rail at \" .. tostring(top) .. \" on a canvas of \" .. tostring(c.h)\n\
         \x20 )\n\
         \x20 self.seen = c.h\n\
         end\n",
    )
    .expect("script");
}

fn open(root: &Path, window: Option<(u32, u32)>, record: Option<&Path>) -> (Project, Session) {
    let mut project = Project::open(root, 0);
    let session = Session::open(
        &mut project,
        SessionConfig {
            seed: 5,
            scene: "main".to_string(),
            record: record.map(Path::to_path_buf),
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
            suspend: None,
            date: None,
            window,
        },
    )
    .expect("the session opens");
    (project, session)
}

#[test]
fn a_project_declaring_one_size_is_unchanged_by_any_window() {
    // What every project until now gets. The window is offered and ignored,
    // because nothing was declared to choose between.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "");
    for window in [None, Some((1280, 800)), Some((3440, 1440))] {
        let (_, session) = open(dir.path(), window, None);
        assert_eq!(
            (session.canvas().width, session.canvas().height),
            (1920, 1080),
            "{window:?}"
        );
    }
}

#[test]
fn a_sixteen_by_ten_window_gets_the_taller_canvas() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "canvas_heights = [1080, 1200]\n");

    let (_, wide) = open(dir.path(), Some((1920, 1080)), None);
    assert_eq!(wide.canvas().height, 1080, "a 16:9 window keeps 16:9");

    let (_, deck) = open(dir.path(), Some((1280, 800)), None);
    assert_eq!(deck.canvas().height, 1200, "a 16:10 window gets 16:10");
    assert_eq!(deck.canvas().width, 1920, "the width never varies");
}

#[test]
fn a_headless_session_gets_the_projects_own_canvas() {
    // No window to be shaped by, so no choice to make -- which is what keeps
    // every fixture and every capture reproducible whatever ran them.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "canvas_heights = [1080, 1200]\n");
    let (_, session) = open(dir.path(), None, None);
    assert_eq!(session.canvas().height, 1080);
}

#[test]
fn the_scripts_rails_follow_the_canvas_they_were_given() {
    // The point of the whole thing: a rail anchored to the bottom sits 120 rows
    // above whichever canvas the session got, so on the taller one it moves
    // down by the extra 120 rows -- and `ui.rect`, which is what the tick's own
    // layout and hit test use, agrees with `ui.canvas`.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "canvas_heights = [1080, 1200]\n");

    for (window, height) in [((1920u32, 1080u32), 1080), ((1280, 800), 1200)] {
        let (mut project, mut session) = open(dir.path(), Some(window), None);
        assert_eq!(session.canvas().height, height, "{window:?}");
        session.step(&mut project, PlayerInput::default());
        let reported = session.take_diagnostics();
        assert!(
            !reported.has_errors(),
            "{window:?}: the script's own checks failed:\n{reported}"
        );
    }
}

#[test]
fn the_scripts_checks_are_not_vacuous() {
    // The negative control. The same script against a canvas whose height the
    // rail was *not* laid out for: if a wrong canvas passed silently, the test
    // above would prove nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "canvas_heights = [1080, 1200]\n");
    // A width the script refuses, reached by declaring a different one.
    std::fs::write(
        dir.path().join("project.toml"),
        "[game]\nname = \"Wide\"\n\n[ui]\ncanvas = [1280, 1080]\n\
         canvas_heights = [1080, 1200]\n",
    )
    .expect("settings");
    let (mut project, mut session) = open(dir.path(), Some((1920, 1080)), None);
    session.step(&mut project, PlayerInput::default());
    let reported = session.take_diagnostics();
    assert!(
        reported.has_errors(),
        "a canvas the script did not expect has to be reported"
    );
    assert!(
        format!("{reported}").contains("width:"),
        "and it has to be the width check that failed:\n{reported}"
    );
}

#[test]
fn the_recording_carries_the_canvas_the_session_chose() {
    // Layout decides what a click hit, so a replay has to lay out against the
    // same canvas. A project declaring one size writes no line, and its logs
    // are byte-for-byte what they always were.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "canvas_heights = [1080, 1200]\n");
    let log = dir.path().join("run.input");
    let (mut project, mut session) = open(dir.path(), Some((1280, 800)), Some(&log));
    for _ in 0..3 {
        session.step(&mut project, PlayerInput::default());
    }
    session.finish().expect("a recording");
    let text = std::fs::read_to_string(&log).expect("the log");
    assert!(text.contains("canvas 1920 1200"), "{text}");

    // And a project that declares one writes nothing.
    let plain = tempfile::tempdir().expect("tempdir");
    write_project(plain.path(), "");
    let log = plain.path().join("run.input");
    let (mut project, mut session) = open(plain.path(), Some((1280, 800)), Some(&log));
    session.step(&mut project, PlayerInput::default());
    session.finish().expect("a recording");
    let text = std::fs::read_to_string(&log).expect("the log");
    assert!(
        !text.contains("canvas"),
        "a project with one size wrote a canvas line:\n{text}"
    );
}
