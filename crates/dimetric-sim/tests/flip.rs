//! Mirroring a sprite is presentation and must not reach the simulation.
//!
//! Worth confirming rather than assuming, because *which way a character
//! faces* is gameplay information even though the flip itself is not. A game
//! decides its facing in script — that is hashed, in a node variable, like any
//! other decision — and then sets `flip_h` to draw it. Only the second half is
//! presentation.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, NoScripts, Sim, SimConfig};

fn scene_with(flip: &str) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\n\
         [[node]]\nid = \"n_hero0000\"\nkind = \"AnimatedSprite2D\"\nname = \"Hero\"\n\
         parent = \"n_root0000\"\nframes = \"asset:sprites/hero\"\nanimation = \"idle\"\n{flip}\n"
    );
    let out = dimetric_scene::parse(&text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

#[test]
fn an_animated_sprite_accepts_flip_h_and_flip_v() {
    // Before this, `flip_h = true` on an `AnimatedSprite2D` was DIM0301 —
    // an unknown property — while `Sprite2D` took it happily.
    let _ = scene_with("flip_h = true");
    let _ = scene_with("flip_v = true");
    let _ = scene_with("flip_h = true\nflip_v = true");
}

#[test]
fn flipping_moves_the_state_hash_because_it_is_a_node_property() {
    // Stated plainly because the request asked whether it moves the hash, and
    // the honest answer is yes — but not for the reason that would be a
    // problem. `flip_h` is authored scene data, and the scene is hashed, the
    // same as `modulate` or `visible`. What matters is that *drawing* it
    // changes nothing: the renderer reads it and writes nothing back (I7).
    let hash_of = |flip: &str| {
        let mut sim = Sim::new(
            scene_with(flip),
            1,
            Box::new(NoScripts),
            SimConfig::default(),
        );
        sim.step(InputFrame::idle(1));
        sim.hash()
    };
    assert_ne!(hash_of(""), hash_of("flip_h = true"));
}

#[test]
fn a_script_setting_the_flip_is_as_reproducible_as_any_other_property() {
    // The way a game actually uses it: pick a facing from gameplay state, then
    // mirror the sheet. Two runs of the same seed have to agree.
    let script = r#"
function on_tick(self)
  local sprite = scene.find("/World/Hero")
  -- Facing is gameplay and lives in a variable; the flip is how it is drawn.
  self.facing_west = (tick.count() % 4) < 2
  sprite:set("flip_h", self.facing_west)
end
"#;
    let run = || {
        let mut host = LuaHost::new(60).expect("lua host");
        host.load("scripts/a.lua", script).expect("loads");
        let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
             [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
             script = \"script:scripts/a.lua\"\n\n\
             [[node]]\nid = \"n_hero0000\"\nkind = \"AnimatedSprite2D\"\nname = \"Hero\"\n\
             parent = \"n_root0000\"\nframes = \"asset:sprites/hero\"\nanimation = \"idle\"\n";
        let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
        assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
        let mut sim = Sim::new(
            out.doc.unwrap().scene,
            9,
            Box::new(host),
            SimConfig::default(),
        );
        for _ in 0..6 {
            sim.step(InputFrame::idle(1));
        }
        let d = sim.take_diagnostics();
        assert!(!d.has_errors(), "{d}");
        sim.hash()
    };
    let once = run();
    for _ in 0..4 {
        assert_eq!(run(), once);
    }
}
