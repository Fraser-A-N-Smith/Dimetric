//! `app.today()` and `input.device()`: two inputs the host has and a tick
//! cannot get for itself.
//!
//! Both end up in what a script writes into the scene — the date in a menu's
//! label and in the seed it picks, the device in every prompt's text — and the
//! scene is hashed. So both have to be reproducible, and both are: the date
//! travels in the log's header and the device in its frames.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::{Device, InputFrame, InputLog, PlayerInput, FIXED_DATE};
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
id = "n_title000"
kind = "Node2D"
name = "Title"
parent = "n_root0000"
script = "script:scripts/title.lua"
"##;

fn load() -> Scene {
    let out = dimetric_scene::parse(SCENE, "t.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_on(date: Option<&str>, script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    if let Some(date) = date {
        host.set_date(date);
    }
    host.load("scripts/title.lua", script).expect("loads");
    Sim::new(load(), 7, Box::new(host), SimConfig::default())
}

fn var(sim: &Sim, name: &str) -> String {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Title").expect("title");
    let uid = state.scene.get(id).expect("title").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get(name))
        .and_then(dimetric_scene::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn frame(device: Device) -> InputFrame {
    InputFrame {
        players: vec![PlayerInput {
            device,
            ..PlayerInput::default()
        }],
        ..InputFrame::default()
    }
}

const DAILY: &str = r#"
function on_tick(self)
  self.day = app.today()
end
"#;

#[test]
fn a_run_nobody_told_the_date_gets_a_fixed_one() {
    // Not today's. A headless run is usually a fixture, and a fixture whose
    // output moved with the calendar could not be checked twice.
    let mut sim = sim_on(None, DAILY);
    sim.step(InputFrame::idle(1));
    assert_eq!(var(&sim, "day"), FIXED_DATE);
}

#[test]
fn a_script_reads_the_day_the_host_was_told() {
    let mut sim = sim_on(Some("2026-10-10"), DAILY);
    sim.step(InputFrame::idle(1));
    assert_eq!(var(&sim, "day"), "2026-10-10");
}

#[test]
fn the_date_does_not_change_under_a_running_session() {
    // It is a date, not a clock. A tick asking twice gets the same answer, and
    // so does the same tick replayed next week.
    let mut sim = sim_on(Some("2026-10-10"), DAILY);
    for _ in 0..200 {
        sim.step(InputFrame::idle(1));
        assert_eq!(var(&sim, "day"), "2026-10-10");
    }
}

#[test]
fn a_daily_seed_is_the_same_for_two_players_on_one_day() {
    // The whole point. Two sessions, one date, one seeded stream — and the
    // session seeds differ, which is what `rng.seed` is for.
    let script = r#"
function on_ready(self)
  local day = app.today()
  local n = 0
  for i = 1, #day do n = n * 31 + string.byte(day, i) end
  rng.seed("daily", n)
  self.first = rng.range("daily", 0, 1000000)
end
"#;
    let roll = |seed: u64, date: &str| {
        let mut host = LuaHost::new(60).expect("lua host");
        host.set_date(date);
        host.load("scripts/title.lua", script).expect("loads");
        let mut sim = Sim::new(load(), seed, Box::new(host), SimConfig::default());
        sim.step(InputFrame::idle(1));
        let state = sim.state();
        let id = state.scene.resolve_path("/World/Title").expect("title");
        let uid = state.scene.get(id).expect("title").uid;
        state
            .vars
            .get(&uid)
            .and_then(|v| v.get("first"))
            .and_then(dimetric_scene::Value::as_int)
            .expect("a roll")
    };
    assert_eq!(roll(7, "2026-10-10"), roll(999_999, "2026-10-10"));
    assert_ne!(roll(7, "2026-10-10"), roll(7, "2026-10-11"));
}

#[test]
fn the_date_a_recording_was_told_travels_with_it() {
    let log = InputLog::new(7, "test", 1).with_date("2026-10-10");
    let text = log.to_text();
    assert!(text.contains("date 2026-10-10"), "{text}");
    let read = InputLog::parse(&text).expect("what we wrote parses");
    assert_eq!(read.date.as_deref(), Some("2026-10-10"));
    assert_eq!(read.date_or_fixed(), "2026-10-10");
}

#[test]
fn a_log_with_no_date_is_byte_for_byte_what_it_always_was() {
    let text = InputLog::new(7, "test", 1).to_text();
    assert!(!text.contains("date"), "{text}");
    assert_eq!(
        InputLog::parse(&text).expect("parses").date_or_fixed(),
        FIXED_DATE,
        "and it replays as what it was told at the time"
    );
}

#[test]
fn a_script_reads_which_device_last_moved() {
    let mut sim = sim_on(
        None,
        r#"
function on_tick(self)
  self.device = input.device()
end
"#,
    );
    for device in [
        Device::Keyboard,
        Device::Pad,
        Device::Mouse,
        Device::Keyboard,
    ] {
        sim.step(frame(device));
        assert_eq!(var(&sim, "device"), device.name());
    }
}

#[test]
fn a_keyboard_frame_hashes_exactly_as_it_did_before_devices_existed() {
    // The property that keeps every recorded fixture passing. A value equal to
    // the default contributes nothing to the hash, and the default is what
    // every log written until now means.
    let hashes = |device: Device| {
        let mut sim = sim_on(None, r#"function on_tick(self) self.n = 1 end"#);
        sim.step(frame(device));
        sim.hash()
    };
    assert_eq!(
        hashes(Device::Keyboard),
        hashes(Device::default()),
        "keyboard is the default"
    );
    // And a pad is a different run, because the prompts a game draws differ.
    assert_ne!(hashes(Device::Keyboard), hashes(Device::Pad));
    assert_ne!(hashes(Device::Pad), hashes(Device::Mouse));
}

#[test]
fn a_device_in_a_frame_survives_the_log() {
    let mut log = InputLog::new(7, "test", 1);
    log.push(frame(Device::Pad));
    log.push(frame(Device::Keyboard));
    log.push(frame(Device::Mouse));
    let text = log.to_text();
    assert!(text.contains(":pad"), "{text}");
    assert!(text.contains(":mouse"), "{text}");
    let read = InputLog::parse(&text).expect("parses");
    assert_eq!(read.frame(0).player(0).device, Device::Pad);
    assert_eq!(read.frame(1).player(0).device, Device::Keyboard);
    assert_eq!(read.frame(2).player(0).device, Device::Mouse);
}

#[test]
fn a_two_player_frame_keeps_each_players_device_to_itself() {
    // The reason the column carries a colon. Columns are positional and
    // interleaved per player, so a bare optional field would be read as the
    // next player's buttons.
    let mut log = InputLog::new(7, "test", 2);
    log.push(InputFrame {
        players: vec![
            PlayerInput {
                buttons: 3,
                device: Device::Pad,
                ..PlayerInput::default()
            },
            PlayerInput {
                buttons: 5,
                device: Device::Keyboard,
                ..PlayerInput::default()
            },
        ],
        ..InputFrame::default()
    });
    log.push(InputFrame {
        players: vec![
            PlayerInput {
                buttons: 7,
                device: Device::Keyboard,
                ..PlayerInput::default()
            },
            PlayerInput {
                buttons: 9,
                device: Device::Mouse,
                ..PlayerInput::default()
            },
        ],
        ..InputFrame::default()
    });
    let read = InputLog::parse(&log.to_text()).expect("parses");
    assert_eq!(read.frame(0).player(0).buttons, 3);
    assert_eq!(read.frame(0).player(0).device, Device::Pad);
    assert_eq!(read.frame(0).player(1).buttons, 5);
    assert_eq!(read.frame(0).player(1).device, Device::Keyboard);
    assert_eq!(read.frame(1).player(0).buttons, 7);
    assert_eq!(read.frame(1).player(0).device, Device::Keyboard);
    assert_eq!(read.frame(1).player(1).buttons, 9);
    assert_eq!(read.frame(1).player(1).device, Device::Mouse);
}

#[test]
fn a_run_that_switches_device_replays_to_the_same_hashes() {
    let script = r#"
function on_ready(self)
  self.seen = ""
end

function on_tick(self)
  self.seen = self.seen .. string.sub(input.device(), 1, 1)
end
"#;
    let devices = [
        Device::Keyboard,
        Device::Pad,
        Device::Pad,
        Device::Mouse,
        Device::Keyboard,
    ];
    let run = || {
        let mut sim = sim_on(Some("2026-10-10"), script);
        devices
            .iter()
            .map(|d| {
                sim.step(frame(*d));
                sim.hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
    let mut sim = sim_on(Some("2026-10-10"), script);
    for d in devices {
        sim.step(frame(d));
    }
    assert_eq!(
        var(&sim, "seen"),
        "kppmk",
        "the prompts a game drew, in order"
    );
}
