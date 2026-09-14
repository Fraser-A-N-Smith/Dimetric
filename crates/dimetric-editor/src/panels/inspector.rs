//! The inspector: one node's properties, from its kind's schema.
//!
//! Driven by the schema rather than by a hand-written form per kind. A kind
//! gains a property and the inspector shows it, with its documentation as the
//! tooltip and its default as the placeholder — which is the difference between
//! a schema that describes the engine and a schema that has to be kept in sync
//! with three other places.

use dimetric_core::NodeUid;
use dimetric_scene::Value;

use crate::editor::Editor;

/// One editable field.
#[derive(Clone, PartialEq, Debug)]
pub struct InspectorRow {
    /// Property name, or a reserved key such as `pos`.
    pub key: String,
    /// The schema's type name, for picking a widget.
    pub type_name: String,
    /// Current value as the literal text a field should hold.
    pub literal: String,
    /// Whether the value is the schema's default, so a client can grey it.
    pub is_default: bool,
    /// Whether the scene file spells this property out.
    pub explicit: bool,
    /// Whether the schema requires it.
    pub required: bool,
    /// Documentation, for a tooltip.
    pub doc: String,
}

/// Reserved keys the inspector shows above the kind's own properties.
///
/// Every node has these, so they are not in any kind's schema — but a node
/// inspector that could not move a node would be a strange thing.
const TRANSFORM_KEYS: &[(&str, &str, &str)] = &[
    ("pos", "vec2", "Position, relative to the parent."),
    ("rot", "angle", "Rotation in degrees."),
    ("scale", "vec2", "Scale, relative to the parent."),
    ("visible", "bool", "Drawn, along with its children."),
    ("z", "int", "Draw order within the layer."),
    ("layer", "int", "Render layer."),
];

/// The fields an inspector shows for a node.
pub fn inspector_rows(editor: &Editor, node: NodeUid) -> Vec<InspectorRow> {
    let Some(doc) = editor.project.open.as_ref() else {
        return Vec::new();
    };
    let Some(n) = doc.scene.by_uid(node).and_then(|id| doc.scene.get(id)) else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for (key, type_name, help) in TRANSFORM_KEYS {
        let (literal, is_default) = match *key {
            "pos" => transform_vec(n.transform.pos, dimetric_core::Vec2Fx::ZERO),
            "scale" => transform_vec(n.transform.scale, dimetric_core::Vec2Fx::ONE),
            "rot" => (
                n.transform.rot.to_degrees_string(),
                n.transform.rot == dimetric_core::Angle::ZERO,
            ),
            "visible" => (n.visible.to_string(), n.visible),
            "z" => (n.z.to_string(), n.z == 0),
            "layer" => (n.layer.to_string(), n.layer == 0),
            _ => continue,
        };
        rows.push(InspectorRow {
            key: key.to_string(),
            type_name: type_name.to_string(),
            literal,
            is_default,
            explicit: !is_default,
            required: false,
            doc: help.to_string(),
        });
    }

    // Then the kind's own properties, in the order the schema declares them,
    // which is the order somebody chose rather than alphabetical order.
    if let Some(schema) = editor.project.registry.get(&n.kind) {
        for property in &schema.properties {
            let current = n.get(&property.name);
            let is_default = match (&property.default, current) {
                (Some(default), Some(value)) => default == value,
                (None, None) => true,
                _ => false,
            };
            rows.push(InspectorRow {
                key: property.name.clone(),
                type_name: property.ty.name(),
                literal: current.map(render).unwrap_or_default(),
                is_default,
                explicit: current.is_some() && !is_default,
                required: property.required,
                doc: property.doc.clone(),
            });
        }
    }
    rows
}

fn transform_vec(value: dimetric_core::Vec2Fx, default: dimetric_core::Vec2Fx) -> (String, bool) {
    (
        format!(
            "[{}, {}]",
            value.x.to_exact_string(),
            value.y.to_exact_string()
        ),
        value == default,
    )
}

/// A value as the literal text a field holds, which is what
/// [`crate::Action::SetProperty`] takes back.
///
/// Strings, enum variants and references come back bare rather than quoted:
/// a text field holds `sprites/hero`, and the quoting is the scene file's
/// business rather than the widget's.
fn render(value: &Value) -> String {
    match value {
        Value::Str(s) | Value::Enum(s) => s.clone(),
        Value::Ref(r) => r.to_text(),
        other => dimetric_scene::write::render(other),
    }
}
