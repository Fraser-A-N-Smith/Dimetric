//! Turning what the simulation asked for into sound.
//!
//! Every rule worth testing here — caps, stealing, stopping what a destroyed
//! node started — is answerable by reading a list, which is what the mock
//! backend is for. No sound card is involved anywhere in this file.

use dimetric_audio::backend::{Event, Log, Mock};
use dimetric_host::speaker::Speaker;
use dimetric_host::Project;
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
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
stream = "asset:sfx/theme"
bus = "Music"
autoplay = true
looping = true

[[node]]
id = "n_blip0000"
kind = "Sound"
name = "Blip"
parent = "n_root0000"
stream = "asset:sfx/blip"
"##;

/// Plays the blip every tick from the third, and stops the music on the tenth.
const SCRIPT: &str = r#"
local n = 0
function on_tick(self)
  n = n + 1
  if n >= 3 then self:find("Blip"):play() end
  if n == 10 then self:find("Music"):stop() end
end
"#;

/// A project on disk with two clips in it.
///
/// The bytes are not audio. Nothing decodes them here — a clip is imported
/// encoded and the backend decodes on first play — so a mock never looks.
fn project_dir(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("dimetric-speaker-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("assets/sfx")).expect("mkdir");
    std::fs::create_dir_all(root.join("scripts")).expect("mkdir");
    std::fs::write(root.join("main.dim"), SCENE).expect("scene");
    std::fs::write(root.join("scripts/room.lua"), SCRIPT).expect("script");
    std::fs::write(root.join("assets/sfx/theme.ogg"), b"OggS not really").expect("clip");
    std::fs::write(root.join("assets/sfx/blip.ogg"), b"OggS nor this").expect("clip");
    root
}

struct Harness {
    sim: Sim,
    speaker: Speaker,
    log: Log,
    root: std::path::PathBuf,
}

impl Harness {
    fn open(name: &str) -> Harness {
        let root = project_dir(name);
        let mut project = Project::open(&root, 0);
        project.import_assets();
        project
            .load_scene("main.dim")
            .unwrap_or_else(|d| panic!("{d}"));

        let (scene, mut diagnostics) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
        diagnostics.extend(project.load_scripts());
        assert!(!diagnostics.has_errors(), "{diagnostics}");

        let mut host = LuaHost::new(60).expect("lua");
        for (path, source) in &project.scripts {
            host.load(path, source)
                .unwrap_or_else(|d| panic!("{path}: {d}"));
        }

        let log: Log = Log::default();
        let speaker = Speaker::with_backend(&project, Box::new(Mock::with_log(log.clone())));
        assert!(!speaker.diagnostics.has_errors(), "{}", speaker.diagnostics);

        Harness {
            sim: Sim::new(scene, 1, Box::new(host), SimConfig::default()),
            speaker,
            log,
            root,
        }
    }

    fn run(&mut self, ticks: u64) {
        let input = InputLog::new(1, "test", 1);
        for tick in 0..ticks {
            self.sim.step(input.frame(tick));
            self.speaker.tick(&self.sim.state());
        }
    }

    fn started(&self) -> Vec<String> {
        self.log
            .borrow()
            .iter()
            .filter_map(|e| match e {
                Event::Started { params, .. } => Some(params.clip.clone()),
                _ => None,
            })
            .collect()
    }

    fn stopped(&self) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|e| matches!(e, Event::Stopped { .. }))
            .count()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_projects_clips_reach_the_backend() {
    let harness = Harness::open("clips");
    let loaded: Vec<String> = harness
        .log
        .borrow()
        .iter()
        .filter_map(|e| match e {
            Event::Loaded { clip, .. } => Some(clip.clone()),
            _ => None,
        })
        .collect();
    assert!(loaded.contains(&"sfx/theme".to_string()), "{loaded:?}");
    assert!(loaded.contains(&"sfx/blip".to_string()), "{loaded:?}");
}

#[test]
fn an_autoplaying_node_is_heard_once() {
    let mut harness = Harness::open("autoplay");
    harness.run(5);
    let theme = harness
        .started()
        .into_iter()
        .filter(|c| c == "sfx/theme")
        .count();
    assert_eq!(theme, 1, "the music started more than once");
}

#[test]
fn a_script_play_reaches_the_backend() {
    let mut harness = Harness::open("play");
    harness.run(4);
    assert!(
        harness.started().contains(&"sfx/blip".to_string()),
        "{:?}",
        harness.started()
    );
}

#[test]
fn a_script_stop_reaches_the_backend() {
    let mut harness = Harness::open("stop");
    harness.run(10);
    assert!(harness.stopped() > 0, "nothing was ever stopped");
}

#[test]
fn one_node_sounding_twice_restarts_rather_than_doubling() {
    // The blip node plays every tick from the third. Two copies of one node's
    // own sound overlapping is a bug every time, so the second start stops the
    // first.
    let mut harness = Harness::open("restart");
    harness.run(8);
    assert_eq!(
        harness.speaker.playing_clip("sfx/blip"),
        1,
        "six ticks of asking left {} blips sounding",
        harness.speaker.playing_clip("sfx/blip")
    );
    // And the music, which nothing restarted, is still going.
    assert_eq!(harness.speaker.playing_clip("sfx/theme"), 1);
}

#[test]
fn a_looping_sound_stops_when_its_node_is_destroyed() {
    // A projectile's loop would otherwise play forever, and being destroyed is
    // how a projectile normally ends.
    let mut harness = Harness::open("destroyed");
    harness.run(2);
    assert!(harness.started().contains(&"sfx/theme".to_string()));

    let music = dimetric_core::NodeUid::parse("n_music000").expect("id");
    harness.sim.shared().borrow_mut().destroy_queue.push(music);
    harness.run(2);
    assert!(harness.stopped() > 0, "the loop outlived its node");
}

#[test]
fn a_missing_clip_is_a_warning_rather_than_silence_with_no_explanation() {
    let root = project_dir("missing");
    std::fs::remove_file(root.join("assets/sfx/blip.ogg")).expect("rm");
    let mut project = Project::open(&root, 0);
    project.import_assets();
    project
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    let (scene, _) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
    let _ = project.load_scripts();

    let mut host = LuaHost::new(60).expect("lua");
    for (path, source) in &project.scripts {
        host.load(path, source)
            .unwrap_or_else(|d| panic!("{path}: {d}"));
    }
    let log: Log = Log::default();
    let mut speaker = Speaker::with_backend(&project, Box::new(Mock::with_log(log)));
    let mut sim = Sim::new(scene, 1, Box::new(host), SimConfig::default());

    let input = InputLog::new(1, "test", 1);
    for tick in 0..4 {
        sim.step(input.frame(tick));
        speaker.tick(&sim.state());
    }
    assert!(
        speaker
            .diagnostics
            .iter()
            .any(|d| d.message.contains("blip")),
        "no complaint about the clip that is not there"
    );
    let _ = std::fs::remove_dir_all(&root);
}
