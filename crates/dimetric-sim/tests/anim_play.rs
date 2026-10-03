//! `anim.play` on the one node kind that has clips.
//!
//! `advance` reads every `AnimatedSprite2D`'s `animation` property each tick
//! and, if the running clip differs, switches back to the property. So
//! `anim.play(node, "walk_se")` set the clip and the next tick put it back —
//! on the only kind that has clips, the documented script route did nothing
//! and reported nothing. `docs/API.md` listed both routes and did not say one
//! cancelled the other.
//!
//! The second half: nothing restarted a clip. Setting `animation` to the
//! value it already holds is rightly not a restart, and `anim.play` of the
//! running clip is documented as not one either — so a non-looping clip that
//! had finished stayed on its last frame for the life of the node. A pooled
//! effect sprite played its burst once and showed the final frame forever.

use dimetric_assets::{Clip, Frame};
use dimetric_core::NodeUid;
use dimetric_scene::{KindRegistry, Value};
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

/// An `AnimatedSprite2D` authored to play `a`, driven by a script.
const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_mover001"
kind = "AnimatedSprite2D"
name = "Mover"
parent = "n_root0000"
frames = "asset:sprites/hero"
animation = "a"
script = "script:scripts/mover.lua"
"##;

fn mover() -> NodeUid {
    NodeUid::parse("n_mover001").unwrap()
}

/// Three frames, two ticks each.
fn clip(name: &str, looping: bool) -> Clip {
    Clip {
        name: name.to_string(),
        frames: (0..3)
            .map(|index| Frame {
                index,
                ticks: 2,
                event: None,
            })
            .collect(),
        looping,
    }
}

fn clips(list: Vec<Clip>) -> dimetric_sim::anim::Clips {
    let mut map = dimetric_sim::anim::Clips::new();
    map.insert("sprites/hero".to_string(), list);
    map
}

fn sim_with(script: &str, list: Vec<Clip>) -> Sim {
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(SCENE, "scene.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/mover.lua", script).expect("loads");
    Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(host),
        SimConfig::default(),
    )
    .with_clips(clips(list))
}

fn run(sim: &mut Sim, ticks: u64) {
    let log = InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
}

/// The clip the node is actually playing, and the frame it is on.
fn playing(sim: &Sim) -> (String, u32) {
    let state = sim.state();
    let a = &state.anim[&mover()];
    (a.clip.clone(), a.frame)
}

/// The node's `animation` property, which is what the scene file carries.
fn property(sim: &Sim) -> String {
    let state = sim.state();
    let id = state.scene.by_uid(mover()).expect("node");
    state
        .scene
        .get(id)
        .and_then(|n| n.get("animation"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_script_can_switch_the_clip_on_an_animated_sprite() {
    // The defect. One call on the first tick, then nothing — and the clip has
    // to stay switched rather than being put back by the next `advance`.
    let script = "local done = false\n\
                  function on_tick(self)\n\
                  \x20 if not done then done = true; anim.play(self, \"b\") end\n\
                  end\n";
    let mut sim = sim_with(script, vec![clip("a", true), clip("b", true)]);
    run(&mut sim, 6);
    assert_eq!(
        playing(&sim).0,
        "b",
        "the clip was put back to the property"
    );
}

#[test]
fn the_two_routes_are_one_route() {
    // `anim.play` writes the property it would otherwise be fighting, so the
    // node carries what it is playing and a save round-trips to the same
    // thing. Anything else leaves two sources of truth for one clip.
    let script = "local done = false\n\
                  function on_tick(self)\n\
                  \x20 if not done then done = true; anim.play(self, \"b\") end\n\
                  end\n";
    let mut sim = sim_with(script, vec![clip("a", true), clip("b", true)]);
    run(&mut sim, 3);
    assert_eq!(property(&sim), "b");
}

#[test]
fn switching_from_a_script_starts_the_new_clip_at_its_first_frame() {
    let script = "local n = 0\n\
                  function on_tick(self)\n\
                  \x20 n = n + 1\n\
                  \x20 if n == 4 then anim.play(self, \"b\") end\n\
                  end\n";
    let mut sim = sim_with(script, vec![clip("a", true), clip("b", true)]);
    run(&mut sim, 4);
    assert_eq!(playing(&sim), ("b".to_string(), 0));
}

#[test]
fn playing_the_running_clip_still_does_not_restart_it() {
    // Documented, relied on, and must survive this: `anim.play(self, "a")`
    // every tick is how a movement script is actually written.
    let script = "function on_tick(self) anim.play(self, \"a\") end\n";
    let mut sim = sim_with(script, vec![clip("a", true)]);
    run(&mut sim, 5);
    assert_eq!(playing(&sim), ("a".to_string(), 2));
}

#[test]
fn a_finished_one_shot_clip_can_be_replayed() {
    // The second half. Six ticks finishes a three-frame clip at two ticks a
    // frame; `anim.restart` has to put it back to frame 0 and playing, which
    // neither `play` nor writing the property could do.
    let script = "function on_tick(self)\n\
                  \x20 if anim.finished(self) then anim.restart(self) end\n\
                  end\n";
    let mut sim = sim_with(script, vec![clip("a", false)]);
    run(&mut sim, 6);
    // Finished and restarted at least once rather than stuck on frame 2.
    run(&mut sim, 1);
    assert!(!sim.state().anim[&mover()].finished, "still finished");
    assert_eq!(playing(&sim).1, 0, "did not go back to the first frame");
}

#[test]
fn restart_does_not_need_the_clip_to_have_finished() {
    let script = "local n = 0\n\
                  function on_tick(self)\n\
                  \x20 n = n + 1\n\
                  \x20 if n == 4 then anim.restart(self) end\n\
                  end\n";
    let mut sim = sim_with(script, vec![clip("a", true)]);
    run(&mut sim, 4);
    assert_eq!(playing(&sim), ("a".to_string(), 0));
}

#[test]
fn restart_on_a_node_with_no_animation_is_quiet() {
    // Nothing to restart is not an error, the way clearing an absent variable
    // is not. It must not invent an entry in `anim` either: that map is
    // hashed, so a node with a phantom playback state is a divergence.
    let script = "function on_tick(self) anim.restart(scene.find(\"/Room\")) end\n";
    let mut sim = sim_with(script, vec![clip("a", true)]);
    run(&mut sim, 3);
    assert!(!sim.diagnostics().has_errors(), "{}", sim.diagnostics());
    let root = NodeUid::parse("n_root0000").unwrap();
    assert!(
        !sim.state().anim.contains_key(&root),
        "restart invented playback state for a node that has none"
    );
}

#[test]
fn a_script_playing_a_clip_the_sheet_does_not_have_is_still_quiet() {
    // Unchanged behaviour: a missing asset should look wrong, not stop a tick.
    let script = "function on_tick(self) anim.play(self, \"nope\") end\n";
    let mut sim = sim_with(script, vec![clip("a", true)]);
    run(&mut sim, 5);
    assert!(!sim.diagnostics().has_errors(), "{}", sim.diagnostics());
}

/// A `TextureRect` is a control, not a sprite — and an animated icon is the
/// same question about the same clips.
mod animated_icon {
    use super::*;

    const UI: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Hud"
script = "script:scripts/mover.lua"

[[node]]
id = "n_icon0000"
kind = "TextureRect"
name = "Icon"
parent = "n_root0000"
texture = "asset:sprites/hero"
animation = "a"
"##;

    fn icon() -> NodeUid {
        NodeUid::parse("n_icon0000").unwrap()
    }

    fn sim(script: &str, list: Vec<Clip>) -> Sim {
        let registry = KindRegistry::with_builtins();
        let out = dimetric_scene::parse(UI, "ui.dim", &registry);
        assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
        let mut host = LuaHost::new(60).expect("lua");
        host.load("scripts/mover.lua", script).expect("loads");
        let mut map = dimetric_sim::anim::Clips::new();
        map.insert("sprites/hero".to_string(), list);
        Sim::new(
            out.doc.unwrap().scene,
            1,
            Box::new(host),
            SimConfig::default(),
        )
        .with_clips(map)
    }

    #[test]
    fn an_animated_icon_advances_without_a_script() {
        // The sheet comes off `texture` rather than `frames`, because that is
        // what a control calls it.
        let mut s = sim("function on_tick(self) end\n", vec![clip("a", true)]);
        run(&mut s, 3);
        assert_eq!(s.state().anim[&icon()].frame, 1);
    }

    #[test]
    fn a_script_can_switch_an_icons_clip() {
        let script = "local done = false\n\
                      function on_tick(self)\n\
                      \x20 if not done then\n\
                      \x20   done = true\n\
                      \x20   anim.play(scene.find(\"/Hud/Icon\"), \"b\")\n\
                      \x20 end\n\
                      end\n";
        let mut s = sim(script, vec![clip("a", true), clip("b", true)]);
        run(&mut s, 6);
        assert_eq!(s.state().anim[&icon()].clip, "b");
    }

    #[test]
    fn the_frame_lands_on_the_node_where_the_renderer_reads_it() {
        let mut s = sim("function on_tick(self) end\n", vec![clip("a", true)]);
        run(&mut s, 3);
        let state = s.state();
        let id = state.scene.by_uid(icon()).expect("icon");
        assert_eq!(
            state
                .scene
                .get(id)
                .and_then(|n| n.get("frame"))
                .and_then(Value::as_int),
            Some(1)
        );
    }
}
