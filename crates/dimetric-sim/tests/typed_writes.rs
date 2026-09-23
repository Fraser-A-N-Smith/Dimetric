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
//!
//! # What the guard is not
//!
//! Four of these types reach a script *as a string*, because a string is what
//! they are written as: a colour is `#rrggbbaa`, an angle is degrees, a
//! reference is `asset:sprites/hero`, an enum is its variant. So the first
//! version of this guard refused `node:set(k, node:get(k))` — `get` and `set`
//! were not symmetric — and, with no way to construct a colour in Lua either,
//! `modulate` could be read and could not be written at all.
//!
//! A string over one of those four is now re-read as the property's own type,
//! by the same parser the scene format uses. That widens what counts as
//! writing the same type; it does not widen what counts as a type.

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

fn run(source: &str) -> String {
    step(source).1
}

/// What the hero's `modulate` ended up as, in state.
fn modulate(sim: &Sim) -> dimetric_scene::Value {
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    state
        .scene
        .get(id)
        .expect("hero")
        .get("modulate")
        .expect("modulate")
        .clone()
}

/// A script that writes `expr` into the hero's `modulate`.
fn tint(expr: &str) -> String {
    format!(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"modulate\", {expr})\n\
         end\n"
    )
}

#[test]
fn a_script_can_construct_a_colour() {
    // The gap this closes. `modulate` is on every Sprite2D, `get` hands it
    // back as `#rrggbbaa`, and until now there was no way to spell one: the
    // sandbox had a `vec2` constructor and no `color` one, so the read gave
    // you a string the write would not take.
    let (sim, out) = step(&tint("color.rgba(255, 143, 74, 255)"));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        modulate(&sim),
        dimetric_scene::Value::Color(dimetric_scene::Color::rgba(255, 143, 74, 255))
    );
}

#[test]
fn a_colour_can_be_written_back_as_the_text_it_was_read_as() {
    // `node:set(k, node:get(k))` has to work, on every type. It did not.
    let (sim, out) = step(&tint("\"#ff00ffff\""));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        modulate(&sim),
        dimetric_scene::Value::Color(dimetric_scene::Color::rgba(255, 0, 255, 255)),
        "the string landed as a colour, not as a string"
    );
}

#[test]
fn a_round_trip_through_get_and_set_keeps_the_value() {
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"modulate\", hero:get(\"modulate\"))\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(
        modulate(&sim),
        dimetric_scene::Value::Color(dimetric_scene::Color::WHITE)
    );
}

#[test]
fn rgb_means_opaque_and_says_so() {
    let (sim, out) = step(&tint("color.rgb(0, 128, 255)"));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        modulate(&sim),
        dimetric_scene::Value::Color(dimetric_scene::Color::rgba(0, 128, 255, 255))
    );
}

#[test]
fn a_string_that_is_not_a_colour_is_refused_with_the_reason() {
    // Widening what counts as writing a colour must not widen what counts as
    // a colour. `#ff8f4a` is refused here exactly as it is in a `.dim` file:
    // alpha is explicit in this engine, and `color.rgb` says "opaque" aloud.
    let out = run(&tint("\"#ff8f4a\""));
    assert!(out.contains("color"), "{out}");
    assert!(out.contains("#ff8f4a"), "the text is quoted back: {out}");
    assert!(out.contains("8 hex digits"), "{out}");
}

#[test]
fn a_number_written_into_a_colour_is_still_a_type_change() {
    let out = run(&tint("7"));
    assert!(out.contains("cannot change a property's type"), "{out}");
    assert!(out.contains("color"), "{out}");
}

#[test]
fn a_channel_outside_a_byte_is_refused_rather_than_clamped() {
    // Clamping would make a colour the author did not write and would not be
    // told about, and the arithmetic that produced the 300 would still be
    // there afterwards.
    let out = run(&tint("color.rgba(255, 300, 0, 255)"));
    assert!(out.contains("0..255"), "{out}");
}

#[test]
fn writing_the_right_type_is_fine() {
    // `flip_h` is a bool kind property and a Lua boolean is one, so this is
    // the same write done correctly and must stay silent.
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"flip_h\", true)\n\
         end\n");
    assert_eq!(out, "", "{out}");
}

#[test]
fn setting_a_reserved_key_through_set_is_refused() {
    // `get` and `set` address the property map; `visible` is not in it, it is
    // on the node. So this used to write a prop nothing reads and leave the
    // node on screen — a silent no-op, at the line that meant to hide it.
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"visible\", false)\n\
         end\n");
    assert!(out.contains("reserved"), "{out}");
    assert!(
        out.contains("self.visible"),
        "it names what to write: {out}"
    );
}

#[test]
fn a_reserved_key_written_through_set_cannot_corrupt_a_save() {
    // The worse half of the same silence. `set("rot", "45")` put a string
    // where the scene writer would emit `rot = "45"`, which the scene parser
    // refuses as `rot should be a angle` — a save written without complaint
    // that could not be loaded. Found while closing the colour gap, because
    // it is the same defect one key over.
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"rot\", \"45\")\n\
         end\n");
    assert!(out.contains("reserved"), "{out}");
}

#[test]
fn the_reserved_keys_are_still_writable_where_they_live() {
    // Refusing the shadow must not take away the real thing.
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 self.visible = false\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    let state = sim.state();
    let id = state.scene.resolve_path("/World").expect("world");
    assert!(!state.scene.get(id).expect("world").visible);
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

#[test]
fn a_reference_can_be_written_back_as_the_text_it_was_read_as() {
    // Not a colour, and the same defect. `texture` is a reference and reaches
    // a script as `asset:sprites/hero`, because that is how it is written — so
    // `set(k, get(k))` on it used to be a type change too. Colour is what got
    // noticed because colour is what was needed; the fix is on the shape
    // rather than on the one type that reported it.
    let (sim, out) = step(
        "function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"texture\", \"asset:sprites/other\")\n\
         end\n",
    );
    assert_eq!(out, "", "{out}");
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Hero").expect("hero");
    let texture = state.scene.get(id).expect("hero").get("texture").cloned();
    assert!(
        matches!(texture, Some(dimetric_scene::Value::Ref(_))),
        "landed as {texture:?}, not a reference"
    );
}

#[test]
fn text_that_is_not_a_reference_is_refused_with_the_reason() {
    let out = run("function on_tick(self)\n\
         \x20 local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"texture\", \"sprites/other\")\n\
         end\n");
    assert!(out.contains("reference"), "{out}");
    assert!(out.contains("sprites/other"), "{out}");
}
