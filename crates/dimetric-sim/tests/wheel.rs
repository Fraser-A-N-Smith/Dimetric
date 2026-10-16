//! A script can read the mouse wheel.
//!
//! `PlayerInput` carried buttons, a stick, an aim and a pointer, and nothing of
//! the wheel. `Device::Mouse` is documented as "a move, a button or a wheel",
//! so a turn of it marked the device and was then thrown away — the one piece
//! of mouse input with nowhere to go. A zoom control, a selected-unit cycle and
//! a scrollable list all want it.
//!
//! It is in the frame rather than beside it for the same reason the device is:
//! a wheel moves what a label shows, and a label is hashed. And it is whole
//! *notches* rather than a platform delta, because a mouse reports lines, a
//! trackpad reports pixels and both report floats — passing that through would
//! put a host's scroll resolution into the simulation and break I3 on the way
//! in. The host accumulates; an integer crosses.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::{Device, InputFrame, InputLog, PlayerInput};
use dimetric_sim::{LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "World"
script = "script:scripts/zoom.lua"
"##;

fn load() -> Scene {
    let out = dimetric_scene::parse(SCENE, "w.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load("scripts/zoom.lua", script).expect("loads");
    Sim::new(load(), 1, Box::new(host), SimConfig::default())
}

fn var(sim: &Sim, name: &str) -> String {
    let state = sim.state();
    let id = state.scene.resolve_path("/World").expect("root");
    let uid = state.scene.get(id).expect("root").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get(name))
        .map(|v| match v {
            dimetric_scene::Value::Str(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .unwrap_or_default()
}

fn turned(notches: i32) -> InputFrame {
    InputFrame {
        players: vec![PlayerInput {
            wheel: notches,
            device: match notches {
                0 => Device::Keyboard,
                _ => Device::Mouse,
            },
            ..PlayerInput::default()
        }],
        ..InputFrame::default()
    }
}

#[test]
fn a_script_reads_the_notches_the_wheel_turned() {
    let mut sim = sim_with(
        "function on_tick(self)\n\
         \x20 self.seen = tostring(input.wheel())\n\
         end\n",
    );
    sim.step(turned(0));
    assert_eq!(var(&sim, "seen"), "0", "a still wheel reads zero, not nil");
    sim.step(turned(3));
    assert_eq!(var(&sim, "seen"), "3");
    sim.step(turned(-2));
    assert_eq!(var(&sim, "seen"), "-2", "positive is away from the player");
    sim.step(turned(0));
    assert_eq!(var(&sim, "seen"), "0", "and it does not linger");
    let reported = sim.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
}

#[test]
fn a_still_wheel_hashes_exactly_as_it_did_before_the_wheel_existed() {
    // The property that keeps every recorded fixture passing: a value equal to
    // the default contributes nothing, and a still wheel is what every frame
    // in every log written until now meant.
    let hashes = |wheel: i32| {
        let mut sim = sim_with("function on_tick(self) self.n = 1 end");
        sim.step(InputFrame {
            players: vec![PlayerInput {
                wheel,
                ..PlayerInput::default()
            }],
            ..InputFrame::default()
        });
        sim.hash()
    };
    assert_eq!(hashes(0), hashes(i32::default()));
    // And a turn is a different run, because what a game draws from it differs.
    assert_ne!(hashes(0), hashes(1));
    assert_ne!(hashes(1), hashes(-1), "the sign is part of it");
    assert_ne!(hashes(1), hashes(2));
}

#[test]
fn a_wheel_in_a_frame_survives_the_log() {
    let mut log = InputLog::new(7, "test", 1);
    log.push(turned(0));
    log.push(turned(1));
    log.push(turned(-4));
    let text = log.to_text();
    assert!(text.contains(" w1"), "{text}");
    assert!(text.contains(" w-4"), "{text}");
    let read = InputLog::parse(&text).expect("parses");
    assert_eq!(read.frame(0).player(0).wheel, 0);
    assert_eq!(read.frame(1).player(0).wheel, 1);
    assert_eq!(read.frame(2).player(0).wheel, -4);
}

#[test]
fn a_log_with_no_wheel_is_byte_for_byte_what_it_always_was() {
    // Omitted when zero, exactly as the device is. Nothing recorded before the
    // wheel existed may gain a column.
    let mut log = InputLog::new(7, "test", 1);
    log.push(InputFrame::idle(1));
    log.push(InputFrame::idle(1));
    let text = log.to_text();
    assert!(
        !text.contains(" w"),
        "an idle run wrote a wheel column:\n{text}"
    );
}

#[test]
fn a_two_player_frame_keeps_each_players_wheel_to_itself() {
    // The reason the column carries a sigil. Columns are positional and
    // interleaved per player, so a bare optional number would be read as the
    // next player's buttons — and `w` is not a hex digit, so it can never be
    // mistaken for one.
    let mut log = InputLog::new(7, "test", 2);
    log.push(InputFrame {
        players: vec![
            PlayerInput {
                buttons: 3,
                wheel: 2,
                ..PlayerInput::default()
            },
            PlayerInput {
                buttons: 5,
                ..PlayerInput::default()
            },
        ],
        ..InputFrame::default()
    });
    // And with both optional columns at once, in the order the writer emits.
    log.push(InputFrame {
        players: vec![
            PlayerInput {
                buttons: 7,
                device: Device::Mouse,
                wheel: -1,
                ..PlayerInput::default()
            },
            PlayerInput {
                buttons: 9,
                device: Device::Pad,
                ..PlayerInput::default()
            },
        ],
        ..InputFrame::default()
    });
    let read = InputLog::parse(&log.to_text()).expect("parses");
    assert_eq!(read.frame(0).player(0).buttons, 3);
    assert_eq!(read.frame(0).player(0).wheel, 2);
    assert_eq!(read.frame(0).player(1).buttons, 5);
    assert_eq!(read.frame(0).player(1).wheel, 0);
    assert_eq!(read.frame(1).player(0).buttons, 7);
    assert_eq!(read.frame(1).player(0).device, Device::Mouse);
    assert_eq!(read.frame(1).player(0).wheel, -1);
    assert_eq!(read.frame(1).player(1).buttons, 9);
    assert_eq!(read.frame(1).player(1).device, Device::Pad);
    assert_eq!(read.frame(1).player(1).wheel, 0);
}

#[test]
fn a_run_that_scrolls_replays_to_the_same_hashes() {
    // The invariant. The wheel moves a label, so it has to be in the recording
    // and come back out of it identically.
    let script = "function on_tick(self)\n\
                  \x20 self.zoom = (self.zoom or 0) + input.wheel()\n\
                  \x20 self.label = tostring(self.zoom)\n\
                  end\n";
    let turns = [0, 2, 0, -1, 3, 0];

    let mut live = sim_with(script);
    let mut log = InputLog::new(7, "test", 1);
    let mut expected = Vec::new();
    for notches in turns {
        let frame = turned(notches);
        log.push(frame.clone());
        live.step(frame);
        expected.push(live.hash());
    }

    let read = InputLog::parse(&log.to_text()).expect("parses");
    let mut replayed = sim_with(script);
    let mut actual = Vec::new();
    for index in 0..turns.len() as u64 {
        replayed.step(read.frame(index).clone());
        actual.push(replayed.hash());
    }
    assert_eq!(expected, actual);
    assert_eq!(var(&replayed, "label"), "4", "0+2+0-1+3+0");
}
