//! Node kinds a project declares for itself.

use dimetric_scene::project_kinds::merge;
use dimetric_scene::{KindRegistry, PropertyType, Value};

/// Parse a scene against a registry, insisting it is clean.
fn parse_with(registry: &KindRegistry, text: &str) -> dimetric_scene::Scene {
    let out = dimetric_scene::parse(text, "test.dim", registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.expect("the scene parsed").scene
}

fn merged(text: &str) -> (KindRegistry, dimetric_core::Diagnostics) {
    let mut registry = KindRegistry::with_builtins();
    let diagnostics = merge(&mut registry, text, "kinds.toml");
    (registry, diagnostics)
}

const ENEMY: &str = r##"
[[kind]]
name = "Enemy"
extends = "Collider"
doc = "An enemy with authored stats."

[[kind.property]]
name = "max_health"
type = "int"
default = 40
doc = "Health at spawn."

[[kind.property]]
name = "speed"
type = "scalar"
default = 35.0
doc = "Walk speed."
"##;

#[test]
fn a_project_can_declare_a_kind_of_its_own() {
    let (registry, diagnostics) = merged(ENEMY);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let enemy = registry.get("Enemy").expect("the kind is registered");
    assert_eq!(enemy.doc, "An enemy with authored stats.");
    assert_eq!(
        enemy.property("max_health").map(|p| p.ty.clone()),
        Some(PropertyType::Int)
    );
    assert_eq!(
        enemy.property("max_health").and_then(|p| p.default.clone()),
        Some(Value::Int(40))
    );
}

#[test]
fn extends_brings_the_base_kinds_properties() {
    // A project kind is the engine's plus its own, not a replacement — so a
    // declared enemy is still a collider with a shape and a radius.
    let (registry, _) = merged(ENEMY);
    let enemy = registry.get("Enemy").unwrap();
    assert!(
        enemy.property("radius").is_some(),
        "inherited from Collider"
    );
    assert!(enemy.property("shape").is_some());
    assert!(enemy.property("max_health").is_some(), "and its own");
}

#[test]
fn a_declared_property_can_replace_an_inherited_one() {
    let text = format!(
        "{ENEMY}\n[[kind.property]]\nname = \"radius\"\ntype = \"scalar\"\ndefault = 12.0\ndoc = \"Bigger.\"\n"
    );
    let (registry, diagnostics) = merged(&text);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let enemy = registry.get("Enemy").unwrap();
    let radius: Vec<_> = enemy
        .properties
        .iter()
        .filter(|p| p.name == "radius")
        .collect();
    assert_eq!(radius.len(), 1, "one radius, not two");
    assert_eq!(
        radius[0].default,
        Some(Value::Scalar(dimetric_core::Fx::from_int(12)))
    );
}

#[test]
fn a_scene_can_use_a_declared_kind_and_set_its_properties() {
    let (registry, _) = merged(ENEMY);
    let scene = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Enemy"
name = "Brute"
max_health = 110
speed = 20.0
"##;
    let out = dimetric_scene::parse(scene, "room.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let doc = out.doc.unwrap();
    let node = doc.scene.get(doc.scene.root().unwrap()).unwrap();
    assert_eq!(node.get("max_health"), Some(&Value::Int(110)));
}

#[test]
fn a_declared_property_is_validated_like_any_other() {
    let (registry, _) = merged(ENEMY);
    let scene = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Enemy"
name = "Brute"
max_health = "lots"
"##;
    let out = dimetric_scene::parse(scene, "room.dim", &registry);
    assert!(out.diagnostics.has_errors(), "a string is not an int");
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == dimetric_core::Code::TYPE_MISMATCH));
}

#[test]
fn a_default_goes_through_the_same_exactness_rule_as_an_authored_value() {
    // 0.1 is not exactly representable, and a default that quietly rounded
    // would be a looser way into the scene than authoring the number.
    let text = r##"
[[kind]]
name = "Thing"
[[kind.property]]
name = "wobble"
type = "scalar"
default = 0.1
"##;
    let (_, diagnostics) = merged(text);
    assert!(diagnostics.has_errors());
    assert!(diagnostics
        .iter()
        .any(|d| d.code == dimetric_core::Code::NOT_REPRESENTABLE));
}

#[test]
fn extending_a_kind_that_does_not_exist_is_refused() {
    let text = "[[kind]]\nname = \"Enemy\"\nextends = \"Creature\"\n";
    let (registry, diagnostics) = merged(text);
    assert!(diagnostics.has_errors());
    assert!(diagnostics.to_string().contains("Creature"));
    assert!(
        registry.get("Enemy").is_none(),
        "and nothing was registered"
    );
}

#[test]
fn a_kind_that_shadows_a_reserved_key_is_refused() {
    let text = "[[kind]]\nname = \"Thing\"\n\n[[kind.property]]\nname = \"pos\"\ntype = \"vec2\"\n";
    let (_, diagnostics) = merged(text);
    assert!(diagnostics.has_errors());
    assert!(diagnostics
        .iter()
        .any(|d| d.code == dimetric_core::Code::RESERVED_KEY));
}

#[test]
fn a_property_that_is_both_required_and_defaulted_is_refused() {
    let text = "[[kind]]\nname = \"Thing\"\n\n[[kind.property]]\nname = \"x\"\ntype = \"int\"\nrequired = true\ndefault = 1\n";
    let (_, diagnostics) = merged(text);
    assert!(diagnostics.has_errors());
    assert!(diagnostics.to_string().contains("one or the other"));
}

#[test]
fn an_enum_property_declares_its_variants() {
    let text = "[[kind]]\nname = \"Thing\"\n\n[[kind.property]]\nname = \"mood\"\ntype = \"enum\"\nvariants = [\"calm\", \"angry\"]\ndefault = \"calm\"\n";
    let (registry, diagnostics) = merged(text);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let ty = registry
        .get("Thing")
        .unwrap()
        .property("mood")
        .unwrap()
        .ty
        .clone();
    assert_eq!(ty, PropertyType::Enum(vec!["calm".into(), "angry".into()]));
}

#[test]
fn a_malformed_kinds_file_reports_rather_than_panicking() {
    let (_, diagnostics) = merged("[[kind]\nname = ");
    assert!(diagnostics.has_errors());
}

#[test]
fn a_file_with_several_mistakes_reports_all_of_them() {
    // Three typos should name three typos, not the first one.
    let text = r##"
[[kind]]
name = "A"
extends = "Nope"

[[kind]]
name = "B"
[[kind.property]]
name = "x"
type = "notatype"

[[kind]]
name = "C"
[[kind.property]]
name = "y"
type = "int"
required = true
default = 3
"##;
    let (_, diagnostics) = merged(text);
    assert_eq!(diagnostics.iter().filter(|d| d.is_error()).count(), 3);
}

#[test]
fn an_instance_can_override_a_property_the_project_declared() {
    // The whole point of declaring a kind: the numbers that make a wraith a
    // wraith live in the scene, where an override can reach them.
    use dimetric_scene::instance::{resolve, SceneSource};
    use dimetric_scene::Scene;

    let (registry, diagnostics) = merged(ENEMY);
    assert!(!diagnostics.has_errors(), "{diagnostics}");

    let prefab = parse_with(
        &registry,
        r##"format = "dimetric"
version = 1

[scene]
root = "n_en_root0"

[[node]]
id = "n_en_root0"
kind = "Enemy"
name = "Enemy"
max_health = 40
"##,
    );

    struct One(Scene);
    impl SceneSource for One {
        fn load(&self, _: &dimetric_scene::Reference) -> Result<Scene, dimetric_core::Diagnostic> {
            Ok(self.0.clone())
        }
    }

    let room = parse_with(
        &registry,
        r##"format = "dimetric"
version = 1

[scene]
root = "n_room0000"

[[node]]
id = "n_room0000"
kind = "Node2D"
name = "Room"

[[node]]
id = "n_brute000"
kind = "Instance"
name = "Brute"
parent = "n_room0000"
scene = "scene:prefabs/enemy"

[[override]]
instance = "n_brute000"
target = "n_en_root0"
max_health = 110
"##,
    );

    let (resolved, diagnostics) = resolve(&room, &One(prefab), &registry);
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let brute = resolved
        .iter()
        .find(|(_, n)| n.kind == "Enemy")
        .map(|(_, n)| n)
        .expect("the instance brought the enemy in");
    assert_eq!(brute.get("max_health"), Some(&Value::Int(110)));
    assert_eq!(
        brute.base, "Collider",
        "and it still behaves as what it extends"
    );
}
