//! Writing a run out without stopping it.
//!
//! `app.suspend()` was the only way to put a run on disk and it quits, so a
//! game that wanted an autosave had no shape for one: a turn-based campaign
//! lost the whole session to a power cut, and the only honest advice was "save
//! and quit at the end of every turn".
//!
//! The reason it worked that way was deliberate and is written down in
//! `dimetric_sim::suspend`: a run that wrote its save and kept playing lets a
//! player checkpoint, play on, die and resume, which undoes the last minute.
//! That is true, and it is a design decision about reloading rather than
//! anything to do with determinism — so it belongs to the game. `suspend()`
//! still quits and the strict shape is still one call away; `checkpoint()` is
//! the named opt-in.
//!
//! The last test is the one that matters: a run that checkpoints hashes, tick
//! for tick, exactly like one that does not. The request lives on the script
//! host rather than in `SimState` precisely so that it cannot, and that is
//! what keeps a recorded session independent of whether a save happened to
//! land beside it.

use std::path::Path;

use dimetric_core::StateHash;
use dimetric_host::Project;
use dimetric_player::{Session, SessionConfig};
use dimetric_sim::PlayerInput;

/// A project whose script checkpoints on cue and keeps moving state about.
///
/// `every` is how often it checkpoints, `0` for never — one script text for
/// both sides of the hash comparison, so what is compared is the checkpoint
/// and not two different programs.
fn write_project(root: &Path, every: u32) {
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(
        root.join("project.toml"),
        "[game]\nname = \"Autosaving\"\n\n[sim]\ntick_rate = 60\n",
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
        format!(
            r#"
function on_tick(self)
  local walker = scene.find("/World/Walker")
  -- A seeded stream and something in the tree, so a checkpoint that
  -- disturbed either would show up in the hash.
  local roll = rng.range("stride", 1, 6)
  self.total = (self.total or 0) + roll
  walker.pos = walker.pos + vec2(roll, 1)
  -- `tick.count()` is 0-based, so the `> 0` is what keeps the very first
  -- tick from being a checkpoint.
  local every = {every}
  if every > 0 and tick.count() > 0 and tick.count() % every == 0 then
    app.checkpoint()
  end
end
"#
        ),
    )
    .expect("script");
}

fn config(suspend: Option<&Path>) -> SessionConfig {
    SessionConfig {
        seed: 7,
        scene: "main".to_string(),
        record: None,
        settings: dimetric_render::RenderSettings::default(),
        device: dimetric_audio::Device::Silent,
        profile: None,
        suspend: suspend.map(Path::to_path_buf),
        date: None,
    }
}

fn open(root: &Path, suspend: Option<&Path>) -> (Project, Session) {
    let mut project = Project::open(root, 0);
    let session = Session::open(&mut project, config(suspend)).expect("the session opens");
    (project, session)
}

fn idle() -> PlayerInput {
    PlayerInput::default()
}

#[test]
fn a_checkpoint_writes_the_run_and_carries_on_playing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path(), 5);
    let (mut project, mut session) = open(dir.path(), Some(save.path()));

    // Six steps, so the script sees counts 0..=5 and asks on the last of them.
    for _ in 0..6 {
        session.step(&mut project, idle());
    }
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert!(
        !session.quit_requested(),
        "a checkpoint is not a suspend: the session plays on"
    );
    assert!(session.suspended(), "and there is a run on disk to resume");

    match dimetric_host::suspend::probe(save.path()) {
        dimetric_host::suspend::Slot::Ready { scene, tick, seed } => {
            assert_eq!(scene, "main");
            assert_eq!(tick, 6, "the state after the tick that asked");
            assert_eq!(seed, 7);
        }
        other => panic!("{other:?}"),
    }

    // And it really does keep going, which is the whole feature.
    for _ in 0..10 {
        session.step(&mut project, idle());
    }
    assert_eq!(session.tick(), 16);
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
}

#[test]
fn a_later_checkpoint_replaces_the_earlier_one() {
    // One slot. Two would be a save-slot feature, which is a different thing
    // with a UI attached; what a game asked for is an autosave.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path(), 5);
    let (mut project, mut session) = open(dir.path(), Some(save.path()));

    for _ in 0..6 {
        session.step(&mut project, idle());
    }
    let first = match dimetric_host::suspend::probe(save.path()) {
        dimetric_host::suspend::Slot::Ready { tick, .. } => tick,
        other => panic!("{other:?}"),
    };
    assert_eq!(first, 6);

    for _ in 0..5 {
        session.step(&mut project, idle());
    }
    match dimetric_host::suspend::probe(save.path()) {
        dimetric_host::suspend::Slot::Ready { tick, .. } => {
            assert_eq!(tick, 11, "the newest write is the one on disk")
        }
        other => panic!("{other:?}"),
    }
    let diagnostics = session.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
}

#[test]
fn resuming_a_checkpoint_consumes_it() {
    // The anti-scumming the engine does keep: a save is good for one resume,
    // because `resume` deletes it once the state is installed. A game that
    // wants another has to write another.
    let dir = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(dir.path(), 4);
    let (mut project, mut session) = open(dir.path(), Some(save.path()));
    for _ in 0..5 {
        session.step(&mut project, idle());
    }
    assert!(session.suspended());

    // Resume through the same door a script would, from outside a tick.
    std::fs::write(
        dir.path().join("scripts/run.lua"),
        "function on_tick(self)\n\x20 app.resume()\nend\n",
    )
    .expect("script");
    let (mut project2, mut session2) = open(dir.path(), Some(save.path()));
    assert!(session2.suspended(), "the slot is still there");
    session2.step(&mut project2, idle());
    let diagnostics = session2.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert!(
        !session2.suspended(),
        "the resume consumed it, so it cannot be resumed twice"
    );
    let _ = (&mut project, &mut session);
}

#[test]
fn a_session_with_nowhere_to_save_says_so_once_and_runs_on() {
    // A replay's behaviour and a test's. Reported, because the game asked for
    // something and did not get it — but once, not once per turn: a game that
    // checkpoints every turn would otherwise bury everything else `dim replay`
    // has to say under hundreds of identical lines.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), 2);
    let (mut project, mut session) = open(dir.path(), None);
    for _ in 0..20 {
        session.step(&mut project, idle());
    }
    let reported = session.take_diagnostics();
    let refusals = reported
        .0
        .iter()
        .filter(|d| d.code == dimetric_core::Code::SUSPEND_REFUSED)
        .count();
    assert_eq!(refusals, 1, "nine asks, one line about it:\n{reported}");
    assert_eq!(session.tick(), 20, "and the run carried on regardless");
    assert!(!session.suspended());
    // A warning, not an error. This is what makes a checkpoint a no-op line in
    // a recording rather than something that fails the replay: `dim replay`
    // judges a run by its hashes, and a game asking for a save it cannot have
    // is not a divergence.
    assert!(
        !reported.has_errors(),
        "a refused checkpoint must not fail a replay:\n{reported}"
    );
}

#[test]
fn checkpointing_moves_no_hash_at_all() {
    // The invariant. The request sits on the script host and not in
    // `SimState`, so whether a run wrote itself to disk cannot change what it
    // replays to — and `app.suspended()`, which the script reads into a
    // variable every tick, is a host answer that a session with no save root
    // gives as false. Two runs, same seed, same script, one autosaving every
    // other tick and one never: every hash has to agree.
    let saving = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write_project(saving.path(), 2);
    let (mut project_a, mut session_a) = open(saving.path(), Some(save.path()));

    let quiet = tempfile::tempdir().expect("tempdir");
    write_project(quiet.path(), 0);
    let (mut project_b, mut session_b) = open(quiet.path(), None);

    let mut with: Vec<StateHash> = Vec::new();
    let mut without: Vec<StateHash> = Vec::new();
    for _ in 0..12 {
        session_a.step(&mut project_a, idle());
        session_b.step(&mut project_b, idle());
        with.push(session_a.hash());
        without.push(session_b.hash());
    }
    let diagnostics = session_a.take_diagnostics();
    assert!(!diagnostics.has_errors(), "{diagnostics}");

    assert_eq!(
        with, without,
        "a run that checkpointed hashed differently from one that did not"
    );
    assert!(
        dimetric_host::suspend::probe(save.path()).ready(),
        "and the saving run really did write one, so the comparison means something"
    );
}

#[test]
fn reading_whether_a_save_exists_does_move_the_hash() {
    // The caveat, on the record rather than left to be found in a failing
    // replay. `app.suspended()` is a host answer about a file beside the game,
    // so a script that writes it into state has made its state depend on that
    // file — and a replay is deliberately told `false`, because a recorded
    // session must reproduce on a machine that has no save at all.
    //
    // This is the same shape as reading your own bindings back: the engine
    // keeps the fact out of `SimState`, and a game that copies it in has
    // chosen to hash it. Worth knowing before building an autosave indicator
    // a replay has to agree with.
    let write = |root: &Path| {
        std::fs::create_dir_all(root.join("scripts")).expect("scripts");
        std::fs::write(root.join("project.toml"), "[game]\nname = \"Indicator\"\n")
            .expect("settings");
        std::fs::write(
            root.join("main.dim"),
            "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
             [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
             script = \"script:scripts/run.lua\"\n",
        )
        .expect("scene");
        std::fs::write(
            root.join("scripts/run.lua"),
            "function on_tick(self)\n\
             \x20 if tick.count() == 1 then app.checkpoint() end\n\
             \x20 self.saved = app.suspended()\n\
             end\n",
        )
        .expect("script");
    };

    let saving = tempfile::tempdir().expect("tempdir");
    let save = tempfile::tempdir().expect("saves");
    write(saving.path());
    let (mut project_a, mut session_a) = open(saving.path(), Some(save.path()));

    let quiet = tempfile::tempdir().expect("tempdir");
    write(quiet.path());
    let (mut project_b, mut session_b) = open(quiet.path(), None);

    for _ in 0..4 {
        session_a.step(&mut project_a, idle());
        session_b.step(&mut project_b, idle());
    }
    let _ = session_a.take_diagnostics();
    let _ = session_b.take_diagnostics();
    assert!(
        dimetric_host::suspend::probe(save.path()).ready(),
        "one of them wrote a save"
    );
    assert_ne!(
        session_a.hash(),
        session_b.hash(),
        "a script that reads `app.suspended()` into state hashes what it read, \
         which is exactly why a replay is told false"
    );
}
