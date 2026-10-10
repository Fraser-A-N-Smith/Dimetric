//! Suspending a run and continuing it, exactly.
//!
//! The test that matters is the last one: a run that stopped at tick N and was
//! continued from disk hashes, tick for tick, like one that never stopped. Any
//! weaker check passes for a resume that restored the tree and lost the RNG
//! streams — which is the shape every rejected alternative had, and which plays
//! on happily while rolling different numbers from the run it claims to be.

use std::path::Path;

use dimetric_core::StateHash;
use dimetric_host::Project;
use dimetric_player::{Session, SessionConfig};
use dimetric_sim::PlayerInput;

/// A project whose script moves state about in a way only an exact resume
/// reproduces: a seeded stream, a running total, and a node property.
///
/// It also carries both halves of the surface under test. `app.suspended()` is
/// false for a session with no save root, so the resume branch is inert there —
/// which is what lets one script text serve the uninterrupted run and the
/// interrupted one, and makes the comparison between them a comparison of the
/// resume rather than of two different programs.
fn write_project(root: &Path) {
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(
        root.join("project.toml"),
        "[game]\nname = \"Suspendable\"\n\n[sim]\ntick_rate = 60\n",
    )
    .expect("settings");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/run.lua\"\n\n\
         [[node]]\nid = \"n_walk0000\"\nkind = \"Node2D\"\nname = \"Walker\"\n\
         parent = \"n_root0000\"\n",
    )
    .expect("scene");
    std::fs::write(
        root.join("scripts/run.lua"),
        "function on_tick(self)\n\
         \x20 local walker = scene.find(\"/World/Walker\")\n\
         \x20 -- A seeded stream, so a resume that lost the streams diverges.\n\
         \x20 local roll = rng.range(\"stride\", 1, 6)\n\
         \x20 self.total = (self.total or 0) + roll\n\
         \x20 -- And something in the tree, so a resume that lost the tree does.\n\
         \x20 walker.pos = walker.pos + vec2(roll, 1)\n\
         \x20 if app.suspended() then\n\
         \x20   app.resume()\n\
         \x20 elseif tick.count() == 20 then\n\
         \x20   app.suspend()\n\
         \x20 end\n\
         end\n",
    )
    .expect("script");
}

fn config(suspend: Option<&Path>) -> SessionConfig {
    recording_config(suspend, None)
}

fn recording_config(suspend: Option<&Path>, record: Option<&Path>) -> SessionConfig {
    SessionConfig {
        seed: 7,
        scene: "main".to_string(),
        record: record.map(Path::to_path_buf),
        settings: dimetric_render::RenderSettings::default(),
        device: dimetric_audio::Device::Silent,
        // No profile: a run and a profile are different kinds of thing, and
        // this test is about one of them.
        profile: None,
        suspend: suspend.map(Path::to_path_buf),
        date: None,
    }
}

/// Open a session on `root`, keeping the project alive beside it.
fn open(root: &Path, suspend: Option<&Path>) -> (Project, Session) {
    let mut project = Project::open(root, 0);
    let session = Session::open(&mut project, config(suspend)).expect("the session opens");
    (project, session)
}

fn idle() -> PlayerInput {
    PlayerInput::default()
}

#[test]
fn a_session_with_nowhere_to_save_says_so_and_runs_on() {
    // The replay's behaviour, and a test's. A script that asks is told no
    // rather than silently ignored: `quit` has nothing to report when a
    // headless run drops it, and this does — the game asked for something it
    // did not get.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path(), None);
    for _ in 0..22 {
        session.step(&mut project, idle());
    }
    let reported = session.take_diagnostics();
    assert!(
        reported
            .0
            .iter()
            .any(|d| d.code == dimetric_core::Code::SUSPEND_REFUSED),
        "{reported}"
    );
    assert_eq!(session.tick(), 22, "the run stopped when it was refused");
    assert!(!session.suspended());
}

#[test]
fn suspending_writes_the_run_and_asks_to_quit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path());
    let (mut project, mut session) = open(dir.path(), Some(save.path()));

    for _ in 0..21 {
        session.step(&mut project, idle());
    }
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert!(
        session.quit_requested(),
        "suspending carries the quit with it"
    );
    assert!(session.suspended(), "nothing was written");

    let slot = dimetric_host::suspend::probe(save.path());
    match slot {
        dimetric_host::suspend::Slot::Ready { scene, tick, seed } => {
            assert_eq!(scene, "main");
            assert_eq!(tick, 21, "the save is the state after the tick that asked");
            assert_eq!(seed, 7);
        }
        other => panic!("{other:?}"),
    }
    // Text on disk, under the root the profile would use, not inside the game.
    let dir_on_disk = dimetric_host::suspend::suspended_dir(save.path());
    assert!(dir_on_disk.join("scene.dim").is_file());
    assert!(dir_on_disk.join("state.toml").is_file());
}

#[test]
fn resuming_consumes_the_save() {
    // Exactly once per save. A run that could be resumed twice is a run a
    // player can suspend, play on, lose, and then take back.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path());

    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    for _ in 0..21 {
        session.step(&mut project, idle());
    }
    drop(session);
    assert!(dimetric_host::suspend::probe(save.path()).ready());

    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    session.step(&mut project, idle());
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert!(
        !dimetric_host::suspend::probe(save.path()).ready(),
        "the save survived being resumed"
    );
    assert!(!session.suspended());
    let _ = project;
}

#[test]
fn discarding_throws_the_run_away() {
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path());
    std::fs::write(
        dir.path().join("scripts/run.lua"),
        "function on_tick(self)\n\
         \x20 if tick.count() == 0 then app.discard_suspended() end\n\
         end\n",
    )
    .expect("script");

    // Something to throw away.
    let registry = dimetric_scene::KindRegistry::with_builtins();
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("scene");
    let (scene, _) = project.runtime_scene().expect("runtime scene");
    let state = dimetric_sim::SimState::new(scene, 1);
    dimetric_host::suspend::write(save.path(), &state, "main", &registry).expect("write");
    assert!(dimetric_host::suspend::probe(save.path()).ready());

    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    session.step(&mut project, idle());
    assert!(!dimetric_host::suspend::probe(save.path()).ready());
}

/// Hashes for `ticks` uninterrupted ticks, one per tick.
fn uninterrupted(root: &Path, ticks: usize) -> Vec<StateHash> {
    let (mut project, mut session) = open(root, None);
    let mut out = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        session.step(&mut project, idle());
        out.push(session.hash());
    }
    out
}

#[test]
fn a_resumed_run_plays_on_identically() {
    // The one that matters.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path());

    let straight = uninterrupted(dir.path(), 40);

    // Stop at tick 21: the script asks on the tick whose `tick.count()` is 20,
    // and the request is honoured after that tick has finished, so the state
    // written is the one at tick 21.
    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    for _ in 0..21 {
        session.step(&mut project, idle());
    }
    let at_suspend = session.hash();
    assert_eq!(
        at_suspend, straight[20],
        "the interrupted run did not match before it was interrupted"
    );
    drop(session);

    // A different session, as a relaunch is.
    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    assert!(session.suspended(), "nothing to continue");

    // One tick on the entry scene, during which the script sees the waiting
    // run and asks for it. The resume lands between that tick and the next.
    session.step(&mut project, idle());
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(
        session.hash(),
        at_suspend,
        "the resumed state is not the state that was written"
    );
    assert_eq!(session.tick(), 21, "the run did not continue its own clock");

    // And on, against the run that never stopped.
    for (tick, want) in straight.iter().enumerate().skip(21) {
        session.step(&mut project, idle());
        assert_eq!(
            session.hash(),
            *want,
            "tick {} diverged after the resume",
            tick + 1
        );
    }
}

#[test]
fn a_stale_save_reads_as_no_save_at_all() {
    // A Continue row that fails when pressed is worse than one that was never
    // shown, so `suspended()` is false for a save this build cannot use — and
    // the reason is reported rather than swallowed.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path());

    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    for _ in 0..21 {
        session.step(&mut project, idle());
    }
    drop(session);

    let state_file = dimetric_host::suspend::suspended_dir(save.path()).join("state.toml");
    let text = std::fs::read_to_string(&state_file).expect("state");
    std::fs::write(
        &state_file,
        text.replace(
            &format!("engine = \"{}\"", env!("CARGO_PKG_VERSION")),
            "engine = \"0.0.1-ancient\"",
        ),
    )
    .expect("rewrite");

    assert!(!dimetric_host::suspend::probe(save.path()).ready());
    let (_project, session) = open(dir.path(), Some(save.path()));
    assert!(!session.suspended());
    let reported = session.diagnostics.to_string();
    assert!(reported.contains("DIM1002"), "{reported}");
    let _ = project;
}

#[test]
fn a_recording_of_a_resumed_run_replays_against_the_save_it_names() {
    // A recorded resumed session is the second half of a run. Replaying its
    // frames from a fresh scene would reproduce something nobody played, so the
    // log says which run it continued — by the hash the save restores to — and
    // travels with that save, because the slot it came out of is emptied by the
    // resume.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    let out = tempfile::tempdir().expect("out");
    write_project(dir.path());

    let straight = uninterrupted(dir.path(), 40);

    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    for _ in 0..21 {
        session.step(&mut project, idle());
    }
    drop(session);

    // A relaunch that records. The resume happens on its first tick.
    let log_path = out.path().join("run.input");
    let mut project = Project::open(dir.path(), 0);
    let mut session = Session::open(
        &mut project,
        recording_config(Some(save.path()), Some(&log_path)),
    )
    .expect("session");
    for _ in 0..20 {
        session.step(&mut project, idle());
    }
    let reported = session.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
    let played_to = session.hash();
    assert_eq!(session.tick(), 40);
    assert_eq!(played_to, straight[39]);
    session.finish().expect("write the log");

    // The log, and the save it kept beside itself.
    let text = std::fs::read_to_string(&log_path).expect("log");
    let log = dimetric_sim::InputLog::parse(&text).expect("the log parses");
    assert_eq!(log.from_tick, 21, "{text}");
    assert_eq!(
        log.frames.len(),
        19,
        "the menu tick was recorded as the run"
    );
    assert_eq!(log.end_tick(), 40);
    let beside = dimetric_host::suspend::sidecar_save(&log_path);
    assert!(beside.is_dir(), "the save the log names was not kept");

    // And it replays, tick for tick, against the run that never stopped.
    let registry = dimetric_scene::KindRegistry::with_builtins();
    let (resumed_state, _) =
        dimetric_host::savefile::load(&beside, &registry).expect("the sidecar loads");
    assert_eq!(
        Some(resumed_state.hash()),
        log.resumed,
        "the log names a different state from the save beside it"
    );

    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("scene");
    project.import_assets();
    let (scene, _) = project.runtime_scene().expect("runtime scene");
    // The scripts, which a replay harness that forgets them replays as a run
    // in which nothing happened — silently, because a node whose script was
    // never loaded is simply not dispatched to.
    assert!(!project.load_scripts().has_errors());
    let mut host = project.script_host().expect("host");
    let script_diags = host.load_all(
        project
            .scripts
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str())),
    );
    assert!(script_diags.is_empty(), "{script_diags:?}");
    let report = dimetric_host::Replay {
        log: &log,
        ticks: None,
        expected: Some(&straight[21..40]),
        probes: &[],
        clips: project.clips(),
        templates: project.templates().0,
        resume: Some(resumed_state),
    }
    .run_in(
        Some(&mut project),
        scene,
        Box::new(host),
        project_config(dir.path()),
    );
    assert!(!report.diagnostics.has_errors(), "{}", report.diagnostics);
    assert!(report.divergence.is_none(), "{:?}", report.divergence);
    assert_eq!(report.hashes.last().copied(), Some(played_to));
}

#[test]
fn a_resumed_log_replayed_without_its_save_is_refused() {
    // Not attempted and reported as a divergence afterwards. A replay of the
    // wrong thing and a replay that broke look identical from the outside, and
    // only one of them is a bug in the engine.
    let mut log = dimetric_sim::InputLog::new(1, "0.1.0", 1);
    log.resumed_from(
        dimetric_core::StateHash::from_hex(&"ab".repeat(32)).expect("hash"),
        21,
        1,
    );

    let refusal = dimetric_host::Replay {
        log: &log,
        ticks: None,
        expected: None,
        probes: &[],
        clips: Default::default(),
        templates: Default::default(),
        resume: None,
    }
    .resume_mismatch()
    .expect("a log that resumed and no save is a mismatch");
    assert_eq!(refusal.code, dimetric_core::Code::LOG_MISMATCH);

    // And a save that is not the one it names.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("scene");
    let (scene, _) = project.runtime_scene().expect("runtime scene");
    let other = dimetric_sim::SimState::new(scene, 99);
    let refusal = dimetric_host::Replay {
        log: &log,
        ticks: None,
        expected: None,
        probes: &[],
        clips: Default::default(),
        templates: Default::default(),
        resume: Some(other),
    }
    .resume_mismatch()
    .expect("the wrong save is a mismatch");
    assert_eq!(refusal.code, dimetric_core::Code::LOG_MISMATCH);
}

/// The project's own settings, which is what a session runs on.
fn project_config(root: &Path) -> dimetric_sim::SimConfig {
    Project::open(root, 0).sim_config()
}
