//! A clip draws its own frames, end to end: the clip plays, and the sheet slice
//! the renderer picks is the one that clip names.
//!
//! The simulation kept `AnimState.frame` as the position *within* the clip — 0,
//! 1, 2 — and wrote that number to the node's `frame` property. The renderer
//! read `frame` as an index into the *whole strip*. Nothing between them mapped
//! one to the other, and `Clip::frames[i].index`, the strip index the importer
//! computed from the sidecar's `from`/`to`, was never read after import.
//!
//! So every clip drew the first frames of its sheet. Whatever a figure played,
//! it showed strip frames 0 onward — on this game's sheets, `idle_ne` and then
//! `walk_ne`. Attacks, hurt reactions and deaths had never been drawn.
//!
//! It got through because the sim-side test asserted the clip-relative number
//! landed on the node, and nothing checked what the renderer made of it. This
//! test is the missing half: it spans both crates on purpose, because the
//! defect lived in the gap between them and neither side was wrong alone.

use dimetric_assets::sheet::Framed;
use dimetric_assets::{Clip, Frame, Image};
use dimetric_core::NodeUid;
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::KindRegistry;
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

/// Eight frames of 8x8 side by side, each a flat shade of its own index, so the
/// pixel a frame draws names the frame it came from.
const FRAMES: u32 = 8;
const SIDE: u32 = 8;

fn atlas() -> Atlas {
    let mut pixels = Vec::with_capacity((FRAMES * SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        let _ = y;
        for x in 0..FRAMES * SIDE {
            let frame = (x / SIDE) as u8;
            // 0x10 apart, so the shade reads back as the frame index exactly.
            pixels.extend_from_slice(&[frame * 0x10, 0x00, 0x00, 0xFF]);
        }
    }
    Atlas::pack_framed(
        vec![
            Framed {
                image: Image {
                    name: "sprites/hero".into(),
                    width: FRAMES * SIDE,
                    height: SIDE,
                    pixels,
                },
                frames: FRAMES,
            },
            Framed {
                image: dimetric_render::atlas::placeholder(8),
                frames: 1,
            },
        ],
        256,
    )
}

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_hero0000"
kind = "AnimatedSprite2D"
name = "Hero"
parent = "n_root0000"
frames = "asset:sprites/hero"
animation = "idle"
script = "script:scripts/hero.lua"
"##;

/// A clip over strip frames `start ..= start + count - 1`.
fn clip_at(name: &str, start: u32, count: u32, looping: bool) -> Clip {
    Clip {
        name: name.to_string(),
        frames: (0..count)
            // Two ticks a frame, so the first tick of a clip shows its first
            // frame: `advance` runs after the script in the same tick and
            // counts a tick against the frame immediately, so a one-tick frame
            // is already gone by the time anything looks.
            .map(|offset| Frame {
                index: start + offset,
                ticks: 2,
                event: None,
            })
            .collect(),
        looping,
    }
}

fn sim_with(script: &str) -> Sim {
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(SCENE, "scene.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/hero.lua", script).expect("loads");
    let mut clips = dimetric_sim::anim::Clips::new();
    clips.insert(
        "sprites/hero".to_string(),
        vec![
            // Two frames each, laid out the way a real sheet is: idle first,
            // then the clips nobody could draw.
            clip_at("idle", 0, 2, true),
            clip_at("attack", 2, 2, false),
            clip_at("hurt", 4, 2, false),
            clip_at("die", 6, 2, false),
        ],
    );
    Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(host),
        SimConfig::default(),
    )
    .with_clips(clips)
}

fn run(sim: &mut Sim, ticks: u64) {
    let log = InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
}

/// Which strip frame the renderer actually sampled, read off the UVs.
///
/// The sheet is one strip of eight, so a frame's `u_min` is its index eighths
/// of the way across the packed region — the renderer's own arithmetic, read
/// back out.
fn sampled_frame(sim: &Sim, atlas: &Atlas) -> u32 {
    let state = sim.state();
    let camera = Camera::new((64, 64));
    let frame = extract(&state.scene, atlas, &camera, None);
    let want = NodeUid::parse("n_hero0000").expect("an id");
    let item = frame
        .sprites
        .iter()
        .find(|i| i.node == want)
        .expect("the hero was drawn");
    let whole = atlas.region("sprites/hero").expect("the strip is packed");
    let [u0, _, _, _] = item.uv;
    let [w0, _, w1, _] = whole.uv;
    let across = (u0 - w0) / (w1 - w0);
    let index = (across * FRAMES as f32).round();
    assert!(
        (across * FRAMES as f32 - index).abs() < 1e-3,
        "the slice did not land on a frame boundary: {across}"
    );
    index as u32
}

#[test]
fn a_clip_draws_its_own_frames_and_not_the_sheets_first() {
    // The defect, end to end. Each of these clips starts somewhere other than
    // zero, and before the fix every one of them drew frame 0 or 1.
    let atlas = atlas();
    for (clip, first, second) in [("attack", 2, 3), ("hurt", 4, 5), ("die", 6, 7)] {
        let mut sim = sim_with(&format!(
            "function on_tick(self)\n\
             \x20 if tick.count() == 1 then anim.play(scene.find(\"/Room/Hero\"), \"{clip}\") end\n\
             end\n"
        ));
        // Two ticks: `advance` runs before the scripts, so the `anim.play` on
        // tick one takes effect on tick two's advance.
        run(&mut sim, 2);
        assert_eq!(
            sampled_frame(&sim, &atlas),
            first,
            "{clip} should draw its own first frame"
        );
        run(&mut sim, 1);
        assert_eq!(
            sampled_frame(&sim, &atlas),
            second,
            "{clip} should walk its own frames"
        );
        let reported = sim.take_diagnostics();
        assert!(!reported.has_errors(), "{reported}");
    }
}

#[test]
fn the_idle_clip_still_draws_the_sheets_first_frames() {
    // The control. A clip that *does* start at zero drew correctly before and
    // has to go on doing so — which is exactly why the defect was invisible on
    // the one clip every figure spends most of its time in.
    let atlas = atlas();
    let mut sim = sim_with("function on_tick(self) end\n");
    run(&mut sim, 1);
    assert_eq!(sampled_frame(&sim, &atlas), 0);
    run(&mut sim, 1);
    assert_eq!(sampled_frame(&sim, &atlas), 1);
}

#[test]
fn a_death_held_for_forty_ticks_stays_on_the_floor() {
    // The report's own case. A non-looping clip finishes and holds its last
    // frame; before the fix it held the *strip's* frame 1, so the adventurer
    // stood through its whole death.
    let atlas = atlas();
    let mut sim = sim_with(
        "function on_tick(self)\n\
         \x20 if tick.count() == 1 then anim.play(scene.find(\"/Room/Hero\"), \"die\") end\n\
         end\n",
    );
    run(&mut sim, 40);
    assert_eq!(
        sampled_frame(&sim, &atlas),
        7,
        "the last frame of `die`, not of the sheet"
    );
}
