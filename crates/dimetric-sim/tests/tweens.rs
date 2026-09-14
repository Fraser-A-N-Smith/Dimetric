//! Cosmetic tweens and frame animation: deterministic, snapshotted, hashed.

use dimetric_assets::{Clip, Frame};
use dimetric_core::{Angle, Fx, NodeUid, Vec2Fx};
use dimetric_scene::{Color, KindRegistry, Value};
use dimetric_sim::tween::Easing;
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

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
kind = "Sprite2D"
name = "Mover"
parent = "n_root0000"
pos = [0.0, 0.0]
texture = "asset:sprites/hero"
script = "script:scripts/mover.lua"
"##;

fn sim_with(script: &str) -> Sim {
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(SCENE, "scene.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/mover.lua", script)
        .expect("script loads");
    Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(host),
        SimConfig::default(),
    )
}

fn mover() -> NodeUid {
    NodeUid::parse("n_mover001").unwrap()
}

fn position(sim: &Sim) -> Vec2Fx {
    let state = sim.state();
    let id = state.scene.by_uid(mover()).expect("the mover exists");
    state.scene.get(id).expect("node").transform.pos
}

fn run(sim: &mut Sim, ticks: u64) {
    let log = InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
}

/// Starts one tween on the first tick and nothing after.
fn tween_once(body: &str) -> String {
    format!(
        "local started = false\nfunction on_tick(self)\n  if not started then\n    started = true\n    {body}\n  end\nend\n"
    )
}

#[test]
fn a_tween_arrives_exactly_on_its_target() {
    let mut sim = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(10, 20), 10, \"linear\")",
    ));
    run(&mut sim, 11);
    assert_eq!(position(&sim), Vec2Fx::from_ints(10, 20));
    // And it is gone, rather than sitting on the target for ever.
    assert!(sim.state().tweens.is_empty());
}

#[test]
fn a_tween_is_part_way_there_part_way_through() {
    let mut sim = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(100, 0), 8, \"linear\")",
    ));
    // A tween started during `on_tick` advances in the same tick, because the
    // advance phase comes later in the order. So `ticks = 8` means eight ticks
    // from the call to the target, not nine.
    run(&mut sim, 4);
    assert_eq!(position(&sim), Vec2Fx::from_ints(50, 0));
}

#[test]
fn progress_that_is_not_an_exact_fraction_is_still_the_same_everywhere() {
    // Six tenths is not representable in I16F16, so a tween six ticks into ten
    // is a hair under 60. That is fine and it is the point: every machine is a
    // hair under 60 by exactly the same amount.
    let mut sim = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(100, 0), 10, \"linear\")",
    ));
    run(&mut sim, 6);
    let x = position(&sim).x;
    assert!(x < Fx::from_int(60) && x > Fx::from_int(59), "{x:?}");

    let mut again = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(100, 0), 10, \"linear\")",
    ));
    run(&mut again, 6);
    assert_eq!(position(&again).x, x);
}

#[test]
fn a_tween_of_zero_ticks_lands_immediately() {
    let mut sim = sim_with(&tween_once("tween.to(self, \"pos\", vec2(4, 4), 0)"));
    run(&mut sim, 2);
    assert_eq!(position(&sim), Vec2Fx::from_ints(4, 4));
}

#[test]
fn starting_a_tween_on_a_property_replaces_the_one_already_running() {
    // Two tweens fighting over one number is never what anybody meant.
    let script = "local n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n == 1 then tween.to(self, \"pos\", vec2(100, 0), 100) end\n  if n == 2 then tween.to(self, \"pos\", vec2(0, 50), 10) end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 3);
    assert_eq!(sim.state().tweens[&mover()].len(), 1);
    run(&mut sim, 20);
    assert_eq!(position(&sim), Vec2Fx::from_ints(0, 50));
}

#[test]
fn a_tween_can_be_cancelled_where_it_stands() {
    let script = "local n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n == 1 then tween.to(self, \"pos\", vec2(100, 0), 100) end\n  if n == 11 then tween.cancel(self, \"pos\") end\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 12);
    assert!(sim.state().tweens.is_empty());
    let held = position(&sim);
    run(&mut sim, 20);
    assert_eq!(position(&sim), held, "it stays where it was cancelled");
}

#[test]
fn a_script_can_ask_whether_a_tween_is_running() {
    let script = "function on_tick(self)\n  if not tween.running(self, \"pos\") then\n    tween.to(self, \"pos\", vec2(self:get(\"z\") or 0, 0), 4)\n  end\n  self.seen = tween.running(self, \"pos\")\nend\n";
    let mut sim = sim_with(script);
    run(&mut sim, 2);
    let state = sim.state();
    assert_eq!(
        state.vars[&mover()].get("seen"),
        Some(&Value::Bool(true)),
        "the tween it just started is running"
    );
}

#[test]
fn tweening_a_property_that_does_not_exist_is_an_error_rather_than_a_guess() {
    let mut sim = sim_with(&tween_once("tween.to(self, \"nonsense\", 4, 10)"));
    run(&mut sim, 2);
    assert!(
        sim.diagnostics()
            .to_string()
            .contains("nothing to tween from"),
        "{}",
        sim.diagnostics()
    );
}

#[test]
fn tweening_between_two_different_shapes_is_refused() {
    let mut sim = sim_with(&tween_once("tween.to(self, \"pos\", 4, 10)"));
    run(&mut sim, 2);
    assert!(
        sim.diagnostics().to_string().contains("vec2"),
        "{}",
        sim.diagnostics()
    );
}

#[test]
fn an_unknown_easing_names_the_ones_that_exist() {
    let mut sim = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(1, 1), 4, \"bouncy\")",
    ));
    run(&mut sim, 2);
    assert!(
        sim.diagnostics().to_string().contains("ease_in_out"),
        "{}",
        sim.diagnostics()
    );
}

#[test]
fn a_tween_survives_a_snapshot_and_restore() {
    // Which is the whole reason tweens are simulation state rather than
    // presentation: a rollback with tweens outside it would leave them
    // mid-flight, writing to positions that had just been rewound.
    let mut sim = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(100, 0), 20, \"ease_in_out\")",
    ));
    run(&mut sim, 6);
    let snapshot = sim.snapshot();
    let hash_at_six = sim.hash();

    run(&mut sim, 10);
    assert_ne!(sim.hash(), hash_at_six);

    sim.restore(snapshot);
    assert_eq!(sim.hash(), hash_at_six);
    run(&mut sim, 10);
    let replayed = position(&sim);

    let mut fresh = sim_with(&tween_once(
        "tween.to(self, \"pos\", vec2(100, 0), 20, \"ease_in_out\")",
    ));
    run(&mut fresh, 16);
    assert_eq!(replayed, position(&fresh));
}

#[test]
fn a_running_tween_changes_the_state_hash() {
    let mut still = sim_with("function on_tick(self) end\n");
    let mut moving = sim_with(&tween_once("tween.to(self, \"pos\", vec2(9, 9), 30)"));
    run(&mut still, 4);
    run(&mut moving, 4);
    assert_ne!(still.hash(), moving.hash());
}

// -- easing -------------------------------------------------------------

#[test]
fn every_easing_starts_at_zero_and_ends_at_one() {
    for easing in [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ] {
        assert_eq!(easing.shape(Fx::ZERO), Fx::ZERO, "{easing:?}");
        assert_eq!(easing.shape(Fx::ONE), Fx::ONE, "{easing:?}");
    }
}

#[test]
fn ease_in_is_behind_linear_and_ease_out_is_ahead_of_it() {
    let quarter = Fx::ONE / 4;
    assert!(Easing::EaseIn.shape(quarter) < Easing::Linear.shape(quarter));
    assert!(Easing::EaseOut.shape(quarter) > Easing::Linear.shape(quarter));
}

#[test]
fn easing_names_round_trip() {
    for easing in [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ] {
        assert_eq!(Easing::parse(easing.name()), Some(easing));
    }
    assert_eq!(Easing::parse("bouncy"), None);
}

#[test]
fn an_angle_tween_goes_the_short_way_round() {
    // 350 degrees to 10 degrees is twenty degrees, not three hundred and forty.
    let from = Value::Angle(Angle::from_degrees_str("350").unwrap());
    let to = Value::Angle(Angle::from_degrees_str("10").unwrap());
    let half = dimetric_sim::tween::interpolate(&from, &to, Fx::ONE / 2);
    let Value::Angle(midpoint) = half else {
        panic!("an angle tween should produce an angle");
    };
    let degrees = midpoint.to_degrees_f32();
    assert!(
        !(20.0..=340.0).contains(&degrees),
        "midpoint went the long way: {degrees}"
    );
}

#[test]
fn a_colour_tween_moves_every_channel() {
    let from = Value::Color(Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    });
    let to = Value::Color(Color {
        r: 255,
        g: 100,
        b: 50,
        a: 0,
    });
    let Value::Color(half) = dimetric_sim::tween::interpolate(&from, &to, Fx::ONE / 2) else {
        panic!("a colour tween should produce a colour");
    };
    assert_eq!((half.r, half.g, half.b, half.a), (128, 50, 25, 128));
}

#[test]
fn two_values_of_different_shapes_hold_and_then_snap() {
    let from = Value::Str("a".to_string());
    let to = Value::Str("b".to_string());
    assert_eq!(
        dimetric_sim::tween::interpolate(&from, &to, Fx::ONE / 2),
        from
    );
    assert_eq!(dimetric_sim::tween::interpolate(&from, &to, Fx::ONE), to);
}

// -- animation ----------------------------------------------------------

fn walk_clip() -> Clip {
    Clip {
        name: "walk".to_string(),
        frames: vec![
            Frame {
                index: 0,
                ticks: 2,
                event: None,
            },
            Frame {
                index: 1,
                ticks: 2,
                event: Some("step".to_string()),
            },
            Frame {
                index: 2,
                ticks: 2,
                event: None,
            },
        ],
        looping: true,
    }
}

fn clips(clip: Clip) -> dimetric_sim::anim::Clips {
    let mut map = dimetric_sim::anim::Clips::new();
    map.insert("sprites/hero".to_string(), vec![clip]);
    map
}

#[test]
fn a_clip_advances_a_frame_every_time_its_ticks_run_out() {
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"walk\") end\n")
        .with_clips(clips(walk_clip()));
    run(&mut sim, 1);
    assert_eq!(sim.state().anim[&mover()].frame, 0);
    run(&mut sim, 2);
    assert_eq!(sim.state().anim[&mover()].frame, 1);
    run(&mut sim, 2);
    assert_eq!(sim.state().anim[&mover()].frame, 2);
}

#[test]
fn a_looping_clip_comes_back_round() {
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"walk\") end\n")
        .with_clips(clips(walk_clip()));
    run(&mut sim, 7);
    assert_eq!(sim.state().anim[&mover()].frame, 0);
    assert!(!sim.state().anim[&mover()].finished);
}

#[test]
fn a_clip_that_does_not_loop_finishes_on_its_last_frame() {
    let mut clip = walk_clip();
    clip.looping = false;
    let mut sim =
        sim_with("function on_tick(self) anim.play(self, \"walk\") end\n").with_clips(clips(clip));
    run(&mut sim, 20);
    let state = sim.state();
    assert_eq!(state.anim[&mover()].frame, 2);
    assert!(state.anim[&mover()].finished);
    assert!(!state.anim[&mover()].playing);
}

#[test]
fn reaching_a_frame_with_an_event_calls_the_script() {
    let script = "function on_tick(self) anim.play(self, \"walk\") end\nfunction on_anim_event(self, name)\n  self.steps = (self.steps or 0) + 1\n  self.last = name\nend\n";
    let mut sim = sim_with(script).with_clips(clips(walk_clip()));
    run(&mut sim, 3);
    let state = sim.state();
    assert_eq!(
        state.vars[&mover()].get("last"),
        Some(&Value::Str("step".to_string()))
    );
    assert_eq!(state.vars[&mover()].get("steps"), Some(&Value::Int(1)));
}

#[test]
fn playing_the_clip_that_is_already_playing_does_not_restart_it() {
    // So `anim.play(self, "walk")` every tick is harmless, which is how anyone
    // actually writes a movement script.
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"walk\") end\n")
        .with_clips(clips(walk_clip()));
    run(&mut sim, 5);
    assert_eq!(sim.state().anim[&mover()].frame, 2);
}

#[test]
fn switching_clips_starts_the_new_one_from_the_beginning() {
    let mut idle = walk_clip();
    idle.name = "idle".to_string();
    let mut map = clips(walk_clip());
    map.get_mut("sprites/hero").unwrap().push(idle);

    let script = "local n = 0\nfunction on_tick(self)\n  n = n + 1\n  if n < 5 then anim.play(self, \"walk\") else anim.play(self, \"idle\") end\nend\n";
    let mut sim = sim_with(script).with_clips(map);
    run(&mut sim, 5);
    let state = sim.state();
    assert_eq!(state.anim[&mover()].clip, "idle");
    assert_eq!(state.anim[&mover()].frame, 0);
}

#[test]
fn a_clip_the_project_does_not_have_holds_rather_than_crashing() {
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"nonexistent\") end\n")
        .with_clips(clips(walk_clip()));
    run(&mut sim, 10);
    assert_eq!(sim.state().anim[&mover()].frame, 0);
    assert!(!sim.diagnostics().has_errors());
}

#[test]
fn a_simulation_with_no_clips_at_all_still_runs() {
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"walk\") end\n");
    run(&mut sim, 10);
    assert!(!sim.diagnostics().has_errors());
}

#[test]
fn an_animated_sprite_node_plays_without_a_script_driving_it() {
    // `animation = "walk"` in the scene file should be enough. Requiring a
    // script to call `anim.play` every tick would make the property a lie.
    const ANIMATED: &str = r##"format = "dimetric"
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
animation = "walk"
"##;
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(ANIMATED, "scene.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut sim = Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(dimetric_sim::NoScripts),
        SimConfig::default(),
    )
    .with_clips(clips(walk_clip()));

    run(&mut sim, 3);
    assert_eq!(sim.state().anim[&mover()].frame, 1);
}

#[test]
fn the_frame_the_engine_landed_on_is_written_onto_the_node() {
    // This is the only path from a playing animation to something being drawn:
    // the renderer reads the scene and never asks the simulation anything.
    let mut sim = sim_with("function on_tick(self) anim.play(self, \"walk\") end\n")
        .with_clips(clips(walk_clip()));
    let scene_frame = |sim: &Sim| {
        let state = sim.state();
        let id = state.scene.by_uid(mover()).expect("node");
        state
            .scene
            .get(id)
            .and_then(|n| n.get("frame"))
            .and_then(Value::as_int)
    };
    // A Sprite2D is not an animated node, so nothing is written to it.
    run(&mut sim, 3);
    assert_eq!(scene_frame(&sim), None);
}

#[test]
fn an_animated_node_carries_its_frame_in_the_scene() {
    const ANIMATED: &str = r##"format = "dimetric"
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
animation = "walk"
"##;
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(ANIMATED, "scene.dim", &registry);
    let mut sim = Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(dimetric_sim::NoScripts),
        SimConfig::default(),
    )
    .with_clips(clips(walk_clip()));

    for expected in [(1u64, 0i64), (3, 1), (5, 2), (7, 0)] {
        while sim.state().tick.0 < expected.0 {
            run(&mut sim, 1);
        }
        let state = sim.state();
        let id = state.scene.by_uid(mover()).expect("node");
        assert_eq!(
            state
                .scene
                .get(id)
                .and_then(|n| n.get("frame"))
                .and_then(Value::as_int),
            Some(expected.1),
            "at tick {}",
            expected.0
        );
    }
}

#[test]
fn two_sheets_may_each_have_a_clip_called_walk() {
    let mut map = dimetric_sim::anim::Clips::new();
    map.insert("sprites/hero".to_string(), vec![walk_clip()]);
    let mut slow = walk_clip();
    slow.frames[0].ticks = 50;
    map.insert("sprites/skeleton".to_string(), vec![slow]);

    const ANIMATED: &str = r##"format = "dimetric"
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
frames = "asset:sprites/skeleton"
animation = "walk"
"##;
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(ANIMATED, "scene.dim", &registry);
    let mut sim = Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(dimetric_sim::NoScripts),
        SimConfig::default(),
    )
    .with_clips(map);

    run(&mut sim, 5);
    // The skeleton's own "walk" holds its first frame for fifty ticks, so this
    // is still frame zero. Picking the hero's would have advanced it.
    assert_eq!(sim.state().anim[&mover()].frame, 0);
}

#[test]
fn setting_playing_to_false_in_the_scene_holds_the_frame() {
    const ANIMATED: &str = r##"format = "dimetric"
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
animation = "walk"
playing = false
"##;
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(ANIMATED, "scene.dim", &registry);
    let mut sim = Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(dimetric_sim::NoScripts),
        SimConfig::default(),
    )
    .with_clips(clips(walk_clip()));

    run(&mut sim, 20);
    assert_eq!(sim.state().anim[&mover()].frame, 0);
}

/// A scene with one `AnimatedSprite2D`, optionally at a non-default speed.
fn animated_scene(extra: &str) -> dimetric_scene::Scene {
    let text = format!(
        r##"format = "dimetric"
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
animation = "walk"
{extra}
"##
    );
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(&text, "scene.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn animated_sim(extra: &str) -> Sim {
    Sim::new(
        animated_scene(extra),
        1,
        Box::new(dimetric_sim::NoScripts),
        SimConfig::default(),
    )
    .with_clips(clips(walk_clip()))
}

#[test]
fn playback_speed_is_an_integer_ratio_that_is_actually_applied() {
    // The schema declares `speed_numerator` and `speed_denominator`. A declared
    // property that does nothing is worse than no property.
    let mut normal = animated_sim("");
    let mut double = animated_sim("speed_numerator = 2");
    let mut half = animated_sim("speed_denominator = 2");

    // Each frame is authored at two ticks, so after two ticks normal play is
    // one frame in, double play is two, and half play has not moved.
    run(&mut normal, 2);
    run(&mut double, 2);
    run(&mut half, 2);
    assert_eq!(normal.state().anim[&mover()].frame, 1);
    assert_eq!(double.state().anim[&mover()].frame, 2);
    assert_eq!(half.state().anim[&mover()].frame, 0);
}

#[test]
fn double_speed_gets_through_a_clip_in_half_the_ticks() {
    // Three frames at two ticks each: six ticks a cycle, three at double speed.
    let ticks_to_loop = |extra: &str| {
        let mut sim = animated_sim(extra);
        for tick in 1..=20 {
            run(&mut sim, 1);
            // Back at frame 0 having been past it.
            if tick > 1 && sim.state().anim[&mover()].frame == 0 {
                return tick;
            }
        }
        panic!("the clip never looped");
    };
    assert_eq!(ticks_to_loop(""), 6);
    assert_eq!(ticks_to_loop("speed_numerator = 2"), 3);
}

#[test]
fn a_speed_that_would_round_a_frame_to_nothing_still_holds_it_a_tick() {
    // A zero-length frame advances infinitely fast and hangs the walker.
    assert_eq!(
        dimetric_sim::anim::Speed {
            numerator: 1000,
            denominator: 1
        }
        .hold(2),
        1
    );
}

#[test]
fn a_nonsense_speed_is_taken_as_normal_rather_than_dividing_by_zero() {
    let mut sim = animated_sim("speed_numerator = 0\nspeed_denominator = 0");
    run(&mut sim, 2);
    assert_eq!(sim.state().anim[&mover()].frame, 1);
}
