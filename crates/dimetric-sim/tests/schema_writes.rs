//! A write to a property the node carries no value for.
//!
//! `node:set` checks a write against the value the node already holds, because
//! that value came through the parser and the parser had the schema. Canonical
//! form omits a property equal to its default and the parser fills those back
//! in, so the only properties absent from a loaded node are the ones with **no
//! default and not required** — `region` on a sprite, `limits` on a camera,
//! `cone_angle` on a light, `tile_size` on a tile layer, and any such property
//! of a project-declared kind. A required property is never absent, which is
//! why the reference and enum cases below need a declared kind to reach.
//!
//! There was nothing to check those against, so a write to one landed in state
//! exactly as the scripting boundary made it: `set("region", { 0, 0, 24, 3 })`
//! stored a *list of integers* where a rect belonged. The renderer coped, so
//! nothing on screen said so. A save of that state wrote `region = [0, 0, 24,
//! 3]` — integers, where an authored rect is `[0.0, 0.0, 24.0, 3.0]` — the
//! loader typed it from the schema, and the reloaded state hashed differently
//! from the live state it came from. Written as the string `"[0, 0, 24, 3]"`
//! instead, the save did not load at all.
//!
//! So the kind's declaration is the type of record when the node has no value.
//! The simulation does not carry a `KindRegistry` in [`SimState`] — a registry
//! is unchanging project data, not state, and belongs beside the fonts and the
//! module cache — so the host hands one in and `node:set` reads it from there.

use dimetric_scene::schema::{NodeKindSchema, PropertySchema, PropertyType};
use dimetric_scene::{KindRegistry, Scene, Value};
use dimetric_sim::{input::InputFrame, LuaHost, Sim, SimConfig};

/// A sprite with no `region`, a camera with no `limits`, a light with no
/// `cone_angle`: three properties with no default, left out as an author would
/// leave them out.
const SCENE: &str = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
     [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
     script = \"script:main.lua\"\n\n\
     [[node]]\nid = \"n_hero0000\"\nkind = \"Sprite2D\"\nname = \"Hero\"\n\
     parent = \"n_root0000\"\ntexture = \"asset:sprites/hero\"\n\n\
     [[node]]\nid = \"n_view0000\"\nkind = \"Camera2D\"\nname = \"View\"\n\
     parent = \"n_root0000\"\n\n\
     [[node]]\nid = \"n_lamp0000\"\nkind = \"Light2D\"\nname = \"Lamp\"\n\
     parent = \"n_root0000\"\nshape = \"Cone\"\n";

fn scene_from(text: &str, registry: &KindRegistry) -> Scene {
    let out = dimetric_scene::parse(text, "f.dim", registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.expect("a document").scene
}

/// Run one tick of `source` against `SCENE`, and return the simulation and
/// whatever it complained about.
fn step(source: &str) -> (Sim, String) {
    step_with(source, SCENE, &KindRegistry::with_builtins())
}

fn step_with(source: &str, text: &str, registry: &KindRegistry) -> (Sim, String) {
    let mut host = LuaHost::new(60).expect("host");
    host.set_kinds(registry.clone());
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "script did not load: {load:?}");
    let mut sim = Sim::new(
        scene_from(text, registry),
        1,
        Box::new(host),
        SimConfig::default(),
    );
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

/// A script body that runs once, on ready.
fn once(body: &str) -> String {
    format!("function on_ready(self)\n{body}\nend\n")
}

/// What a node's property ended up as, in state.
fn prop(sim: &Sim, path: &str, key: &str) -> Option<Value> {
    let state = sim.state();
    let id = state.scene.resolve_path(path)?;
    state.scene.get(id)?.get(key).cloned()
}

/// Save the scene the way the host does, load it back, and hash both.
fn round_trips(sim: &Sim) -> Result<(), String> {
    let state = sim.state();
    let registry = KindRegistry::with_builtins();
    let text = dimetric_scene::write::to_canonical_text(&state.scene, &registry, None);
    let out = dimetric_scene::parse(&text, "saved.dim", &registry);
    if out.diagnostics.has_errors() {
        return Err(format!(
            "the save did not load:\n{}\n{text}",
            out.diagnostics
        ));
    }
    let hash = |scene: &Scene| {
        let mut h = dimetric_core::StateHasher::new();
        scene.hash_state(&mut h);
        h.finish()
    };
    let live = hash(&state.scene);
    let back = hash(&out.doc.expect("a document").scene);
    match live == back {
        true => Ok(()),
        false => Err(format!("{live} live, {back} reloaded, from:\n{text}")),
    }
}

#[test]
fn a_rect_written_as_a_list_of_integers_is_a_rect() {
    // The reported case, exactly: a sprite authored without a `region` because
    // the whole texture was wanted, and a script narrowing it later.
    let (sim, out) = step(&once(
        "  scene.find(\"/World/Hero\"):set(\"region\", { 0, 0, 24, 3 })",
    ));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Hero", "region").map(|v| v.type_name()),
        Some("rect")
    );
    round_trips(&sim).expect("a save of this state round-trips");
}

#[test]
fn a_rect_written_as_a_table_of_pos_and_size_is_a_rect() {
    // The shape `get` hands back, so `set(k, get(k))` works on a property the
    // node did not author once something has written one.
    let (sim, out) = step(&once(
        "  local view = scene.find(\"/World/View\")\n\
         \x20 view:set(\"limits\", { pos = vec2(-8, -8), size = vec2(48, 24) })\n\
         \x20 view:set(\"limits\", view:get(\"limits\"))",
    ));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/View", "limits"),
        Some(Value::Rect(dimetric_core::Rect::new(
            dimetric_core::Vec2Fx::from_ints(-8, -8),
            dimetric_core::Vec2Fx::from_ints(48, 24),
        )))
    );
    round_trips(&sim).expect("a save of this state round-trips");
}

#[test]
fn a_rect_written_as_its_own_text_is_a_rect() {
    // The spelling that used not merely to hash differently but to produce a
    // save that would not open: `DIM0201 region should be a rect, found a
    // string`.
    let (sim, out) = step(&once(
        "  scene.find(\"/World/Hero\"):set(\"region\", \"[1, 2, 96, 48]\")",
    ));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Hero", "region").map(|v| v.type_name()),
        Some("rect")
    );
    round_trips(&sim).expect("a save of this state round-trips");
}

#[test]
fn an_angle_written_as_text_is_an_angle() {
    // Degrees in text is an angle's written form, and `cone_angle` has no
    // default, so a cone light that did not author one took the string.
    let (sim, out) = step(&once(
        "  scene.find(\"/World/Lamp\"):set(\"cone_angle\", \"30.0\")",
    ));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Lamp", "cone_angle").map(|v| v.type_name()),
        Some("angle")
    );
    round_trips(&sim).expect("a save of this state round-trips");
}

#[test]
fn a_rect_written_as_the_wrong_number_of_numbers_says_so() {
    let out = run(&once(
        "  scene.find(\"/World/Hero\"):set(\"region\", { 0, 0, 24 })",
    ));
    assert!(out.contains("DIM0505"), "{out}");
    assert!(out.contains("a rect is four numbers"), "{out}");
}

#[test]
fn a_write_of_the_wrong_type_entirely_is_refused() {
    // Nothing authored, and a boolean is not a spelling of a rect.
    let out = run(&once("  scene.find(\"/World/Hero\"):set(\"region\", true)"));
    assert!(out.contains("DIM0505"), "{out}");
    assert!(out.contains("cannot change a property's type"), "{out}");
    assert!(out.contains("is a rect"), "{out}");
}

#[test]
fn the_declared_type_is_only_consulted_when_the_node_has_no_value() {
    // The authored value stays the type of record: it is the schema's answer
    // *and* the author's, and a write that passed against it passed before any
    // of this existed. `modulate` has a default, so every sprite carries one.
    let (sim, out) = step(&once(
        "  local hero = scene.find(\"/World/Hero\")\n\
         \x20 hero:set(\"modulate\", hero:get(\"modulate\"))",
    ));
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Hero", "modulate").map(|v| v.type_name()),
        Some("color")
    );
}

#[test]
fn a_write_the_declared_type_takes_is_a_write_the_authored_value_would_take() {
    // The two routes have to agree, or a property behaves differently
    // depending on whether anyone has written it yet. Same script, run once
    // against a sprite with no `region` and once against one that authored
    // one: the value in state is the same either way.
    let script = once("  scene.find(\"/World/Hero\"):set(\"region\", \"[1, 2, 96, 48]\")");
    let (absent, out) = step(&script);
    assert_eq!(out, "", "{out}");
    let authored_scene = SCENE.replace(
        "texture = \"asset:sprites/hero\"\n",
        "texture = \"asset:sprites/hero\"\nregion = [0.0, 0.0, 16.0, 16.0]\n",
    );
    let (authored, out) = step_with(&script, &authored_scene, &KindRegistry::with_builtins());
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&absent, "/World/Hero", "region"),
        prop(&authored, "/World/Hero", "region")
    );
}

/// A project-declared kind with two no-default properties, to check that a
/// kind the engine never heard of is typed the same way a built-in is.
fn project_registry() -> KindRegistry {
    let mut registry = KindRegistry::with_builtins();
    registry
        .register(
            NodeKindSchema::new(
                "Marker",
                "A test kind with properties that have no default.",
                vec![
                    PropertySchema::new(
                        "facing",
                        PropertyType::Enum(vec!["North".into(), "South".into()]),
                        None,
                        "Which way it points.",
                    ),
                    PropertySchema::new("area", PropertyType::Rect, None, "Where it applies."),
                    PropertySchema::new(
                        "banner",
                        PropertyType::AssetRef,
                        None,
                        "Art to draw over it.",
                    ),
                    PropertySchema::new(
                        "span",
                        PropertyType::Vec2i,
                        None,
                        "How many cells it covers.",
                    ),
                ],
            )
            .based_on("Node2D"),
        )
        .expect("the kind registers");
    registry
}

const PROJECT_SCENE: &str = "format = \"dimetric\"\nversion = 1\n\n[scene]\n\
     root = \"n_root0000\"\n\n\
     [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
     script = \"script:main.lua\"\n\n\
     [[node]]\nid = \"n_mark0000\"\nkind = \"Marker\"\nname = \"Mark\"\n\
     parent = \"n_root0000\"\n";

#[test]
fn a_project_declared_kinds_properties_are_typed_too() {
    let (sim, out) = step_with(
        &once(
            "  local mark = scene.find(\"/World/Mark\")\n\
             \x20 mark:set(\"area\", { 0, 0, 12, 12 })\n\
             \x20 mark:set(\"facing\", \"South\")",
        ),
        PROJECT_SCENE,
        &project_registry(),
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Mark", "area").map(|v| v.type_name()),
        Some("rect")
    );
    assert_eq!(
        prop(&sim, "/World/Mark", "facing"),
        Some(Value::Enum("South".into()))
    );
}

#[test]
fn an_enum_variant_the_kind_does_not_have_is_refused() {
    // Only the schema knows the variant set — the value-driven guard says so
    // in its own documentation — and a variant the schema rejects is `DIM0202`
    // at load, so the save would not open.
    let out = step_with(
        &once("  scene.find(\"/World/Mark\"):set(\"facing\", \"Sideways\")"),
        PROJECT_SCENE,
        &project_registry(),
    )
    .1;
    assert!(out.contains("DIM0505"), "{out}");
    assert!(out.contains("not one of North, South"), "{out}");
}

#[test]
fn a_kind_this_host_was_never_told_about_is_left_alone() {
    // No registry entry, nothing to check against, and guessing would be worse
    // than the gap this leaves. `Project::script_host` is what stops a real
    // project ever being in this position; a host told only the built-ins is.
    let out = step_with(
        &once("  scene.find(\"/World/Mark\"):set(\"area\", { 0, 0, 12, 12 })"),
        PROJECT_SCENE,
        &project_registry(),
    );
    assert_eq!(out.1, "", "{}", out.1);

    // The same write, on a host that knows only the built-ins.
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([(
        "main.lua",
        once("  scene.find(\"/World/Mark\"):set(\"area\", { 0, 0, 12, 12 })").as_str(),
    )]);
    assert!(load.is_empty(), "{load:?}");
    let mut sim = Sim::new(
        scene_from(PROJECT_SCENE, &project_registry()),
        1,
        Box::new(host),
        SimConfig::default(),
    );
    sim.step(InputFrame::idle(1));
    assert!(sim.take_diagnostics().0.is_empty());
    assert_eq!(
        prop(&sim, "/World/Mark", "area").map(|v| v.type_name()),
        Some("list"),
        "with no schema there is nothing to type it with"
    );
}

#[test]
fn a_reference_written_as_text_is_a_reference() {
    // Every built-in reference property with no default is also *required*, so
    // this case needs a declared kind to reach at all. It is reachable, and a
    // string stored where a reference belongs is a save that will not open.
    let (sim, out) = step_with(
        &once("  scene.find(\"/World/Mark\"):set(\"banner\", \"asset:sprites/flag\")"),
        PROJECT_SCENE,
        &project_registry(),
    );
    assert_eq!(out, "", "{out}");
    assert_eq!(
        prop(&sim, "/World/Mark", "banner").map(|v| v.type_name()),
        Some("reference")
    );
}

#[test]
fn a_reference_with_the_wrong_prefix_is_refused() {
    // The four reference types share one `Value` variant, so the variant
    // cannot tell them apart and only the schema can. Stored, this is
    // `DIM0401` at load — a save that will not open, not one that hashes
    // differently.
    let out = step_with(
        &once("  scene.find(\"/World/Mark\"):set(\"banner\", \"scene:prefabs/flag\")"),
        PROJECT_SCENE,
        &project_registry(),
    )
    .1;
    assert!(out.contains("DIM0505"), "{out}");
    assert!(out.contains("asset: reference is expected"), "{out}");
}

#[test]
fn a_vec2_written_over_a_vec2i_is_refused_the_way_an_authored_one_would_be() {
    // A grid coordinate is not a vector of scalars, and a script has no way to
    // build one — `vec2` is fixed-point. This refuses rather than widening the
    // type guard, which keeps the declared type and the authored value saying
    // the same thing: the same write over an authored `span` is refused too.
    //
    // The gap is real and it is narrow: `tile_size` on a `TileLayer` is the
    // only built-in property it covers, and a layer that wants one can author
    // it. Worth reporting rather than fixing by making `{16, 16}` mean a
    // vec2i, which would make a list of two integers ambiguous.
    let out = step_with(
        &once("  scene.find(\"/World/Mark\"):set(\"span\", vec2(3, 4))"),
        PROJECT_SCENE,
        &project_registry(),
    )
    .1;
    assert!(out.contains("DIM0505"), "{out}");
    assert!(out.contains("is a vec2i"), "{out}");
}
