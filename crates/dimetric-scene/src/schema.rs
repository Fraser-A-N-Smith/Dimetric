//! Node kinds and their property schemas.
//!
//! Node kinds are data, not a Rust enum. A kind is a string plus a list of
//! property schemas, registered at runtime, so a third-party crate can add
//! `HexTileLayer` without patching the engine.
//!
//! Registration is not optional. Unknown properties are a hard error
//! (`DIM0301`) — it is what catches the `raduis = 72.0` typo class, which
//! otherwise sits in a file for months doing nothing — and the cost of that
//! decision is that a kind must declare its schema before its scenes will
//! load.

use std::collections::BTreeMap;

use dimetric_core::{Code, Diagnostic, Fx};

use crate::value::Value;

/// Keys that belong to every node and may not be reused by a kind.
///
/// Properties sit directly on the node table rather than in a `props`
/// sub-table. That costs this fixed reserved list, and saves a header line per
/// node and a level of nesting in every agent patch. Readability won.
pub const RESERVED_KEYS: &[&str] = &[
    "id", "kind", "name", "parent", "scene", "script", "pos", "rot", "scale", "visible", "z",
    "layer", "tags",
];

/// True when `key` is reserved for the engine.
pub fn is_reserved(key: &str) -> bool {
    RESERVED_KEYS.contains(&key)
}

/// The type of a property, which drives both parsing and validation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PropertyType {
    /// Fixed-point scalar. Written `12.5`, always with a decimal point.
    Scalar,
    /// Whole number. Written `42`, never with a decimal point.
    Int,
    /// `true` or `false`.
    Bool,
    /// Free text.
    Str,
    /// `[x, y]` of scalars.
    Vec2,
    /// `[x, y]` of integers, for cells and chunk coordinates.
    Vec2i,
    /// `[x, y, w, h]` of scalars.
    Rect,
    /// Degrees in text.
    Angle,
    /// `"#rrggbbaa"`.
    Color,
    /// One of a fixed set of names.
    Enum(Vec<String>),
    /// `"asset:..."`.
    AssetRef,
    /// `"scene:..."`.
    SceneRef,
    /// `"node:..."`.
    NodeRef,
    /// `"script:..."`.
    ScriptRef,
    /// A homogeneous list.
    List(Box<PropertyType>),
    /// A string-keyed map with homogeneous values.
    Map(Box<PropertyType>),
}

impl PropertyType {
    /// The name used in diagnostics and generated schemas.
    pub fn name(&self) -> String {
        match self {
            PropertyType::Scalar => "scalar".into(),
            PropertyType::Int => "int".into(),
            PropertyType::Bool => "bool".into(),
            PropertyType::Str => "string".into(),
            PropertyType::Vec2 => "vec2".into(),
            PropertyType::Vec2i => "vec2i".into(),
            PropertyType::Rect => "rect".into(),
            PropertyType::Angle => "angle".into(),
            PropertyType::Color => "color".into(),
            PropertyType::Enum(names) => format!("enum({})", names.join(" | ")),
            PropertyType::AssetRef => "asset-ref".into(),
            PropertyType::SceneRef => "scene-ref".into(),
            PropertyType::NodeRef => "node-ref".into(),
            PropertyType::ScriptRef => "script-ref".into(),
            PropertyType::List(inner) => format!("list<{}>", inner.name()),
            PropertyType::Map(inner) => format!("map<{}>", inner.name()),
        }
    }
}

/// One property of a node kind.
#[derive(Clone, Debug)]
pub struct PropertySchema {
    /// Key as it appears in the file.
    pub name: String,
    /// Declared type.
    pub ty: PropertyType,
    /// Value assumed when the key is absent.
    ///
    /// Canonical form omits any property equal to its default, so adding a
    /// property to a kind does not rewrite every existing scene.
    pub default: Option<Value>,
    /// When true, absence is `DIM0303`.
    pub required: bool,
    /// Inclusive bounds for numeric properties, checked as `DIM0202`.
    pub range: Option<(Fx, Fx)>,
    /// One line for generated documentation.
    pub doc: String,
}

impl PropertySchema {
    /// A property with a default and no range.
    pub fn new(name: &str, ty: PropertyType, default: Option<Value>, doc: &str) -> PropertySchema {
        PropertySchema {
            name: name.to_string(),
            ty,
            default,
            required: false,
            range: None,
            doc: doc.to_string(),
        }
    }

    /// Mark as required.
    pub fn required(mut self) -> PropertySchema {
        self.required = true;
        self.default = None;
        self
    }

    /// Attach inclusive numeric bounds.
    pub fn ranged(mut self, lo: Fx, hi: Fx) -> PropertySchema {
        self.range = Some((lo, hi));
        self
    }
}

/// A registered node kind.
#[derive(Clone, Debug)]
pub struct NodeKindSchema {
    /// The `kind` string in the file.
    pub kind: String,
    /// The built-in kind this one behaves as.
    ///
    /// A built-in is its own base. A project kind that extends one carries the
    /// built-in's name here, which is how the engine knows that an `Enemy`
    /// collides: the simulation and the renderer ask what a node *is*, and a
    /// name the engine has never heard of would otherwise simply be skipped.
    pub base: String,
    /// One line for generated documentation.
    pub doc: String,
    /// Properties, in declaration order. Canonical form sorts them
    /// alphabetically on disk regardless.
    pub properties: Vec<PropertySchema>,
}

impl NodeKindSchema {
    /// Build a kind from its properties.
    pub fn new(kind: &str, doc: &str, properties: Vec<PropertySchema>) -> NodeKindSchema {
        NodeKindSchema {
            kind: kind.to_string(),
            base: kind.to_string(),
            doc: doc.to_string(),
            properties,
        }
    }

    /// The same kind, behaving as `base`.
    pub fn based_on(mut self, base: &str) -> NodeKindSchema {
        self.base = base.to_string();
        self
    }

    /// Find a property schema by key.
    pub fn property(&self, name: &str) -> Option<&PropertySchema> {
        self.properties.iter().find(|p| p.name == name)
    }
}

/// The set of kinds a project understands.
///
/// Backed by a `BTreeMap` so that iteration order — which reaches generated
/// documentation and JSON schemas — does not depend on hashing (I4).
#[derive(Clone, Debug, Default)]
pub struct KindRegistry {
    kinds: BTreeMap<String, NodeKindSchema>,
}

impl KindRegistry {
    /// An empty registry.
    pub fn empty() -> KindRegistry {
        KindRegistry::default()
    }

    /// A registry holding every built-in kind.
    pub fn with_builtins() -> KindRegistry {
        let mut r = KindRegistry::empty();
        for schema in crate::kinds::builtin_kinds() {
            r.register(schema)
                .expect("built-in kinds must not shadow reserved keys");
        }
        r
    }

    /// Add a kind.
    ///
    /// Fails with `DIM0104` if the kind declares a reserved key, which is
    /// checked here rather than at load time so the mistake surfaces when the
    /// kind is written, not when someone else's scene fails to open.
    pub fn register(&mut self, schema: NodeKindSchema) -> Result<(), Diagnostic> {
        for p in &schema.properties {
            if is_reserved(&p.name) {
                return Err(Diagnostic::new(
                    Code::RESERVED_KEY,
                    format!(
                        "node kind {:?} declares {:?}, which is reserved for the engine",
                        schema.kind, p.name
                    ),
                )
                .with_field("kind", schema.kind.clone())
                .with_field("property", p.name.clone()));
            }
        }
        self.kinds.insert(schema.kind.clone(), schema);
        Ok(())
    }

    /// Look up a kind.
    pub fn get(&self, kind: &str) -> Option<&NodeKindSchema> {
        self.kinds.get(kind)
    }

    /// True when the kind is registered.
    pub fn contains(&self, kind: &str) -> bool {
        self.kinds.contains_key(kind)
    }

    /// Every kind, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &NodeKindSchema> {
        self.kinds.values()
    }

    /// How many kinds are registered.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}
