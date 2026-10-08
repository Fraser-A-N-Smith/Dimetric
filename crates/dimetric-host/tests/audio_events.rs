//! A game setting its own volume.
//!
//! `docs/API.md` has said since M12 that a volume change reaches the mixer
//! through `event.emit`, and the mixer has had `set_bus_gain` for as long. The
//! two were never joined: `dim-play` never drained the event list, so a game's
//! Options screen emitted three events per launch and nothing read them.
//!
//! The alternative open to a game was writing `volume_db` onto every `Sound`
//! node in every scene — forty-odd nodes, rewritten on every scene load, and
//! still unable to reach a `continuous` music voice carried over from the
//! previous floor. That is reimplementing a mixer bus in script.
//!
//! None of this is simulation state. A volume is a fact about a device, the
//! game owns the setting in its profile, and the channel is one-way.

use dimetric_audio::{percent_to_gain, Bus};
use dimetric_core::Tick;
use dimetric_host::speaker::{Speaker, BUS_VOLUME};
use dimetric_host::Project;
use dimetric_scene::Value;
use dimetric_sim::event::GameEvent;
use indexmap::IndexMap;

/// A project with nothing in it, which is all a speaker needs.
fn project(dir: &std::path::Path) -> Project {
    std::fs::write(
        dir.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    Project::open(dir, 0)
}

/// A speaker on the mock backend, and the list it writes to.
fn speaker(project: &Project) -> (Speaker, dimetric_audio::backend::Log) {
    let mock = dimetric_audio::backend::Mock::new();
    let log = mock.log();
    (Speaker::with_backend(project, Box::new(mock)), log)
}

/// An `audio.bus_volume` event, as a game emits one.
fn volume(bus: &str, percent: i64) -> GameEvent {
    let mut payload = IndexMap::new();
    payload.insert("bus".to_string(), Value::Str(bus.to_string()));
    payload.insert("percent".to_string(), Value::Int(percent));
    GameEvent {
        tick: Tick(0),
        kind: BUS_VOLUME.to_string(),
        payload: Value::Map(payload),
    }
}

/// An event with a payload that is not a table of bus and percent.
fn malformed(payload: Value) -> GameEvent {
    GameEvent {
        tick: Tick(0),
        kind: BUS_VOLUME.to_string(),
        payload,
    }
}

#[test]
fn a_bus_volume_event_reaches_the_bus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());
    let (mut speaker, _log) = speaker(&project);

    assert_eq!(speaker.bus_gain(Bus::Music), 1.0, "a bus starts at full");
    assert!(speaker.apply_event(&volume("Music", 80)));
    assert_eq!(speaker.bus_gain(Bus::Music), percent_to_gain(80));
    assert!(!speaker.diagnostics.has_errors(), "{}", speaker.diagnostics);

    // And the other two, independently: turning the music down must not touch
    // the effects.
    assert_eq!(speaker.bus_gain(Bus::Sfx), 1.0);
    assert!(speaker.apply_event(&volume("Ui", 40)));
    assert_eq!(speaker.bus_gain(Bus::Ui), percent_to_gain(40));
    assert_eq!(speaker.bus_gain(Bus::Music), percent_to_gain(80));
}

#[test]
fn off_is_silent() {
    // The one value a curve must get exactly right. A logarithmic scale has no
    // bottom, so "off" is a case of its own rather than something approached.
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());
    let (mut speaker, _log) = speaker(&project);
    assert!(speaker.apply_event(&volume("Music", 0)));
    assert_eq!(speaker.bus_gain(Bus::Music), 0.0);
    assert!(!speaker.diagnostics.has_errors());

    // And back again, which is the other half of a player changing their mind.
    assert!(speaker.apply_event(&volume("Music", 100)));
    assert_eq!(speaker.bus_gain(Bus::Music), 1.0);
}

#[test]
fn the_backend_hears_about_it_too() {
    // The pool's gain decides what a *new* voice is worth; the backend's decides
    // what the ones already sounding are. A music voice carried over from the
    // previous floor is the whole reason the second one matters — writing
    // `volume_db` onto a scene's `Sound` nodes could never have reached it.
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());
    let (mut speaker, log) = speaker(&project);
    speaker.apply_event(&volume("Music", 0));

    let told: Vec<(Bus, f32)> = log
        .borrow()
        .iter()
        .filter_map(|e| match e {
            dimetric_audio::backend::Event::BusGain { bus, gain, .. } => Some((*bus, *gain)),
            _ => None,
        })
        .collect();
    assert_eq!(told, vec![(Bus::Music, 0.0)], "the backend was never told");

    // And over a fade rather than in one sample, because a gain moved
    // instantly is a click.
    let faded = log.borrow().iter().any(|e| {
        matches!(
            e,
            dimetric_audio::backend::Event::BusGain { seconds, .. } if *seconds > 0.0
        )
    });
    assert!(faded, "the change was instant");
}

#[test]
fn the_steps_a_stepped_slider_takes_are_evenly_spaced() {
    // Why decibels and not amplitude. A linear gain makes halfway up already
    // most of the way loud; these are five steps of eight decibels each, which
    // is five equal changes to a listener.
    let gains: Vec<f32> = (0..=5).map(|step| percent_to_gain(step * 20)).collect();
    assert_eq!(gains[0], 0.0);
    assert_eq!(gains[5], 1.0);
    for pair in gains[1..].windows(2) {
        let ratio = pair[1] / pair[0];
        assert!(
            (ratio - 2.511).abs() < 0.01,
            "{pair:?} is not one step: {ratio}"
        );
    }
}

#[test]
fn a_percent_outside_the_range_is_clamped_rather_than_refused() {
    // A game that computed 120 meant loud. A volume control is not a gain
    // stage, so it stops at full rather than amplifying.
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());
    let (mut speaker, _log) = speaker(&project);
    assert!(speaker.apply_event(&volume("Sfx", 120)));
    assert_eq!(speaker.bus_gain(Bus::Sfx), 1.0);
    assert!(speaker.apply_event(&volume("Sfx", -5)));
    assert_eq!(speaker.bus_gain(Bus::Sfx), 0.0);
    assert!(!speaker.diagnostics.has_errors(), "{}", speaker.diagnostics);
}

#[test]
fn an_event_of_another_kind_is_left_alone() {
    // The channel is free text so a game can tell its own host about its own
    // achievements. An engine that refused or logged what it did not recognise
    // would make the channel useless for everything else.
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());
    let (mut speaker, _log) = speaker(&project);
    let theirs = GameEvent {
        tick: Tick(3),
        kind: "steam.achievement".to_string(),
        payload: Value::Str("first_blood".to_string()),
    };
    assert!(!speaker.apply_event(&theirs), "the engine claimed it");
    assert!(speaker.diagnostics.0.is_empty(), "{}", speaker.diagnostics);
    assert_eq!(speaker.bus_gain(Bus::Music), 1.0);
}

#[test]
fn a_payload_this_build_cannot_read_is_reported() {
    // A kind the engine *does* claim, with a payload it cannot use, is a
    // diagnostic: the game asked for something and did not get it. The bus is
    // left where it was rather than guessed at.
    let dir = tempfile::tempdir().expect("tempdir");
    let project = project(dir.path());

    let cases: Vec<GameEvent> = vec![
        malformed(Value::Str("Music".to_string())),
        malformed(Value::Map(IndexMap::new())),
        volume("Master", 50),
        {
            let mut payload = IndexMap::new();
            payload.insert("bus".to_string(), Value::Str("Music".to_string()));
            payload.insert("percent".to_string(), Value::Str("80".to_string()));
            malformed(Value::Map(payload))
        },
    ];
    for event in cases {
        let (mut speaker, _log) = speaker(&project);
        assert!(speaker.apply_event(&event), "{event:?} was not claimed");
        assert_eq!(
            speaker.diagnostics.0.len(),
            1,
            "{event:?}: {}",
            speaker.diagnostics
        );
        assert_eq!(
            speaker.diagnostics.0[0].code,
            dimetric_core::Code::AUDIO_EVENT_BAD
        );
        assert_eq!(
            speaker.bus_gain(Bus::Music),
            1.0,
            "{event:?} moved a bus anyway"
        );
    }
}
