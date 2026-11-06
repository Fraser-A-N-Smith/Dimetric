//! `scale` reaches the transform through the script's node handle.
//!
//! `scale` is a transform key in the scene format, in the reserved-key table,
//! and in a tween — and it was not in the handle's metamethods, which routed
//! `pos`, `rot` and `visible` and treated everything else as a script
//! variable. So `node.scale = vec2(1.1875, 0.8125)` landed in the node's
//! *variables*: nothing on screen moved, nothing was reported, and a tween
//! back to `(1, 1)` started from a transform that had never left it.
//!
//! `node:set("scale", ...)` was refused as a reserved key, with a message
//! saying to write `self.scale = ...` instead — which was the write that did
//! not work. Both halves are fixed here.
//!
//! The second half of the entry is the general one: a reserved key the handle
//! does not route is now refused rather than shadowed, because silence is what
//! made this cost an afternoon.

use dimetric_core::{Fx, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene, Value};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

fn scene() -> Scene {
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:main.lua\"\n\n\
         [[node]]\nid = \"n_hero0000\"\nkind = \"Sprite2D\"\nname = \"Hero\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/hero\"\n\n\
         [[node]]\nid = \"n_trim0000\"\nkind = \"Sprite2D\"\nname = \"Trim\"\n\
         parent = \"n_hero0000\"\ntexture = \"asset:sprites/hero\"\npos = [4.0, 0.0]\n";
    let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// Run one tick of `source` and return the simulation and what it complained
/// about.
fn step(source: &str) -> (Sim, String) {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "script did not load: {load:?}");
    let mut sim = Sim::new(scene(), 1, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    let complaints = sim
        .take_diagnostics()
        .0
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    (sim, complaints)
}

/// The hero's scale, as it stands in the transform.
fn scale_of(sim: &Sim, path: &str) -> Vec2Fx {
    let state = sim.state();
    let id = state.scene.resolve_path(path).expect(path);
    state.scene.get(id).expect(path).transform.scale
}

/// A script variable on the root, for reading values back out of Lua.
fn var(sim: &Sim, name: &str) -> String {
    let state = sim.state();
    let id = state.scene.resolve_path("/World").expect("root");
    let uid = state.scene.get(id).expect("root").uid;
    state
        .vars
        .get(&uid)
        .and_then(|v| v.get(name))
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .unwrap_or_else(|| "<unset>".to_string())
}

#[test]
fn writing_scale_reaches_the_transform() {
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 scene.find(\"/World/Hero\").scale = vec2(2, 2)\n\
         end\n",
    );
    assert_eq!(complaints, "");
    assert_eq!(
        scale_of(&sim, "/World/Hero"),
        Vec2Fx::from_ints(2, 2),
        "the write landed in the node's variables before this"
    );
}

#[test]
fn reading_scale_answers_the_transform() {
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 self.before = tostring(hero.scale:x())\n\
         \x20 hero.scale = vec2(2, 2)\n\
         \x20 self.after = tostring(hero.scale:x())\n\
         end\n",
    );
    assert_eq!(complaints, "");
    assert_eq!(var(&sim, "before"), "1.0", "a fresh node stands at one");
    assert_eq!(var(&sim, "after"), "2.0", "and reads back what was written");
}

#[test]
fn a_squash_can_be_written_and_sprung_back() {
    // The shape the game wanted: a blow squashes the figure it lands on and
    // the figure springs back. Written as a non-uniform scale and a return to
    // one, which is the whole of it.
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero.scale = vec2(1.1875, 0.8125)\n\
         \x20 self.x = tostring(hero.scale:x())\n\
         \x20 self.y = tostring(hero.scale:y())\n\
         end\n",
    );
    assert_eq!(complaints, "");
    assert_eq!(var(&sim, "x"), "1.1875");
    assert_eq!(var(&sim, "y"), "0.8125");
    assert_eq!(
        scale_of(&sim, "/World/Hero"),
        Vec2Fx::new(Fx::from_f64_lossy(1.1875), Fx::from_f64_lossy(0.8125)),
        "exactly representable in fixed point, so it survives the round trip"
    );
}

#[test]
fn scaling_a_parent_moves_its_children() {
    // Why `scale` belongs on the transform rather than on the sprite: the
    // write has to mark the subtree's world transforms stale, or a scaled
    // parent would draw its children where they were.
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 scene.find(\"/World/Hero\").scale = vec2(3, 1)\n\
         end\n",
    );
    assert_eq!(complaints, "");
    let state = sim.state();
    let trim = state.scene.resolve_path("/World/Hero/Trim").expect("trim");
    assert_eq!(
        state.scene.world_of(trim).expect("a transform").pos,
        Vec2Fx::from_ints(12, 0),
        "four units along inside a parent scaled three is twelve"
    );
}

#[test]
fn node_set_still_refuses_scale_but_now_says_where_to_write_it() {
    // `node:set` addresses the property map and `scale` is not in it, so
    // setting it there would shadow the real one. The refusal stands; what
    // changed is that the advice it gives is now true.
    let (_, complaints) = step(
        "function on_tick(self)\n\
         \x20 scene.find(\"/World/Hero\"):set(\"scale\", 2)\n\
         end\n",
    );
    assert!(complaints.contains("reserved key"), "{complaints}");
    assert!(
        complaints.contains("self.scale = ..."),
        "it has to name the write that works: {complaints}"
    );
}

#[test]
fn a_reserved_key_the_handle_does_not_route_is_refused() {
    // The general half. Before this, `node.kind = "x"` stored a variable
    // called `kind` beside the node's real kind: nothing read it, nothing said
    // so, and the node was unchanged. That is the same silence `scale` had.
    for key in [
        "kind", "id", "name", "parent", "z", "layer", "tags", "script",
    ] {
        let (sim, complaints) = step(&format!(
            "function on_tick(self)\n\
             \x20 scene.find(\"/World/Hero\").{key} = \"x\"\n\
             end\n"
        ));
        assert!(
            complaints.contains("reserved key"),
            "{key} was shadowed silently: {complaints:?}"
        );
        // And nothing was stored under it.
        let state = sim.state();
        let id = state.scene.resolve_path("/World/Hero").expect("hero");
        let uid = state.scene.get(id).expect("hero").uid;
        assert!(
            state.vars.get(&uid).and_then(|v| v.get(key)).is_none(),
            "{key} left a variable behind"
        );
    }
}

#[test]
fn the_four_keys_the_handle_routes_are_still_writable() {
    // The control for the refusal above: it must not have caught the ones that
    // work. `scene` is the fifth reserved key with a runtime meaning and is
    // not a node property at all, so it stays refused.
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero.pos = vec2(5, 6)\n\
         \x20 hero.rot = \"90.0\"\n\
         \x20 hero.scale = vec2(2, 2)\n\
         \x20 hero.visible = false\n\
         end\n",
    );
    assert_eq!(complaints, "");
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    let node = state.scene.get(id).expect("hero");
    assert_eq!(node.transform.pos, Vec2Fx::from_ints(5, 6));
    assert_eq!(node.transform.scale, Vec2Fx::from_ints(2, 2));
    assert!(!node.visible);
}

#[test]
fn an_ordinary_variable_is_untouched_by_the_refusal() {
    // The other control: only the reserved list is refused, and a script's own
    // variables are exactly as free as they were.
    let (sim, complaints) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero.pending_damage = 7\n\
         \x20 hero.scaley = \"not a reserved key\"\n\
         \x20 self.read = tostring(hero.pending_damage)\n\
         end\n",
    );
    assert_eq!(complaints, "");
    assert_eq!(var(&sim, "read"), "7");
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    let uid = state.scene.get(id).expect("hero").uid;
    assert!(state.vars.get(&uid).and_then(|v| v.get("scaley")).is_some());
}
