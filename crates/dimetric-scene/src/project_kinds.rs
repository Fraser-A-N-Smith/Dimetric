//! Node kinds a project declares for itself.
//!
//! The built-in kinds describe what the engine can draw and simulate. They
//! cannot describe what makes a wraith a wraith, because the engine has never
//! heard of a wraith — and a project that cannot say so has nowhere to put the
//! numbers an instance override is supposed to change. The stats end up in a
//! script instead, which is not where a designer looks for them.
//!
//! A `kinds.toml` in the project root fixes that:
//!
//! ```toml
//! [[kind]]
//! name = "Enemy"
//! extends = "Collider"
//! doc = "An enemy with authored stats."
//!
//! [[kind.property]]
//! name = "max_health"
//! type = "int"
//! default = 40
//! doc = "Health at spawn."
//! ```
//!
//! `extends` copies the base kind's properties first, so a project kind is the
//! engine's plus its own rather than a replacement. Everything else about it —
//! validation, canonical form, the generated documentation, the inspector — is
//! the same machinery the built-ins go through, because it *is* the same
//! machinery: this module only produces [`NodeKindSchema`]s.

use dimetric_core::{Code, Diagnostic, Diagnostics};

use crate::schema::{KindRegistry, NodeKindSchema, PropertySchema, PropertyType};

/// Name of the file a project declares its kinds in.
pub const KINDS_FILE: &str = "kinds.toml";

/// Read project kinds from TOML and add them to a registry.
///
/// Reports everything wrong rather than stopping at the first problem: a file
/// with three typos should name three typos.
pub fn merge(registry: &mut KindRegistry, text: &str, source: &str) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(doc) => doc,
        Err(e) => {
            diagnostics.push(
                Diagnostic::new(Code::PARSE_FAILED, e.to_string())
                    .with_span(dimetric_core::Span::file(source.to_string())),
            );
            return diagnostics;
        }
    };

    let Some(kinds) = doc.get("kind").and_then(|i| i.as_array_of_tables()) else {
        return diagnostics;
    };

    for table in kinds.iter() {
        let Some(name) = table.get("name").and_then(|i| i.as_str()) else {
            diagnostics.push(Diagnostic::new(
                Code::PARSE_FAILED,
                "a [[kind]] needs a `name`",
            ));
            continue;
        };

        // `extends` starts from the base kind's properties, so a project kind
        // is the engine's plus its own. Naming a kind that does not exist is an
        // error rather than an empty base: it is almost always a typo, and
        // silently producing a kind with no transform would be baffling.
        let mut properties = Vec::new();
        let mut behaves_as = name.to_string();
        if let Some(base) = table.get("extends").and_then(|i| i.as_str()) {
            match registry.get(base) {
                Some(schema) => {
                    properties.extend(schema.properties.iter().cloned());
                    // The base of the base, so a chain of project kinds still
                    // arrives at the built-in that decides how a node behaves.
                    behaves_as = schema.base.clone();
                }
                None => {
                    diagnostics.push(
                        Diagnostic::new(
                            Code::UNKNOWN_KIND,
                            format!("node kind {name:?} extends {base:?}, which is not registered"),
                        )
                        .with_field("kind", name.to_string())
                        .with_field("extends", base.to_string()),
                    );
                    continue;
                }
            }
        }

        if let Some(declared) = table.get("property").and_then(|i| i.as_array_of_tables()) {
            for property in declared.iter() {
                match parse_property(property, name) {
                    Ok(schema) => {
                        // A project may replace an inherited property — raising
                        // a default, say — rather than having two of the name.
                        properties.retain(|p: &PropertySchema| p.name != schema.name);
                        properties.push(schema);
                    }
                    Err(d) => diagnostics.push(d),
                }
            }
        }

        let doc_line = table
            .get("doc")
            .and_then(|i| i.as_str())
            .unwrap_or("A node kind declared by this project.");
        let schema = NodeKindSchema::new(name, doc_line, properties).based_on(&behaves_as);
        if let Err(d) = registry.register(schema) {
            diagnostics.push(d);
        }
    }
    diagnostics
}

fn parse_property(table: &toml_edit::Table, kind: &str) -> Result<PropertySchema, Diagnostic> {
    let bad = |what: &str| {
        Diagnostic::new(Code::PARSE_FAILED, format!("{kind}: {what}"))
            .with_field("kind", kind.to_string())
    };
    let name = table
        .get("name")
        .and_then(|i| i.as_str())
        .ok_or_else(|| bad("a property needs a `name`"))?;
    let type_name = table
        .get("type")
        .and_then(|i| i.as_str())
        .ok_or_else(|| bad(&format!("property {name:?} needs a `type`")))?;
    let ty = property_type(table, type_name).ok_or_else(|| {
        bad(&format!(
            "property {name:?} has no type called {type_name:?}"
        ))
    })?;

    // The default goes through the same literal parser scene files use, so a
    // default is exactly as exact as an authored value — `0.1` is refused here
    // for the reason it is refused there.
    let default = match table.get("default").and_then(|i| i.as_value()) {
        Some(value) => Some(
            // On the literal text, not a decoded float: a default is exactly as
            // exact as an authored value, and `0.1` is refused here for the
            // reason it is refused there.
            crate::parse::parse_value_literal(value.to_string().trim(), &ty)
                .map_err(|d| d.with_field("property", name.to_string()))?,
        ),
        None => None,
    };

    let required = table
        .get("required")
        .and_then(|i| i.as_bool())
        .unwrap_or(false);
    if required && default.is_some() {
        return Err(bad(&format!(
            "property {name:?} is required and has a default; it can be one or the other"
        )));
    }

    let mut schema = PropertySchema::new(
        name,
        ty,
        default,
        table.get("doc").and_then(|i| i.as_str()).unwrap_or(""),
    );
    if required {
        schema = schema.required();
    }
    if let (Some(lo), Some(hi)) = (
        table.get("min").and_then(number),
        table.get("max").and_then(number),
    ) {
        schema = schema.ranged(lo, hi);
    }
    Ok(schema)
}

fn number(item: &toml_edit::Item) -> Option<dimetric_core::Fx> {
    let value = item.as_value()?;
    // Through the exact parser, on the literal text, for the same reason every
    // other number in this engine is.
    dimetric_core::Fx::parse_exact(value.to_string().trim()).ok()
}

/// The property type a name refers to.
fn property_type(table: &toml_edit::Table, name: &str) -> Option<PropertyType> {
    Some(match name {
        "scalar" => PropertyType::Scalar,
        "int" => PropertyType::Int,
        "bool" => PropertyType::Bool,
        "string" => PropertyType::Str,
        "vec2" => PropertyType::Vec2,
        "vec2i" => PropertyType::Vec2i,
        "rect" => PropertyType::Rect,
        "angle" => PropertyType::Angle,
        "color" => PropertyType::Color,
        "asset" => PropertyType::AssetRef,
        "scene" => PropertyType::SceneRef,
        "node" => PropertyType::NodeRef,
        "script" => PropertyType::ScriptRef,
        "enum" => {
            let variants: Vec<String> = table
                .get("variants")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect();
            if variants.is_empty() {
                return None;
            }
            PropertyType::Enum(variants)
        }
        _ => return None,
    })
}
