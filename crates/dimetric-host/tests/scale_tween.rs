//! A tween on `scale` is visible frame to frame.
//!
//! The two halves of this live in different crates — `dimetric-sim` writes the
//! scale into the transform, `dimetric-render` reads the world transform — and
//! each was right on its own while the seam between them was not: the renderer
//! never read `scale`, so a tween ran, was hashed, was snapshotted, and changed
//! nothing anybody could see. This is the end-to-end check that the seam holds.

use dimetric_core::NodeUid;
use dimetric_render::{extract, Atlas, Camera, Projection};
use dimetric_sim::{LuaHost, Sim, SimConfig};

const SCENE: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_bar00000"
kind = "Sprite2D"
name = "Bar"
parent = "n_root0000"
texture = "asset:sprites/bar"
script = "script:scripts/bar.lua"
"##;

/// A 16x4 bar, the shape a health bar is.
fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            dimetric_render::atlas::Source {
                name: "sprites/bar".into(),
                width: 16,
                height: 4,
                pixels: [0x20u8, 0xC0, 0x40, 0xFF].repeat(16 * 4),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        128,
    )
}

/// How wide the bar is drawn, in pixels.
fn width(sim: &Sim) -> i32 {
    let camera = Camera {
        projection: Projection::TopDown,
        ..Camera::new((64, 64))
    };
    let state = sim.state();
    let frame = extract(&state.scene, &atlas(), &camera, None);
    let bar = frame
        .sprites
        .iter()
        .find(|i| i.node == NodeUid::parse("n_bar00000").unwrap())
        .expect("the bar is drawn");
    bar.size.x.round_int()
}

#[test]
fn a_health_bar_can_shrink() {
    // Ten ticks from full width to a tenth of it, read off the drawn quad
    // rather than off the transform — the transform was already right.
    let script = "local started = false\n\
                  function on_tick(self)\n\
                  \x20 if not started then\n\
                  \x20   started = true\n\
                  \x20   tween.to(self, \"scale\", vec2(fx.parse(\"0.25\"), fx.new(1)), 10, \"linear\")\n\
                  \x20 end\n\
                  end\n";
    let registry = dimetric_scene::KindRegistry::with_builtins();
    let out = dimetric_scene::parse(SCENE, "s.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut host = LuaHost::new(60).expect("lua");
    host.load("scripts/bar.lua", script).expect("loads");
    let mut sim = Sim::new(
        out.doc.unwrap().scene,
        1,
        Box::new(host),
        SimConfig::default(),
    );

    assert_eq!(width(&sim), 16, "it starts at its own width");
    let log = dimetric_sim::InputLog::new(1, "test", 1);
    let mut seen = vec![width(&sim)];
    for tick in 0..11 {
        sim.step(log.frame(tick));
        seen.push(width(&sim));
    }
    assert!(!sim.diagnostics().has_errors(), "{}", sim.diagnostics());
    assert_eq!(
        *seen.last().expect("ticks"),
        4,
        "it ends at a quarter: {seen:?}"
    );
    assert!(
        seen.windows(2).any(|w| w[1] != w[0]),
        "nothing changed between frames: {seen:?}"
    );
    assert!(
        seen.windows(2).all(|w| w[1] <= w[0]),
        "it did not shrink monotonically: {seen:?}"
    );
}
