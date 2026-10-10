//! What a Controls page reads, and why it is in the recording.
//!
//! A page writes a captured key's name into a label and draws its rows from the
//! bindings it reads. Labels are hashed, so both have to be reproducible — and
//! the only reproducible channel into a tick is the input frame. That is the
//! same rule `input.device()` follows, and it is the reason these are frame
//! fields rather than questions asked of the host.
//!
//! The bindings themselves stay the host's. A recording stores *actions*, so a
//! session played under any bindings replays identically under any other —
//! which is why remapping cannot be done in script. What this adds is that a
//! game may now **read** them, and then they are recorded.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::{InputFrame, InputLog, PlayerInput};
use dimetric_sim::{LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "World"

[[node]]
id = "n_page0000"
kind = "Node2D"
name = "Page"
parent = "n_root0000"
script = "script:scripts/page.lua"
"##;

fn load() -> Scene {
    let out = dimetric_scene::parse(SCENE, "c.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/page.lua", script).expect("loads");
    Sim::new(load(), 7, Box::new(host), SimConfig::default())
}

fn var(sim: &Sim, name: &str) -> String {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Page").expect("page");
    let uid = state.scene.get(id).expect("page").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get(name))
        .and_then(dimetric_scene::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A frame carrying what the host did with the input.
fn frame(captured: Option<&str>, capturing: bool, rebinds: &[(&str, &[&str])]) -> InputFrame {
    InputFrame {
        players: vec![PlayerInput::default()],
        captured: captured.map(str::to_string),
        capturing,
        rebinds: rebinds
            .iter()
            .map(|(a, keys)| {
                (
                    a.to_string(),
                    keys.iter().map(|k| k.to_string()).collect::<Vec<_>>(),
                )
            })
            .collect(),
    }
}

/// A page that writes down whatever it is told, exactly as a real one would.
const PAGE: &str = r#"
function on_ready(self)
  self.seen = ""
end

function on_tick(self)
  local got = input.captured()
  if got then self.seen = self.seen .. got .. ";" end
  if input.capturing() then self.prompt = "press a key" else self.prompt = "" end
  self.row = table.concat(input.bindings("fire"), "+")
end
"#;

#[test]
fn a_script_learns_which_key_was_pressed() {
    let mut sim = sim_with(PAGE);
    sim.step(frame(None, true, &[]));
    assert_eq!(var(&sim, "prompt"), "press a key");
    assert_eq!(var(&sim, "seen"), "", "nothing yet");

    sim.step(frame(Some("KeyF"), false, &[("fire", &["KeyF"])]));
    assert_eq!(var(&sim, "seen"), "KeyF;");
    assert_eq!(var(&sim, "prompt"), "", "the prompt comes down");
    assert_eq!(var(&sim, "row"), "KeyF", "and the row shows it");
}

#[test]
fn a_cancelled_capture_ends_without_a_name() {
    // Which is how a page tells the two apart: `capturing` stops and nothing
    // was captured.
    let mut sim = sim_with(PAGE);
    sim.step(frame(None, true, &[]));
    assert_eq!(var(&sim, "prompt"), "press a key");
    sim.step(frame(None, false, &[]));
    assert_eq!(var(&sim, "prompt"), "");
    assert_eq!(var(&sim, "seen"), "", "nothing was bound");
}

#[test]
fn a_name_is_only_there_on_the_tick_it_settled() {
    let mut sim = sim_with(PAGE);
    sim.step(frame(Some("KeyF"), false, &[]));
    assert_eq!(var(&sim, "seen"), "KeyF;");
    for _ in 0..5 {
        sim.step(frame(None, false, &[]));
    }
    assert_eq!(var(&sim, "seen"), "KeyF;", "it did not repeat");
}

#[test]
fn a_page_draws_every_row_from_what_the_host_reported() {
    let mut sim = sim_with(
        r#"
function on_tick(self)
  self.fire = table.concat(input.bindings("fire"), "+")
  self.undo = table.concat(input.bindings("undo"), "+")
  self.none = tostring(#input.bindings("alt"))
end
"#,
    );
    sim.step(frame(
        None,
        false,
        &[("fire", &["Space", "PadSouth"]), ("undo", &["KeyZ"])],
    ));
    assert_eq!(var(&sim, "fire"), "Space+PadSouth", "in the order given");
    assert_eq!(var(&sim, "undo"), "KeyZ");
    assert_eq!(var(&sim, "none"), "0", "an action nothing was said about");
}

#[test]
fn the_table_persists_until_the_host_changes_a_row() {
    let mut sim = sim_with(PAGE);
    sim.step(frame(None, false, &[("fire", &["Space"])]));
    assert_eq!(var(&sim, "row"), "Space");
    for _ in 0..5 {
        sim.step(frame(None, false, &[]));
    }
    assert_eq!(var(&sim, "row"), "Space", "nothing said, nothing changed");
    sim.step(frame(None, false, &[("fire", &["KeyF"])]));
    assert_eq!(var(&sim, "row"), "KeyF");
}

#[test]
fn a_run_nobody_told_reads_no_bindings() {
    // A headless run and a replay of a log written before this existed. The
    // honest answer is nothing, which is what those runs saw.
    let mut sim = sim_with(PAGE);
    sim.step(InputFrame::idle(1));
    assert_eq!(var(&sim, "row"), "");
    assert_eq!(var(&sim, "seen"), "");
    assert_eq!(var(&sim, "prompt"), "");
}

#[test]
fn an_idle_frame_hashes_exactly_as_it_did_before_any_of_this() {
    // The property that keeps every recorded fixture passing: the three new
    // frame fields and the bindings table contribute nothing when empty.
    let hashes = |frames: Vec<InputFrame>| {
        let mut sim = sim_with(r#"function on_tick(self) self.n = 1 end"#);
        frames
            .into_iter()
            .map(|f| {
                sim.step(f);
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hashes(vec![InputFrame::idle(1); 3]),
        hashes(vec![frame(None, false, &[]); 3]),
        "an explicitly empty frame is an idle one"
    );
    // And each of the three does move it, because each is something a page
    // would draw.
    assert_ne!(
        hashes(vec![InputFrame::idle(1)]),
        hashes(vec![frame(Some("KeyF"), false, &[])])
    );
    assert_ne!(
        hashes(vec![InputFrame::idle(1)]),
        hashes(vec![frame(None, true, &[])])
    );
    assert_ne!(
        hashes(vec![InputFrame::idle(1)]),
        hashes(vec![frame(None, false, &[("fire", &["KeyF"])])])
    );
}

#[test]
fn a_page_replays_to_the_same_hashes() {
    let script = PAGE;
    let session = || {
        vec![
            frame(None, false, &[("fire", &["Space", "Mouse0"])]),
            frame(None, true, &[]),
            frame(None, true, &[]),
            frame(Some("KeyF"), false, &[("fire", &["KeyF"])]),
            frame(None, true, &[]),
            frame(None, false, &[]),
        ]
    };
    let run = || {
        let mut sim = sim_with(script);
        session()
            .into_iter()
            .map(|f| {
                sim.step(f);
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
    let mut sim = sim_with(script);
    for f in session() {
        sim.step(f);
    }
    assert_eq!(var(&sim, "seen"), "KeyF;");
    assert_eq!(var(&sim, "row"), "KeyF");
}

#[test]
fn a_session_with_a_capture_survives_the_log() {
    let mut log = InputLog::new(7, "test", 1);
    for f in [
        frame(None, false, &[("fire", &["Space", "Mouse0"])]),
        frame(None, true, &[]),
        frame(Some("KeyF"), false, &[("fire", &["KeyF"])]),
        frame(None, false, &[]),
    ] {
        log.push(f);
    }
    let text = log.to_text();
    assert!(text.contains("bind fire Space Mouse0"), "{text}");
    assert!(text.contains("capture -"), "{text}");
    assert!(text.contains("capture KeyF"), "{text}");

    let read = InputLog::parse(&text).expect("what we wrote parses");
    assert_eq!(read.frames.len(), 4);
    assert_eq!(
        read.frame(0).rebinds,
        vec![(
            "fire".to_string(),
            vec!["Space".to_string(), "Mouse0".to_string()]
        )]
    );
    assert!(read.frame(1).capturing);
    assert_eq!(read.frame(1).captured, None);
    assert_eq!(read.frame(2).captured.as_deref(), Some("KeyF"));
    assert!(!read.frame(2).capturing);
    assert_eq!(read.frame(3), frame(None, false, &[]));
}

#[test]
fn a_log_with_no_page_in_it_is_byte_for_byte_what_it_always_was() {
    let mut log = InputLog::new(7, "test", 1);
    log.push(InputFrame::idle(1));
    let text = log.to_text();
    assert!(!text.contains("bind"), "{text}");
    assert!(!text.contains("capture"), "{text}");
}

#[test]
fn a_binding_table_survives_a_snapshot_and_a_rollback() {
    // It is state, so it has to. A rollback that forgot the bindings would
    // leave a Controls page drawing empty rows.
    let mut sim = sim_with(PAGE);
    sim.step(frame(None, false, &[("fire", &["Space"])]));
    let saved = sim.snapshot();
    sim.step(frame(None, false, &[("fire", &["KeyF"])]));
    assert_eq!(var(&sim, "row"), "KeyF");
    sim.restore(saved);
    sim.step(frame(None, false, &[]));
    assert_eq!(var(&sim, "row"), "Space", "the rollback put it back");
}
