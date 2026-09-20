//! A script may not change a node property's type.
//!
//! `node:set` has no schema to consult — `from_lua` turns a Lua string into a
//! `Value::Str` whatever the property is declared as. That used to succeed,
//! and cost twice over:
//!
//!   * the renderer read the property back with its typed accessor, got
//!     nothing, and silently stopped drawing the node;
//!   * a save wrote the string, the loader typed it correctly from the schema,
//!     and the restored state hashed **differently from the live state it came
//!     from** — a save that did not round-trip, with nothing saying so.
//!
//! The authored value is the type of record, because it came through the
//! parser, which did have the schema.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

fn scene() -> Scene {
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
         script = \"script:main.lua\"\n\n\
         [[node]]\nid = \"n_hero0000\"\nkind = \"Sprite2D\"\nname = \"Hero\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/hero\"\n\
         modulate = \"#ffffffff\"\n";
    let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn run(source: &str) -> String {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "script did not load: {load:?}");
    let mut sim = Sim::new(scene(), 1, Box::new(host), SimConfig::default());
    sim.step(InputFrame::idle(1));
    sim.take_diagnostics()
        .0
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn writing_a_string_into_a_colour_is_refused() {
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"modulate\", \"#ff00ffff\")\n\
         end\n");
    assert!(out.contains("color"), "{out}");
    assert!(out.contains("string"), "{out}");
}

#[test]
fn writing_the_right_type_is_fine() {
    // `visible` is a bool and a Lua boolean is one, so this is the same write
    // done correctly and must stay silent.
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"visible\", false)\n\
         end\n");
    assert_eq!(out, "", "{out}");
}

#[test]
fn a_property_the_scene_never_authored_is_left_alone() {
    // Nothing to compare against, so nothing is refused. The check uses the
    // authored value as the type of record and has no opinion where there is
    // none — it is a guard against *changing* a type, not a schema.
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"something_unauthored\", \"anything\")\n\
         end\n");
    assert_eq!(out, "", "{out}");
}
