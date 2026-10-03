//! Music that survives a scene load.
//!
//! A floor change is `scene.request_load`, which swaps the whole tree, and the
//! speaker stops voices whose node has left it — correctly, because a
//! projectile's loop must not outlive the projectile. So a region theme
//! restarted from bar one on every floor, and "music continues across a level
//! transition" is the default expectation of every game with levels.
//!
//! A voice from a `continuous` node is keyed by `(stream, bus)` instead, so the
//! next scene's own copy of the same track finds it already sounding and
//! continues it.

use dimetric_audio::backend::{Event, Log, Mock};
use dimetric_host::speaker::Speaker;
use dimetric_host::Project;
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

/// A floor with a theme and a one-shot, both on their own buses.
fn floor(name: &str, theme: Option<&str>) -> String {
    let mut out = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"{name}\"\n\n\
         [[node]]\nid = \"n_loop0000\"\nkind = \"Sound\"\nname = \"Loop\"\n\
         parent = \"n_root0000\"\nstream = \"asset:sfx/loop\"\n\
         autoplay = true\nlooping = true\n"
    );
    if let Some(stream) = theme {
        // A *different* node id on each floor, deliberately: the point is that
        // the voice is not keyed by the node.
        let id = match name {
            "Floor1" => "n_music001",
            _ => "n_music002",
        };
        out.push_str(&format!(
            "\n[[node]]\nid = \"{id}\"\nkind = \"Sound\"\nname = \"Theme\"\n\
             parent = \"n_root0000\"\nstream = \"asset:sfx/{stream}\"\n\
             bus = \"Music\"\nautoplay = true\nlooping = true\ncontinuous = true\n"
        ));
    }
    out
}

fn project_dir(name: &str, second: Option<&str>) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("dimetric-continuous-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("assets/sfx")).expect("mkdir");
    std::fs::write(root.join("main.dim"), floor("Floor1", Some("theme"))).expect("scene");
    std::fs::write(root.join("floor2.dim"), floor("Floor2", second)).expect("scene");
    for clip in ["theme", "other", "loop"] {
        std::fs::write(
            root.join(format!("assets/sfx/{clip}.ogg")),
            b"OggS not really",
        )
        .expect("clip");
    }
    root
}

struct Harness {
    project: Project,
    sim: Sim,
    speaker: Speaker,
    log: Log,
}

impl Harness {
    /// Open `main.dim`, with `floor2.dim` carrying `second` as its theme.
    fn open(name: &str, second: Option<&str>) -> Harness {
        let root = project_dir(name, second);
        let mut project = Project::open(&root, 0);
        project.import_assets();
        project
            .load_scene("main.dim")
            .unwrap_or_else(|d| panic!("{d}"));
        let (scene, diagnostics) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
        assert!(!diagnostics.has_errors(), "{diagnostics}");

        let log: Log = Log::default();
        let speaker = Speaker::with_backend(&project, Box::new(Mock::with_log(log.clone())));
        assert!(!speaker.diagnostics.has_errors(), "{}", speaker.diagnostics);
        Harness {
            project,
            sim: Sim::new(
                scene,
                1,
                Box::new(LuaHost::new(60).expect("lua")),
                SimConfig::default(),
            ),
            speaker,
            log,
        }
    }

    fn run(&mut self, ticks: u64) {
        let input = InputLog::new(1, "test", 1);
        for tick in 0..ticks {
            self.sim.step(input.frame(tick));
            self.speaker.tick(&self.sim.state());
        }
    }

    /// Swap to `floor2.dim`, the way a floor change actually happens.
    fn load_floor_two(&mut self) {
        let (scene, diagnostics) = {
            self.project
                .load_scene("floor2.dim")
                .unwrap_or_else(|d| panic!("{d}"));
            self.project
                .runtime_scene()
                .unwrap_or_else(|d| panic!("{d}"))
        };
        assert!(!diagnostics.has_errors(), "{diagnostics}");
        let mut state = self.sim.state().clone();
        state.scene = scene;
        state.scene.update_world_transforms();
        state.readied.clear();
        state.autoplayed.clear();
        self.sim.restore(state);
    }

    /// How many times a clip was started.
    fn starts(&self, clip: &str) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|e| matches!(e, Event::Started { params, .. } if params.clip == clip))
            .count()
    }

    fn stops(&self) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|e| matches!(e, Event::Stopped { .. }))
            .count()
    }
}

#[test]
fn a_theme_survives_a_scene_load() {
    // The defect: one start before the swap, one after, and the bar it was on
    // thrown away. It has to be started exactly once.
    let mut h = Harness::open("survives", Some("theme"));
    h.run(3);
    assert_eq!(h.starts("sfx/theme"), 1, "it did not start");
    h.load_floor_two();
    h.run(3);
    assert_eq!(
        h.starts("sfx/theme"),
        1,
        "the theme restarted on the new floor"
    );
    assert!(h.speaker.theme_playing("sfx/theme", "Music"));
}

#[test]
fn a_node_keyed_voice_still_stops_when_its_node_goes() {
    // The behaviour this must not break: a projectile's loop ends with the
    // projectile, which is why `continuous` is a property and not the default.
    let mut h = Harness::open("one-shot", Some("theme"));
    h.run(3);
    assert_eq!(h.starts("sfx/loop"), 1);
    let before = h.stops();
    h.load_floor_two();
    h.run(1);
    assert!(
        h.stops() > before,
        "the loop on the old floor's node kept playing"
    );
    // And the new floor's own copy starts, because it is a different node.
    h.run(2);
    assert_eq!(h.starts("sfx/loop"), 2);
}

#[test]
fn a_floor_with_no_theme_stops_the_music() {
    // Asked of the scene rather than of a timer: a floor that does not want the
    // track loses it on the tick the swap happens, with no interval to tune and
    // no window in which music plays over a scene that did not ask for it.
    let mut h = Harness::open("silent-floor", None);
    h.run(3);
    assert!(h.speaker.theme_playing("sfx/theme", "Music"));
    h.load_floor_two();
    h.run(1);
    assert!(
        !h.speaker.theme_playing("sfx/theme", "Music"),
        "the theme played on over a floor with no music node"
    );
}

#[test]
fn a_different_track_on_the_same_bus_replaces_the_one_playing() {
    let mut h = Harness::open("replace", Some("other"));
    h.run(3);
    assert!(h.speaker.theme_playing("sfx/theme", "Music"));
    h.load_floor_two();
    h.run(3);
    assert!(
        !h.speaker.theme_playing("sfx/theme", "Music"),
        "the old track is still sounding"
    );
    assert!(h.speaker.theme_playing("sfx/other", "Music"));
    assert_eq!(h.starts("sfx/other"), 1);
}

#[test]
fn a_script_can_still_stop_a_theme() {
    let mut h = Harness::open("stop", Some("theme"));
    h.run(3);
    assert!(h.speaker.theme_playing("sfx/theme", "Music"));

    // What `node:stop()` produces, which for a continuous node has to stop the
    // track rather than a handle a node in some other scene started.
    let state = h.sim.state();
    let id = state.scene.resolve_path("/Floor1/Theme").expect("theme");
    let uid = state.scene.get(id).expect("theme").uid;
    drop(state);
    let mut state = h.sim.state().clone();
    state
        .sounds
        .push(dimetric_sim::sound::SoundEvent::Stop { node: uid });
    h.sim.restore(state);
    h.speaker.tick(&h.sim.state());
    assert!(
        !h.speaker.theme_playing("sfx/theme", "Music"),
        "stop did not reach the theme"
    );
}

#[test]
fn a_reused_node_id_across_a_swap_is_reported() {
    // The trap underneath the sweep, which the game hit and fixed in its
    // generator. The sweep decides "destroyed" by whether the id is still in
    // the tree, so two scene files reusing an id — easy when ids come from a
    // generator — leave a voice attached to whatever landed on that id, and two
    // tracks play at once.
    //
    // `n_loop0000` is the same id on both floors here, and on the second it
    // names a *different* clip, which is what makes it detectable at all.
    let root = std::env::temp_dir().join("dimetric-continuous-reused");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("assets/sfx")).expect("mkdir");
    for clip in ["theme", "other", "loop"] {
        std::fs::write(
            root.join(format!("assets/sfx/{clip}.ogg")),
            b"OggS not really",
        )
        .expect("clip");
    }
    let one = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"One\"\n\n\
         [[node]]\nid = \"n_loop0000\"\nkind = \"Sound\"\nname = \"Loop\"\n\
         parent = \"n_root0000\"\nstream = \"asset:sfx/loop\"\nautoplay = true\nlooping = true\n";
    // Same id, different clip: a generator that reuses ids across floors.
    let two = one
        .replace("sfx/loop", "sfx/other")
        .replace("\"One\"", "\"Two\"");
    std::fs::write(root.join("main.dim"), one).expect("scene");
    std::fs::write(root.join("floor2.dim"), two).expect("scene");

    let mut project = Project::open(&root, 0);
    project.import_assets();
    project
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    let (scene, _) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
    let log: Log = Log::default();
    let mut speaker = Speaker::with_backend(&project, Box::new(Mock::with_log(log.clone())));
    let mut sim = Sim::new(
        scene,
        1,
        Box::new(LuaHost::new(60).expect("lua")),
        SimConfig::default(),
    );
    let input = InputLog::new(1, "test", 1);
    for tick in 0..3 {
        sim.step(input.frame(tick));
        speaker.tick(&sim.state());
    }
    let _ = speaker.diagnostics.iter().count();
    speaker.diagnostics = dimetric_core::Diagnostics::new();

    project
        .load_scene("floor2.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    let (next, _) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
    let mut state = sim.state().clone();
    state.scene = next;
    state.scene.update_world_transforms();
    state.readied.clear();
    state.autoplayed.clear();
    sim.restore(state);
    sim.step(input.frame(3));
    speaker.tick(&sim.state());

    let said: Vec<String> = speaker.diagnostics.iter().map(|d| d.to_string()).collect();
    let joined = said.join("\n");
    assert!(
        joined.contains("DIM1102"),
        "a reused id went unreported: {joined:?}"
    );
    // And the stale voice is stopped rather than left sounding under the new
    // node, which is the actual two-tracks-at-once bug.
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|e| matches!(e, Event::Stopped { .. }))
            .count(),
        1
    );
}
