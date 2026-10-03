//! A declared type can decide a write, and decides it the same way an authored
//! value does.
//!
//! `Value::reshape` answers "may this be written over that?" using an authored
//! value as the type of record. A property with no default is absent from a
//! node that did not author one, so there is no such value — and before
//! `PropertyType::reshape` there was no check at all, which is how a list of
//! integers came to be stored where a rect belonged.
//!
//! The two must agree, or a property behaves differently depending on whether
//! anything has written it yet. `PropertyType::witness` is what makes that
//! structural rather than hopeful: there is one set of rules, in `Value`, and
//! the schema hands it a stand-in of the declared type.

use dimetric_core::{Angle, Fx, Rect, Vec2Fx};
use dimetric_scene::schema::PropertyType;
use dimetric_scene::{Color, KindRegistry, Mismatch, Reference, Value};

/// Every `PropertyType`, once.
///
/// `witness` matches exhaustively, so the compiler already refuses a variant it
/// does not handle. What this list adds is that each one is handled
/// *correctly* — a witness of the wrong variant would compile and would quietly
/// accept the wrong writes.
fn every_type() -> Vec<(PropertyType, &'static str)> {
    vec![
        (PropertyType::Scalar, "scalar"),
        (PropertyType::Int, "int"),
        (PropertyType::Bool, "bool"),
        (PropertyType::Str, "string"),
        (PropertyType::Vec2, "vec2"),
        (PropertyType::Vec2i, "vec2i"),
        (PropertyType::Rect, "rect"),
        (PropertyType::Angle, "angle"),
        (PropertyType::Color, "color"),
        (PropertyType::Enum(vec!["One".into(), "Two".into()]), "enum"),
        (PropertyType::AssetRef, "reference"),
        (PropertyType::SceneRef, "reference"),
        (PropertyType::NodeRef, "reference"),
        (PropertyType::ScriptRef, "reference"),
        (PropertyType::List(Box::new(PropertyType::Vec2)), "list"),
        (PropertyType::Map(Box::new(PropertyType::Int)), "map"),
    ]
}

/// A spread of incoming values, covering each written form a script can hand
/// over and a few that are nothing of the sort.
fn every_incoming() -> Vec<Value> {
    vec![
        Value::Scalar(Fx::from_int(3)),
        Value::Int(3),
        Value::Bool(true),
        Value::Str("One".into()),
        Value::Str("#102030c0".into()),
        Value::Str("45.0".into()),
        Value::Str("asset:sprites/hero".into()),
        Value::Str("[1, 2, 3, 4]".into()),
        Value::Str("nonsense".into()),
        Value::Vec2(Vec2Fx::from_ints(1, 2)),
        Value::Vec2i([1, 2]),
        Value::Rect(Rect::new(Vec2Fx::ZERO, Vec2Fx::from_ints(4, 4))),
        Value::Angle(Angle::ZERO),
        Value::Color(Color::WHITE),
        Value::Enum("One".into()),
        Value::Ref(Reference::Asset("sprites/hero".into())),
        Value::List(vec![Value::Int(1), Value::Int(2)]),
        Value::List(vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(8),
            Value::Int(4),
        ]),
        Value::Map(Default::default()),
    ]
}

#[test]
fn a_witness_has_the_type_it_stands_for() {
    for (ty, name) in every_type() {
        assert_eq!(
            ty.witness().type_name(),
            name,
            "{}'s witness is the wrong type",
            ty.name()
        );
    }
}

#[test]
fn a_declared_type_reshapes_the_way_a_value_of_it_does() {
    // The one set of rules, reached from both sides. Where the schema is
    // stricter it is stricter about *content* — a variant or a prefix — and
    // never about the type, so the two never disagree on whether a write is a
    // type change.
    for (ty, _) in every_type() {
        let witness = ty.witness();
        for incoming in every_incoming() {
            let by_schema = ty.reshape(incoming.clone());
            let by_value = witness.reshape(incoming.clone());
            let same_verdict = match (&by_schema, &by_value) {
                (Ok(_), Ok(_)) => true,
                (Err(Mismatch::Type), Err(Mismatch::Type)) => true,
                (Err(Mismatch::Content(_)), Err(Mismatch::Content(_))) => true,
                // The schema refusing content the value accepted is the whole
                // point of the two extra checks; the reverse would be a bug.
                (Err(Mismatch::Content(_)), Ok(_)) => true,
                _ => false,
            };
            assert!(
                same_verdict,
                "{} and a {} disagree about {incoming}: schema {by_schema:?}, value {by_value:?}",
                ty.name(),
                witness.type_name(),
            );
        }
    }
}

#[test]
fn a_declared_type_never_reshapes_into_another_type() {
    // Whatever comes back is the declared type. A reshape that returned
    // something else would put the wrong value in state with no diagnostic,
    // which is the defect this whole mechanism exists to stop.
    for (ty, name) in every_type() {
        for incoming in every_incoming() {
            if let Ok(value) = ty.reshape(incoming.clone()) {
                assert_eq!(
                    value.type_name(),
                    name,
                    "{} turned {incoming} into a {}",
                    ty.name(),
                    value.type_name()
                );
            }
        }
    }
}

#[test]
fn an_enum_only_takes_its_own_variants() {
    let ty = PropertyType::Enum(vec!["Radial".into(), "Cone".into()]);
    assert_eq!(
        ty.reshape(Value::Str("Cone".into())),
        Ok(Value::Enum("Cone".into()))
    );
    assert!(matches!(
        ty.reshape(Value::Str("Square".into())),
        Err(Mismatch::Content(_))
    ));
}

#[test]
fn a_reference_property_only_takes_its_own_prefix() {
    assert_eq!(
        PropertyType::AssetRef.reshape(Value::Str("asset:sprites/hero".into())),
        Ok(Value::Ref(Reference::Asset("sprites/hero".into())))
    );
    assert!(matches!(
        PropertyType::AssetRef.reshape(Value::Str("scene:prefabs/hero".into())),
        Err(Mismatch::Content(_))
    ));
    assert!(matches!(
        PropertyType::SceneRef.reshape(Value::Str("asset:sprites/hero".into())),
        Err(Mismatch::Content(_))
    ));
}

#[test]
fn every_built_in_default_is_the_type_its_witness_is() {
    // Driven by the real schemas rather than by a list here: a default came
    // from the kind's own declaration, so if a witness and a default disagree
    // about a type then one of the two is wrong about the property.
    let registry = KindRegistry::with_builtins();
    let mut checked = 0;
    for kind in registry.iter() {
        for prop in &kind.properties {
            let Some(default) = &prop.default else {
                continue;
            };
            assert_eq!(
                prop.ty.witness().type_name(),
                default.type_name(),
                "{}.{} is declared {} and defaults to a {}",
                kind.kind,
                prop.name,
                prop.ty.name(),
                default.type_name()
            );
            checked += 1;
        }
    }
    assert!(checked > 40, "only {checked} properties have defaults");
}
