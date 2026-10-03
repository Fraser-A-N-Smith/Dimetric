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

/// What each reserved key is for, in the order [`RESERVED_KEYS`] lists them.
///
/// Separate from the list itself because the list is a validation input and
/// this is a reference: a kind schema needs to know that `pos` is taken, and a
/// person needs to know what it does. Kept beside it so a test can assert the
/// two agree — `docs/API.md` refers to "the reserved keys" in four places and
/// listed them in none, which cost somebody an afternoon working out that
/// `layer` exists.
pub const RESERVED_KEY_DOCS: &[(&str, &str)] = &[
    (
        "id",
        "Permanent identity, as written in the file. What `parent`, `scene` and a          replay probe refer to.",
    ),
    ("kind", "Registered node kind."),
    (
        "name",
        "Human-facing, unique among siblings. What a `/Root/Child` path addresses.",
    ),
    ("parent", "The `id` this node hangs under."),
    (
        "scene",
        "For an `Instance`, the scene to stamp out. A typed `scene:` reference.",
    ),
    (
        "script",
        "Attached script, as a typed `script:` reference. Its hooks run in the tick.",
    ),
    ("pos", "Local translation, `[x, y]`, relative to the parent."),
    ("rot", "Local rotation, in degrees."),
    ("scale", "Local scale, `[x, y]`."),
    (
        "visible",
        "Drawn when true. Also gates the sweep: an invisible collider is not a body.",
    ),
    (
        "z",
        "Draw order within a layer. Beats depth, so a node with a higher `z` draws          in front of one further down the screen. Defaults to 0.",
    ),
    (
        "layer",
        "Render layer, and the coarsest thing draw order sorts on — it beats `z`,          depth and everything else. Use it to keep a whole class of node in front          of or behind another; use `z` within one. Clamped to -128..127.",
    ),
    (
        "tags",
        "Free-form strings. Read by scripts and used to filter spatial queries and          collisions.",
    ),
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
    /// A value of this type, standing in for one a node does not carry.
    ///
    /// [`Value::reshape`] decides what a write may become by comparing it
    /// against the value already in the node, because that value came through
    /// the parser and the parser had the schema. A property with no default is
    /// absent from a node that did not author one, so there is no such value —
    /// and a write to it used to land in state exactly as the scripting
    /// boundary made it, a list or a string where a rect belonged. The save
    /// then wrote that shape out, the loader typed it from the schema, and the
    /// two hashed differently.
    ///
    /// This supplies a value purely to *be* a type. Nothing reads what is in
    /// it and nothing stores it; it exists so the schema can answer the same
    /// question the authored value answers, with the same code.
    pub fn witness(&self) -> Value {
        match self {
            PropertyType::Scalar => Value::Scalar(Fx::ZERO),
            PropertyType::Int => Value::Int(0),
            PropertyType::Bool => Value::Bool(false),
            PropertyType::Str => Value::Str(String::new()),
            PropertyType::Vec2 => Value::Vec2(dimetric_core::Vec2Fx::ZERO),
            PropertyType::Vec2i => Value::Vec2i([0, 0]),
            PropertyType::Rect => Value::Rect(dimetric_core::Rect::ZERO),
            PropertyType::Angle => Value::Angle(dimetric_core::Angle::ZERO),
            PropertyType::Color => Value::Color(crate::value::Color::TRANSPARENT),
            PropertyType::Enum(_) => Value::Enum(String::new()),
            PropertyType::AssetRef => Value::Ref(crate::value::Reference::Asset(String::new())),
            PropertyType::SceneRef => Value::Ref(crate::value::Reference::Scene(String::new())),
            PropertyType::NodeRef => Value::Ref(crate::value::Reference::Node(String::new())),
            PropertyType::ScriptRef => Value::Ref(crate::value::Reference::Script(String::new())),
            PropertyType::List(_) => Value::List(Vec::new()),
            PropertyType::Map(_) => Value::Map(Default::default()),
        }
    }

    /// The prefix a reference property requires, if this is one.
    ///
    /// The four reference types share one [`Value`] variant, so the variant
    /// cannot tell them apart and the schema has to.
    pub fn reference_prefix(&self) -> Option<&'static str> {
        match self {
            PropertyType::AssetRef => Some("asset:"),
            PropertyType::SceneRef => Some("scene:"),
            PropertyType::NodeRef => Some("node:"),
            PropertyType::ScriptRef => Some("script:"),
            _ => None,
        }
    }

    /// Make `incoming` into this declared type, or say why it cannot be.
    ///
    /// The same decision [`Value::reshape`] makes, taken against a declared
    /// type instead of an authored value — so a write accepted over a value of
    /// type `T` is accepted over an absent property declared `T`, and refused
    /// where the other would refuse it. `witness` is what keeps the two from
    /// drifting: there is one set of rules, and this hands it a stand-in.
    ///
    /// Two things only the schema knows are checked on top, and both of them
    /// stop a save from *loading* rather than merely changing its hash: which
    /// variants an enum has, and which prefix a reference property wants. The
    /// value-driven path cannot check either, which is why its own
    /// documentation says a variant is the schema's business.
    pub fn reshape(&self, incoming: Value) -> Result<Value, crate::value::Mismatch> {
        use crate::value::Mismatch;
        let value = self.witness().reshape(incoming)?;
        match (self, &value) {
            (PropertyType::Enum(names), Value::Enum(name)) => {
                if names.iter().any(|n| n == name) {
                    Ok(value)
                } else {
                    Err(Mismatch::Content(format!(
                        "it is not one of {}",
                        names.join(", ")
                    )))
                }
            }
            (_, Value::Ref(r)) => match self.reference_prefix() {
                Some(want) if r.prefix() != want => Err(Mismatch::Content(format!(
                    "a {want} reference is expected and this is a {}",
                    r.prefix()
                ))),
                _ => Ok(value),
            },
            _ => Ok(value),
        }
    }

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
