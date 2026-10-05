//! The one suspended run, and the renames that keep it whole.
//!
//! `savefile` is tested next door and loads back to an identical hash. What is
//! tested here is the policy on top of it: one slot, consumed on resume, and a
//! write that cannot leave half a save behind.
//!
//! Half a save is the failure worth engineering against. A run is two files —
//! a canonical `scene.dim` and a `state.toml` — so writing them in place means
//! a crash between them leaves the new tree beside the old numbers. That state
//! never existed, and it would load without complaint.

use std::path::Path;

use dimetric_host::savefile;
use dimetric_host::suspend::{self, Slot};
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

const SCENE: &str = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
     [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
     script = \"script:main.lua\"\n";

const SCRIPT: &str = "function on_tick(self)\n\
     \x20 self.n = (self.n or 0) + rng.range(\"loot\", 1, 50)\n\
     end\n";

fn registry() -> KindRegistry {
    KindRegistry::with_builtins()
}

fn scene() -> Scene {
    let out = dimetric_scene::parse(SCENE, "s.dim", &registry());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.expect("a document").scene
}

/// A simulation `ticks` in, so the state being written is not a fresh one.
fn run(ticks: u32) -> Sim {
    let mut host = LuaHost::new(60).expect("host");
    host.load("main.lua", SCRIPT).expect("loads");
    let mut sim = Sim::new(scene(), 11, Box::new(host), SimConfig::default());
    for _ in 0..ticks {
        sim.step(InputFrame::idle(1));
    }
    sim
}

/// Every entry directly under `root`, sorted.
fn entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("readable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn an_empty_root_holds_no_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(matches!(suspend::probe(dir.path()), Slot::Empty));
    assert!(!suspend::probe(dir.path()).ready());
    // And discarding nothing is not an error: a player starting a new run does
    // not care whether one was waiting.
    suspend::discard(dir.path()).expect("discard");
}

#[test]
fn a_written_run_reads_back_to_the_same_hash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sim = run(6);
    let want = sim.hash();

    let written = suspend::write(dir.path(), &sim.state(), "floor03", &registry()).expect("write");
    assert_eq!(
        written, want,
        "write reported a hash the state does not have"
    );

    match suspend::probe(dir.path()) {
        Slot::Ready { scene, tick, seed } => {
            assert_eq!(scene, "floor03");
            assert_eq!(tick, 6);
            assert_eq!(seed, 11);
        }
        other => panic!("{other:?}"),
    }

    let (state, header, diagnostics) = suspend::read(dir.path(), &registry()).expect("read");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(state.hash(), want);
    assert_eq!(header.scene, "floor03");
}

#[test]
fn reading_leaves_the_run_where_it_is() {
    // Consuming it is the session's job, and only once the state has actually
    // been installed: a resume that failed half way should leave the run rather
    // than lose it.
    let dir = tempfile::tempdir().expect("tempdir");
    let sim = run(3);
    suspend::write(dir.path(), &sim.state(), "main", &registry()).expect("write");
    suspend::read(dir.path(), &registry()).expect("read");
    assert!(suspend::probe(dir.path()).ready());
    suspend::read(dir.path(), &registry()).expect("read again");
}

#[test]
fn a_second_write_replaces_the_first_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let early = run(2);
    suspend::write(dir.path(), &early.state(), "main", &registry()).expect("write");
    let late = run(9);
    let want = suspend::write(dir.path(), &late.state(), "main", &registry()).expect("write");

    assert_eq!(
        entries(dir.path()),
        vec![suspend::SUSPENDED_DIR.to_string()]
    );
    let (state, _, _) = suspend::read(dir.path(), &registry()).expect("read");
    assert_eq!(state.hash(), want);
    assert_eq!(state.tick.0, 9);
}

#[test]
fn an_interrupted_write_leaves_the_previous_run_rather_than_half_of_a_new_one() {
    // The window the two renames exist to make survivable. A crash between
    // moving the old save aside and moving the new one in leaves only the
    // set-aside copy, which is the previous run, intact — so that is what is
    // continued rather than nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    let before = run(4);
    let want = suspend::write(dir.path(), &before.state(), "main", &registry()).expect("write");

    // Exactly the state the process would be in, reproduced by hand.
    std::fs::rename(
        suspend::suspended_dir(dir.path()),
        dir.path().join("suspended.previous"),
    )
    .expect("set aside");
    assert!(!suspend::suspended_dir(dir.path()).exists());

    assert!(
        suspend::probe(dir.path()).ready(),
        "the previous run was lost"
    );
    let (state, _, _) = suspend::read(dir.path(), &registry()).expect("read");
    assert_eq!(state.hash(), want);
    assert_eq!(state.tick.0, 4, "a different run came back");
}

#[test]
fn strays_from_an_interrupted_write_are_cleaned_up_by_the_next_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("suspended.pending")).expect("stray");
    std::fs::write(
        dir.path().join("suspended.pending/state.toml"),
        "nonsense = true\n",
    )
    .expect("stray");

    let sim = run(1);
    suspend::write(dir.path(), &sim.state(), "main", &registry()).expect("write");
    assert_eq!(
        entries(dir.path()),
        vec![suspend::SUSPENDED_DIR.to_string()]
    );
}

#[test]
fn discarding_removes_the_run_and_anything_left_beside_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sim = run(2);
    suspend::write(dir.path(), &sim.state(), "main", &registry()).expect("write");
    std::fs::create_dir_all(dir.path().join("suspended.previous")).expect("stray");

    suspend::discard(dir.path()).expect("discard");
    assert!(entries(dir.path()).is_empty(), "{:?}", entries(dir.path()));
    assert!(matches!(suspend::probe(dir.path()), Slot::Empty));
}

#[test]
fn a_run_from_another_engine_reads_as_no_run_at_all() {
    // Not as a run that fails when continued. A Continue row that errors when
    // pressed is worse than one that was never shown, so the slot answers
    // "nothing" and says why separately.
    let dir = tempfile::tempdir().expect("tempdir");
    let sim = run(5);
    suspend::write(dir.path(), &sim.state(), "main", &registry()).expect("write");

    let path = suspend::suspended_dir(dir.path()).join(savefile::STATE_FILE);
    let text = std::fs::read_to_string(&path).expect("state");
    std::fs::write(
        &path,
        text.replace(
            &format!("engine = \"{}\"", env!("CARGO_PKG_VERSION")),
            "engine = \"0.0.1-ancient\"",
        ),
    )
    .expect("rewrite");

    match suspend::probe(dir.path()) {
        Slot::Stale(d) => {
            assert_eq!(d.code, dimetric_core::Code::SAVE_VERSION);
            assert!(d.message.contains("0.0.1-ancient"), "{}", d.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(!suspend::probe(dir.path()).ready());
}

#[test]
fn a_directory_that_is_not_a_save_reads_as_stale_rather_than_empty() {
    // Empty would be a lie that loses a player's run silently. Something is
    // there; this build cannot read it, and that is worth saying.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(suspend::suspended_dir(dir.path())).expect("dir");
    std::fs::write(
        suspend::suspended_dir(dir.path()).join(savefile::STATE_FILE),
        "this is not toml ][\n",
    )
    .expect("junk");

    match suspend::probe(dir.path()) {
        Slot::Stale(d) => assert_eq!(d.code, dimetric_core::Code::SAVE_UNREADABLE),
        other => panic!("{other:?}"),
    }
}

#[test]
fn resuming_what_was_never_written_says_what_is_wrong() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = suspend::read(dir.path(), &registry()).expect_err("there is nothing there");
    assert_eq!(err.code, dimetric_core::Code::SUSPEND_REFUSED);
}
