//! A script's `event.emit` reaching the mixer, end to end.
//!
//! The path the window takes: a script emits, the session drains, and the
//! engine applies the kinds it owns. The two halves existed separately —
//! `docs/API.md` said a volume change reaches the mixer this way, and the
//! mixer had `set_bus_gain` — and nothing joined them, so a game's Options
//! screen emitted three events on every launch and all three went nowhere.

use std::path::Path;

use dimetric_audio::{percent_to_gain, Bus};
use dimetric_host::Project;
use dimetric_player::{Session, SessionConfig};
use dimetric_sim::PlayerInput;

/// A project whose script sets its volumes on the first tick, the way an
/// Options screen does on launch.
fn write_project(root: &Path) {
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(root.join("project.toml"), "[game]\nname = \"Loud\"\n").expect("settings");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/options.lua\"\n",
    )
    .expect("scene");
    std::fs::write(
        root.join("scripts/options.lua"),
        "function on_ready(self)\n\
         \x20 event.emit(\"audio.bus_volume\", { bus = \"Music\", percent = 80 })\n\
         \x20 event.emit(\"audio.bus_volume\", { bus = \"Sfx\", percent = 100 })\n\
         \x20 event.emit(\"audio.bus_volume\", { bus = \"Ui\", percent = 100 })\n\
         end\n\
         function on_tick(self)\n\
         \x20 -- A player choosing Off on the Options screen, a few ticks in.\n\
         \x20 if tick.count() == 4 then\n\
         \x20   event.emit(\"audio.bus_volume\", { bus = \"Music\", percent = 0 })\n\
         \x20 end\n\
         \x20 -- And changing their mind.\n\
         \x20 if tick.count() == 8 then\n\
         \x20   event.emit(\"audio.bus_volume\", { bus = \"Music\", percent = 60 })\n\
         \x20 end\n\
         \x20 -- Something the engine does not own, every tick, which has to\n\
         \x20 -- come back out of the drain untouched.\n\
         \x20 event.emit(\"steam.rich_presence\", { floor = tick.count() })\n\
         end\n",
    )
    .expect("script");
}

fn open(root: &Path) -> (Project, Session) {
    let mut project = Project::open(root, 0);
    let session = Session::open(
        &mut project,
        SessionConfig {
            seed: 3,
            scene: "main".to_string(),
            record: None,
            settings: dimetric_render::RenderSettings::default(),
            // Silent still runs the mixer and still decides what would be
            // heard, which is the whole point of having it.
            device: dimetric_audio::Device::Silent,
            profile: None,
            suspend: None,
            date: None,
        },
    )
    .expect("the session opens");
    (project, session)
}

/// Step once, the way the window does: tick, drain, apply.
fn frame(project: &mut Project, session: &mut Session) -> Vec<dimetric_sim::event::GameEvent> {
    session.step(project, PlayerInput::default());
    let events = session.drain_events();
    session.apply_events(&events);
    events
}

#[test]
fn a_scripts_volumes_reach_the_buses() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path());

    assert_eq!(session.bus_gain(Bus::Music), 1.0, "a bus starts at full");
    frame(&mut project, &mut session);
    let reported = session.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");

    assert_eq!(session.bus_gain(Bus::Music), percent_to_gain(80));
    assert_eq!(session.bus_gain(Bus::Sfx), 1.0);
    assert_eq!(session.bus_gain(Bus::Ui), 1.0);
}

#[test]
fn music_goes_off_and_comes_back_while_effects_stay() {
    // What the Options screen does, and the thing the game will check: turning
    // the music off must not take the effects with it.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path());

    for _ in 0..6 {
        frame(&mut project, &mut session);
    }
    assert_eq!(session.bus_gain(Bus::Music), 0.0, "the music is not silent");
    assert_eq!(session.bus_gain(Bus::Sfx), 1.0, "the effects went with it");

    for _ in 0..4 {
        frame(&mut project, &mut session);
    }
    assert_eq!(session.bus_gain(Bus::Music), percent_to_gain(60));
    assert_eq!(session.bus_gain(Bus::Sfx), 1.0);
    let reported = session.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
}

#[test]
fn another_hosts_events_come_back_out_of_the_drain() {
    // The drained list is the whole record of what the simulation said. A
    // runtime that mirrors a volume to an OS mixer, or a Steam integration
    // reading the same list, has to find its own kinds still in it — so
    // applying reads the list rather than consuming from it.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path());

    let events = frame(&mut project, &mut session);
    let theirs: Vec<&str> = events
        .iter()
        .map(|e| e.kind.as_str())
        .filter(|kind| *kind == "steam.rich_presence")
        .collect();
    assert_eq!(theirs.len(), 1, "{events:#?}");
    // And ours are in there too, rather than swallowed.
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "audio.bus_volume")
            .count(),
        3
    );
}

#[test]
fn applying_a_volume_does_not_move_the_state_hash() {
    // The property the whole shape rests on. A run played with the music off
    // has to hash identically to one played with it on, or a recorded session
    // would replay differently depending on a setting in somebody's profile.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());

    let hashes = |apply: bool| {
        let (mut project, mut session) = open(dir.path());
        let mut out = Vec::new();
        for _ in 0..12 {
            session.step(&mut project, PlayerInput::default());
            let events = session.drain_events();
            if apply {
                session.apply_events(&events);
            }
            out.push(session.hash());
        }
        out
    };
    assert_eq!(hashes(true), hashes(false));
}

#[test]
fn a_headless_run_is_free_to_ignore_them() {
    // `dim run` reports the events and has no device to apply them to. Not
    // calling `apply_events` has to be a complete answer rather than a thing
    // that leaves the session half-configured.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path());
    for _ in 0..6 {
        session.step(&mut project, PlayerInput::default());
        let events = session.drain_events();
        assert!(!events.is_empty());
    }
    assert_eq!(session.bus_gain(Bus::Music), 1.0);
    let reported = session.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
}
