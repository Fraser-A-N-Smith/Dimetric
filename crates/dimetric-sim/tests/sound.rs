//! What the simulation asks to be heard, and what that must not cost.
//!
//! M6's acceptance criterion is that a headless run with audio triggered
//! produces the same state hash as a windowed one. That is a statement about
//! where the line is: a sound is presentation, so triggering one cannot consume
//! a random number or write anything that gets hashed. If it did, muting a game
//! would change how it plays.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::sound::SoundEvent;
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

const ROOM: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"
script = "script:scripts/room.lua"

[[node]]
id = "n_music000"
kind = "Sound"
name = "Music"
parent = "n_root0000"
stream = "asset:music/theme"
bus = "Music"
autoplay = true
looping = true
volume_db = -6.0

[[node]]
id = "n_blip0000"
kind = "Sound"
name = "Blip"
parent = "n_root0000"
stream = "asset:sfx/blip"
pitch_variation = 0.25
"##;

fn scene_of(text: &str) -> Scene {
    let out = dimetric_scene::parse(text, "test.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim_with(script: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/room.lua", script).expect("script loads");
    Sim::new(scene_of(ROOM), 9, Box::new(host), SimConfig::default())
}

fn run(sim: &mut Sim, ticks: u64) {
    let log = InputLog::new(9, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
}

const SILENT: &str = "function on_tick(self) end\n";

/// Plays the blip on tick 3 and stops the music on tick 5.
const NOISY: &str = r#"
local n = 0
function on_tick(self)
  n = n + 1
  if n == 3 then self:find("Blip"):play() end
  if n == 5 then self:find("Music"):stop() end
end
"#;

#[test]
fn a_sound_node_starts_itself_when_it_is_told_to() {
    let mut sim = sim_with(SILENT);
    run(&mut sim, 1);
    let sounds = &sim.state().sounds;
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    let SoundEvent::Play(cue) = &sounds[0] else {
        panic!("{sounds:?}")
    };
    assert_eq!(cue.stream, "music/theme");
    assert_eq!(cue.bus, "Music");
    assert!(cue.looping);
    assert_eq!(cue.volume_db.to_string(), "-6.0");
}

#[test]
fn autoplay_happens_once_and_not_every_tick() {
    let mut sim = sim_with(SILENT);
    run(&mut sim, 10);
    assert!(
        sim.state().sounds.is_empty(),
        "still asking: {:?}",
        sim.state().sounds
    );
}

#[test]
fn the_list_holds_this_ticks_sounds_and_not_the_last_ones() {
    // Cleared at the start of a tick rather than the end, so that when `step`
    // returns whoever is listening can read what the tick asked for.
    let mut sim = sim_with(NOISY);
    run(&mut sim, 3);
    assert_eq!(sim.state().sounds.len(), 1, "the blip");
    run(&mut sim, 1);
    assert!(sim.state().sounds.is_empty(), "the blip again");
}

#[test]
fn a_script_can_start_and_stop_a_sound() {
    let mut sim = sim_with(NOISY);
    run(&mut sim, 3);
    let played = sim.state().sounds.clone();
    match &played[..] {
        [SoundEvent::Play(cue)] => assert_eq!(cue.stream, "sfx/blip"),
        other => panic!("{other:?}"),
    }

    run(&mut sim, 2);
    let stopped = sim.state().sounds.clone();
    match &stopped[..] {
        [SoundEvent::Stop { node }] => assert_eq!(node.to_text(), "n_music000"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn playing_something_that_is_not_a_sound_is_an_error_rather_than_silence() {
    let mut sim = sim_with("function on_tick(self) self:play() end\n");
    run(&mut sim, 1);
    let diagnostics = sim.take_diagnostics();
    assert!(diagnostics.has_errors(), "no complaint at all");
}

// -- the line ------------------------------------------------------------

#[test]
fn triggering_audio_does_not_change_the_state_hash() {
    // The criterion itself. Two runs of the same scene and the same script:
    // one where the sound bookkeeping accumulates exactly as it would with a
    // device attached, one where it is wiped after every tick as though nothing
    // ever asked for a sound. The hashes have to agree tick for tick.
    let mut heard = sim_with(NOISY);
    let mut silent = sim_with(NOISY);
    let log = InputLog::new(9, "test", 1);

    let mut asked = 0;
    for tick in 0..30 {
        heard.step(log.frame(tick));
        asked += heard.state().sounds.len();

        silent.step(log.frame(tick));
        {
            let mut state = silent.shared().borrow_mut();
            state.sounds.clear();
            state.autoplayed.clear();
        }

        assert_eq!(
            heard.hash(),
            silent.hash(),
            "audio moved the state at tick {tick}"
        );
    }
    assert!(asked >= 3, "the test heard nothing, so it proved nothing");
}

#[test]
fn a_sound_does_not_consume_the_simulations_randomness() {
    // The other half of the same rule, checked directly: pitch variation is
    // drawn on the presentation side, so a run that plays sounds must leave the
    // simulation's streams exactly where a run that does not leaves them.
    let mut noisy = sim_with(NOISY);
    let mut quiet = sim_with(SILENT);
    run(&mut noisy, 20);
    run(&mut quiet, 20);
    // Neither script draws a random number, so both runs must leave the
    // streams untouched — and the hash covers them, so comparing the hashes of
    // two runs that differ only in the sounds they asked for says it.
    assert_eq!(noisy.state().rng.seed(), quiet.state().rng.seed());
    assert_eq!(noisy.hash(), quiet.hash(), "playing sounds moved the state");
}

#[test]
fn a_rollback_before_a_sound_started_lets_it_start_again() {
    // `autoplayed` is not hashed, but it is snapshotted: a rollback to before
    // the music began has to let it begin, or a replay of that stretch would be
    // silent.
    let mut sim = sim_with(SILENT);
    let before = sim.snapshot();
    run(&mut sim, 5);
    assert!(sim.state().sounds.is_empty());

    sim.restore(before);
    run(&mut sim, 1);
    assert_eq!(sim.state().sounds.len(), 1, "the music did not start again");
}
