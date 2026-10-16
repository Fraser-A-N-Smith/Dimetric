//! A binding the host changes reaches the game that changed it.
//!
//! G53 let a game emit `input.bind` and the host applied it. G58 let a page
//! capture a key. Neither told the simulation anything: `input.bindings(action)`
//! reads [`dimetric_sim::input::InputFrame::rebinds`], the drivers changed their
//! own table and never filled that field, so a Controls page could set a
//! binding and had no way to see it had worked.
//!
//! Four things were broken by the one omission, and all four are below:
//!
//! * A page could not keep the other half of a set. A capture binds the
//!   pressed key *alone*, so a page that wanted "KeyF or PadRB" had to know
//!   what the other half was — and could not read it.
//! * A key taken from another action was still shown against it.
//! * Bindings a launch restored from a saved profile never arrived.
//! * Restore-defaults changed everything and reported nothing.
//!
//! The rows are host facts that reach a label, and a label is hashed, so they
//! travel in the input frame and are in the recording — the rule `input.device`
//! and a capture's name already follow.

use std::path::Path;

use dimetric_player::bindings::{Bindings, Capture, Held};
use dimetric_player::{Session, SessionConfig};
use dimetric_sim::PlayerInput;

/// A project whose script rebinds on cue and checks the rows it reads back.
///
/// The script is the oracle, through the sandbox's own `assert`: a failed one
/// becomes a script-runtime diagnostic the session reports, which is a channel
/// the test can read without `Session` having to hand out its state. The
/// negative control below proves the channel is not vacuous.
///
/// `expect` is what `input.bindings("fire")` must read on the tick after each
/// change — the value the game sees, which is the whole of what G60 is about.
fn write_project(root: &Path, expect: &str) {
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(
        root.join("project.toml"),
        "[game]\nname = \"Controls\"\n\n[input]\nactions = [\"undo\"]\n",
    )
    .expect("settings");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/page.lua\"\n",
    )
    .expect("scene");
    std::fs::write(
        root.join("scripts/page.lua"),
        format!(
            r#"
local function row(action)
  return table.concat(input.bindings(action), "+")
end

function on_tick(self)
  local n = tick.count()
  -- A launch applying a saved profile: two keys for one action.
  if n == 1 then
    event.emit("input.bind", {{ action = "fire", keys = {{ "KeyF", "PadRB" }} }})
  end
  -- And a player putting KeyF on something else, which takes it from fire.
  if n == 4 then
    event.emit("input.bind", {{ action = "undo", keys = {{ "KeyF" }} }})
  end

  -- The defaults, reported before the first tick.
  if n == 1 then
    assert(
      row("fire") == "Space+Mouse0+PadSouth+PadRB",
      "launch: " .. row("fire")
    )
  end
  -- The saved profile. Both halves of the set, which is what a page could
  -- not see at all before: a capture binds one key, so a page that wanted
  -- two had to know the other and could not read it.
  if n == 3 then
    assert(row("fire") == "{expect}", "profile: " .. row("fire"))
  end
  -- The theft. `undo` took KeyF, so fire is down to PadRB and has to say so,
  -- or the page shows KeyF against two actions at once.
  if n == 6 then
    assert(row("undo") == "KeyF", "undo: " .. row("undo"))
    assert(row("fire") == "PadRB", "robbed: " .. row("fire"))
  end
end
"#
        ),
    )
    .expect("script");
}

fn open(root: &Path) -> (dimetric_host::Project, Session) {
    let mut project = dimetric_host::Project::open(root, 0);
    let session = Session::open(
        &mut project,
        SessionConfig {
            seed: 3,
            scene: "main".to_string(),
            record: None,
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
            suspend: None,
            date: None,
        },
    )
    .expect("the session opens");
    (project, session)
}

/// What a driver does for one frame, through the one function both use.
struct Driver {
    bindings: Bindings,
    held: Held,
    capture: Option<Capture>,
    declared: Vec<String>,
}

impl Driver {
    fn new(session: &Session) -> Driver {
        Driver {
            bindings: Bindings::wasd(),
            held: Held::new(),
            capture: None,
            declared: session.declared_actions(),
        }
    }

    fn frame(&mut self, project: &mut dimetric_host::Project, session: &mut Session) {
        session.step(project, PlayerInput::default());
        let problems = dimetric_player::bindings::settle_events(
            session,
            &mut self.bindings,
            &mut self.held,
            &mut self.capture,
            &self.declared,
        );
        assert!(problems.is_empty(), "{problems:?}");
    }
}

/// Report the whole table once, as a launch does before the first tick.
fn report_launch_table(session: &mut Session, driver: &Driver) {
    let every: Vec<_> = dimetric_player::Action::ALL
        .iter()
        .copied()
        .chain((0..driver.declared.len()).map(dimetric_player::Action::Custom))
        .collect();
    let rows = driver.bindings.rows(&every, &driver.declared);
    session.note_bindings(rows);
}

#[test]
fn a_rebind_the_host_applies_is_read_back_by_the_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "KeyF+PadRB");
    let (mut project, mut session) = open(dir.path());
    let mut driver = Driver::new(&session);
    report_launch_table(&mut session, &driver);

    for _ in 0..6 {
        driver.frame(&mut project, &mut session);
    }
    let reported = session.take_diagnostics();
    assert!(
        !reported.has_errors(),
        "the script's own checks failed, so a row did not arrive:\n{reported}"
    );
}

#[test]
fn the_scripts_checks_are_not_vacuous() {
    // The negative control. Exactly the run above with one expectation changed
    // to what the game saw *before* G60 — the row it had already, because the
    // host applied the profile and said nothing. If that still passes, the
    // test above proves nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    // Exactly what the game read before G60: the launch table, unchanged,
    // because the host applied the profile and reported nothing.
    write_project(dir.path(), "Space+Mouse0+PadSouth+PadRB");
    let (mut project, mut session) = open(dir.path());
    let mut driver = Driver::new(&session);
    report_launch_table(&mut session, &driver);

    for _ in 0..6 {
        driver.frame(&mut project, &mut session);
    }
    let reported = session.take_diagnostics();
    assert!(
        reported.has_errors(),
        "a wrong expectation has to be reported, or the oracle is silent"
    );
    assert!(
        format!("{reported}").contains("profile:"),
        "and it has to be the profile check that failed:\n{reported}"
    );
}

#[test]
fn a_rebind_is_in_the_recording() {
    // The invariant. A row reaches a label and a label is hashed, so every row
    // reported has to be in the log and replay exactly — the rule a capture's
    // name already follows. The frame is the only reproducible channel.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path(), "KeyF+PadRB");
    let log = dir.path().join("run.input");
    let mut project = dimetric_host::Project::open(dir.path(), 0);
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed: 3,
            scene: "main".to_string(),
            record: Some(log.clone()),
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
            suspend: None,
            date: None,
        },
    )
    .expect("the session opens");
    let mut driver = Driver::new(&session);
    for _ in 0..6 {
        driver.frame(&mut project, &mut session);
    }
    let written = session.finish().expect("the recording is written");
    assert!(written.is_some(), "a recording was asked for");

    let text = std::fs::read_to_string(&log).expect("a recording");
    let binds: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("bind "))
        .collect();
    assert_eq!(
        binds,
        ["bind fire KeyF PadRB", "bind undo KeyF", "bind fire PadRB"],
        "every row reported, in order, including the one the theft produced:\n{text}"
    );

    // And it parses back to the same rows, which is what makes the replay
    // exact rather than approximately right.
    let read = dimetric_sim::input::InputLog::parse(&text).expect("it parses");
    let reported: Vec<_> = read
        .frames
        .iter()
        .flat_map(|f| {
            f.rebinds
                .iter()
                .map(|(a, k)| format!("{a}={}", k.join("+")))
        })
        .collect();
    assert_eq!(reported, ["fire=KeyF+PadRB", "undo=KeyF", "fire=PadRB"]);
}

#[test]
fn a_live_capture_stays_reported_on_a_frame_with_no_events() {
    // The second drift the shared `settle_events` closed. `note_capture` has
    // to be called every tick, because `InputFrame::capturing` is *taken* on
    // each step — but the windowed driver called it only inside
    // `if !events.is_empty()`, so a page's "press a key for Act" prompt came
    // down one tick after it went up while the capture was still waiting. The
    // scripted driver called it unconditionally and behaved correctly, which
    // is the shape of disagreement that makes a thing photograph correctly and
    // play wrongly.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts");
    std::fs::write(
        dir.path().join("project.toml"),
        "[game]\nname = \"Controls\"\n",
    )
    .expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/page.lua\"\n",
    )
    .expect("scene");
    // Asks once, on the first tick, and then emits nothing at all — so every
    // later frame drains an empty list, which is the case that was broken.
    std::fs::write(
        dir.path().join("scripts/page.lua"),
        "function on_tick(self)\n\
         \x20 if tick.count() == 1 then\n\
         \x20   event.emit(\"input.capture\", { action = \"fire\" })\n\
         \x20 end\n\
         end\n",
    )
    .expect("script");

    let log = dir.path().join("run.input");
    let mut project = dimetric_host::Project::open(dir.path(), 0);
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed: 1,
            scene: "main".to_string(),
            record: Some(log.clone()),
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
            suspend: None,
            date: None,
        },
    )
    .expect("the session opens");
    let mut driver = Driver::new(&session);
    for _ in 0..5 {
        driver.frame(&mut project, &mut session);
    }
    assert!(driver.capture.is_some(), "nothing settled it");
    session.finish().expect("the recording is written");

    let text = std::fs::read_to_string(&log).expect("a recording");
    let waiting = text.lines().filter(|l| *l == "capture -").count();
    // Ticks 2, 3 and 4. Not the tick that asked: the pending field is drained
    // to build the frame *before* the script runs, so the ask lands after its
    // own frame has been written. Three is the discriminating number — with
    // `note_capture` back inside `if !events.is_empty()` only the one frame
    // that carried the event would report it.
    assert_eq!(
        waiting, 3,
        "every tick after the one that asked reports the capture still live:\n{text}"
    );
}
