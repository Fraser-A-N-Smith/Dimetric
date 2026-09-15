//! Reading `.dim` files.
//!
//! Parsing is schema-directed. Several value types share a TOML
//! representation — an enum, a colour and a reference are all quoted strings —
//! so the node kind's schema decides how a literal is read. That is also what
//! makes a wrong type a reportable `DIM0201` rather than a silent acceptance.
//!
//! Numbers are read from the **raw literal text**, never from the `f64` that
//! TOML nominally holds. Going through a float would round `0.1` to something
//! representable and hide exactly the error `DIM0203` exists to raise.

use std::collections::BTreeMap;

use dimetric_core::{Angle, Code, Diagnostic, Diagnostics, Fx, NodeUid, Rect, Span, Vec2Fx};
use indexmap::IndexMap;
use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

use crate::chunk::{decode_rle, Chunk, ChunkData};
use crate::node::{Connection, Node, Override, ParentRef, Transform};
use crate::schema::{is_reserved, KindRegistry, PropertyType};
use crate::tree::Scene;
use crate::value::{Color, Reference, Value};

/// The format marker every scene file carries.
pub const FORMAT_TAG: &str = "dimetric";
/// The scene format version this build writes and reads.
pub const FORMAT_VERSION: i64 = 1;

/// A scene together with the document it was parsed from.
///
/// Keeping the `toml_edit` document alive is the whole of invariant I2. An
/// edit rewrites one value in this document and leaves comments, key order and
/// whitespace untouched, so saving an unedited scene reproduces the input byte
/// for byte without any canonicalisation step.
pub struct SceneDoc {
    /// The format-preserving source document.
    pub doc: DocumentMut,
    /// The parsed tree.
    pub scene: Scene,
    /// Where it came from, for diagnostics.
    pub source_path: String,
}

impl SceneDoc {
    /// Serialize back to text.
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }
}

/// Result of a parse: whatever could be built, plus everything that was wrong
/// with it.
///
/// Validation collects rather than bailing at the first problem. A scene with
/// six typos should report six typos, not make the author run the loader six
/// times.
pub struct ParseOutput {
    /// The scene, when it could be built at all.
    pub doc: Option<SceneDoc>,
    /// Everything found, errors and warnings alike.
    pub diagnostics: Diagnostics,
}

impl ParseOutput {
    /// The scene, or the diagnostics that stopped it being built.
    pub fn into_result(self) -> Result<SceneDoc, Diagnostics> {
        match self.doc {
            Some(d) if !self.diagnostics.has_errors() => Ok(d),
            _ => Err(self.diagnostics),
        }
    }
}

/// Line numbers for the keys of one block, so diagnostics can point at a line.
type KeyLines = BTreeMap<String, u32>;

/// Parse a scene file.
pub fn parse(source: &str, path: &str, registry: &KindRegistry) -> ParseOutput {
    let mut diags = Diagnostics::new();

    let doc: DocumentMut = match source.parse() {
        Ok(d) => d,
        Err(e) => {
            diags.push(
                Diagnostic::new(Code::PARSE_FAILED, e.to_string())
                    .with_span(Span::file(path))
                    .with_field("detail", e.to_string()),
            );
            return ParseOutput {
                doc: None,
                diagnostics: diags,
            };
        }
    };

    let lines = collect_lines(source);
    let mut cx = Cx {
        path,
        registry,
        diags: &mut diags,
    };

    check_header(&doc, &mut cx);

    let declared_root = doc
        .get("scene")
        .and_then(Item::as_table)
        .and_then(|t| t.get("root"))
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str)
        .map(str::to_string);

    let node_tables: Vec<&Table> = doc
        .get("node")
        .and_then(Item::as_array_of_tables)
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    // Pre-pass: every id in the file, so a forward parent reference can be
    // reported as "declared out of order" rather than "does not exist".
    let all_ids: Vec<Option<NodeUid>> = node_tables
        .iter()
        .map(|t| {
            t.get("id")
                .and_then(Item::as_value)
                .and_then(TomlValue::as_str)
                .and_then(|s| NodeUid::parse(s).ok())
        })
        .collect();

    let mut scene = Scene::new();
    let mut seen: Vec<NodeUid> = Vec::new();

    for (index, table) in node_tables.iter().enumerate() {
        let key_lines = lines
            .get("node")
            .and_then(|v| v.get(index))
            .cloned()
            .unwrap_or_default();
        if let Some((node, parent)) = parse_node(table, index, &all_ids, &seen, &key_lines, &mut cx)
        {
            let uid = node.uid;
            let parent_id = match &parent {
                ParentRef::None => None,
                ParentRef::Node(p) => scene.by_uid(*p),
                // A node attached inside an instance is held aside until the
                // instance is resolved; in the file's own tree it hangs off the
                // instance node.
                ParentRef::Inner { instance, .. } => scene.by_uid(*instance),
            };
            match scene.insert(node, parent_id) {
                Ok(_) => seen.push(uid),
                Err(d) => cx
                    .diags
                    .push(d.with_span(span_at(&key_lines, "id", cx.path))),
            }
        }
    }

    parse_overrides(&doc, &lines, &mut scene, &mut cx);
    parse_connections(&doc, &lines, &mut scene, &mut cx);
    parse_chunks(&doc, &lines, &mut scene, &mut cx);

    if let Some(root_text) = declared_root {
        match NodeUid::parse(&root_text) {
            Ok(uid) if scene.by_uid(uid).is_some() => {}
            _ => cx.diags.push(
                Diagnostic::new(
                    Code::MISSING_ROOT,
                    format!("[scene] names root {root_text:?}, which is not a node in this file"),
                )
                .with_span(Span::file(path))
                .with_field("root", root_text),
            ),
        }
    }

    ParseOutput {
        doc: Some(SceneDoc {
            doc,
            scene,
            source_path: path.to_string(),
        }),
        diagnostics: diags,
    }
}

/// Parse a single value literal against a declared type.
///
/// This is how the CLI and the MCP server turn `radius=72.0` on a command line
/// into a typed value. It goes through the same reader the scene loader uses,
/// so a value an agent writes is validated exactly as a value in a file is —
/// `DIM0203` included.
pub fn parse_value_literal(text: &str, ty: &PropertyType) -> Result<Value, Diagnostic> {
    let wrapped = format!("x = {text}");
    let doc: DocumentMut = wrapped.parse().map_err(|e: toml_edit::TomlError| {
        Diagnostic::new(
            Code::TYPE_MISMATCH,
            format!("{text:?} is not a TOML value: {e}"),
        )
        .with_field("literal", text.to_string())
    })?;
    let value = doc
        .get("x")
        .and_then(Item::as_value)
        .ok_or_else(|| Diagnostic::new(Code::TYPE_MISMATCH, format!("{text:?} is not a value")))?;
    let mut diags = Diagnostics::new();
    let registry = KindRegistry::empty();
    let mut cx = Cx {
        path: "<argument>",
        registry: &registry,
        diags: &mut diags,
    };
    let lines = KeyLines::new();
    read_typed(value, ty, "value", &lines, &mut cx)
}

/// Parse a value literal for a named property of a node kind.
pub fn parse_property_literal(
    registry: &KindRegistry,
    kind: &str,
    key: &str,
    text: &str,
) -> Result<Value, Diagnostic> {
    let ty = property_type_of(registry, kind, key)?;
    parse_value_literal(text, &ty)
}

/// The declared type of a property, including the reserved keys.
pub fn property_type_of(
    registry: &KindRegistry,
    kind: &str,
    key: &str,
) -> Result<PropertyType, Diagnostic> {
    Ok(match key {
        "pos" | "scale" => PropertyType::Vec2,
        "rot" => PropertyType::Angle,
        "visible" => PropertyType::Bool,
        "z" | "layer" => PropertyType::Int,
        "name" => PropertyType::Str,
        "script" => PropertyType::ScriptRef,
        "scene" => PropertyType::SceneRef,
        _ => {
            let schema = registry.get(kind).ok_or_else(|| {
                Diagnostic::new(
                    Code::UNKNOWN_KIND,
                    format!("no node kind named {kind:?} is registered"),
                )
                .with_field("kind", kind.to_string())
            })?;
            let prop = schema.property(key).ok_or_else(|| {
                let suggestion = nearest(key, schema.properties.iter().map(|p| p.name.as_str()));
                let mut d = Diagnostic::new(
                    Code::UNKNOWN_PROPERTY,
                    match &suggestion {
                        Some(s) => format!("{kind} has no property {key:?}; did you mean {s:?}?"),
                        None => format!("{kind} has no property {key:?}"),
                    },
                )
                .with_field("kind", kind.to_string())
                .with_field("property", key.to_string());
                if let Some(s) = suggestion {
                    d = d.with_field("suggestion", s);
                }
                d
            })?;
            prop.ty.clone()
        }
    })
}

struct Cx<'a> {
    path: &'a str,
    registry: &'a KindRegistry,
    diags: &'a mut Diagnostics,
}

fn check_header(doc: &DocumentMut, cx: &mut Cx) {
    let format = doc
        .get("format")
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str);
    let version = doc
        .get("version")
        .and_then(Item::as_value)
        .and_then(TomlValue::as_integer);
    match (format, version) {
        (Some(FORMAT_TAG), Some(FORMAT_VERSION)) => {}
        (Some(FORMAT_TAG), Some(v)) => cx.diags.push(
            Diagnostic::new(
                Code::BAD_HEADER,
                format!("scene format version {v} is not supported; this build reads version {FORMAT_VERSION}"),
            )
            .with_span(Span::file(cx.path))
            .with_field("found", v)
            .with_field("supported", FORMAT_VERSION),
        ),
        _ => cx.diags.push(
            Diagnostic::new(
                Code::BAD_HEADER,
                format!("expected `format = \"{FORMAT_TAG}\"` and `version = {FORMAT_VERSION}` at the top of the file"),
            )
            .with_span(Span::file(cx.path)),
        ),
    }
}

fn parse_node(
    table: &Table,
    index: usize,
    all_ids: &[Option<NodeUid>],
    seen: &[NodeUid],
    lines: &KeyLines,
    cx: &mut Cx,
) -> Option<(Node, ParentRef)> {
    let uid = match required_str(table, "id", lines, cx) {
        Some(s) => match NodeUid::parse(&s) {
            Ok(u) => u,
            Err(e) => {
                cx.diags.push(
                    Diagnostic::new(Code::BAD_ID_FORM, e.to_string())
                        .with_span(span_at(lines, "id", cx.path))
                        .with_field("id", s),
                );
                return None;
            }
        },
        None => return None,
    };

    let kind = required_str(table, "kind", lines, cx)?;
    let schema = match cx.registry.get(&kind) {
        Some(s) => s,
        None => {
            cx.diags.push(
                Diagnostic::new(
                    Code::UNKNOWN_KIND,
                    format!("no node kind named {kind:?} is registered"),
                )
                .with_span(span_at(lines, "kind", cx.path))
                .with_field("kind", kind.clone())
                .with_field("id", uid.to_text()),
            );
            return None;
        }
    };

    let name = table
        .get("name")
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| uid.to_text());

    let parent = match table
        .get("parent")
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str)
    {
        None => ParentRef::None,
        Some(s) => match ParentRef::parse(s) {
            Ok(p) => {
                if let Some(owner) = p.owner() {
                    if !seen.contains(&owner) {
                        let known_later = all_ids.iter().skip(index).flatten().any(|u| *u == owner);
                        let (code, message) = if known_later {
                            (
                                Code::CHILD_BEFORE_PARENT,
                                format!("{uid} is declared before its parent {owner}; nodes are written depth-first, parents first"),
                            )
                        } else {
                            (
                                Code::DANGLING_PARENT,
                                format!("{uid} names parent {owner}, which is not in this file"),
                            )
                        };
                        cx.diags.push(
                            Diagnostic::new(code, message)
                                .with_span(span_at(lines, "parent", cx.path))
                                .with_field("id", uid.to_text())
                                .with_field("parent", owner.to_text()),
                        );
                        return None;
                    }
                }
                p
            }
            Err(e) => {
                cx.diags.push(
                    Diagnostic::new(Code::BAD_ID_FORM, e.to_string())
                        .with_span(span_at(lines, "parent", cx.path))
                        .with_field("id", uid.to_text()),
                );
                return None;
            }
        },
    };

    let mut node = Node::new(uid, kind.clone(), name);
    node.base = schema.base.clone();
    node.transform = Transform {
        pos: table
            .get("pos")
            .and_then(|i| read_vec2(i, "pos", lines, cx))
            .unwrap_or(Vec2Fx::ZERO),
        rot: table
            .get("rot")
            .and_then(|i| read_angle(i, "rot", lines, cx))
            .unwrap_or(Angle::ZERO),
        scale: table
            .get("scale")
            .and_then(|i| read_vec2(i, "scale", lines, cx))
            .unwrap_or(Vec2Fx::ONE),
    };
    node.visible = reserved_bool(table, "visible", true, lines, cx);
    node.z = reserved_int(table, "z", lines, cx);
    node.layer = reserved_int(table, "layer", lines, cx);
    node.tags = table
        .get("tags")
        .and_then(Item::as_value)
        .and_then(TomlValue::as_array)
        .map(|a| {
            a.iter()
                .filter_map(TomlValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    node.script = read_reference(table, "script", "script:", lines, cx);
    node.scene = read_reference(table, "scene", "scene:", lines, cx);
    if let ParentRef::Inner { inner, .. } = &parent {
        node.inner_parent = Some(*inner);
    }

    for (key, item) in table.iter() {
        if is_reserved(key) {
            continue;
        }
        let Some(prop) = schema.property(key) else {
            let suggestion = nearest(key, schema.properties.iter().map(|p| p.name.as_str()));
            let mut d = Diagnostic::new(
                Code::UNKNOWN_PROPERTY,
                match &suggestion {
                    Some(s) => format!("{kind} has no property {key:?}; did you mean {s:?}?"),
                    None => format!("{kind} has no property {key:?}"),
                },
            )
            .with_span(span_at(lines, key, cx.path))
            .with_field("kind", kind.clone())
            .with_field("property", key.to_string())
            .with_field("id", uid.to_text());
            if let Some(s) = suggestion {
                d = d.with_field("suggestion", s);
            }
            cx.diags.push(d);
            continue;
        };
        let Some(value) = item.as_value() else {
            continue;
        };
        match read_typed(value, &prop.ty, key, lines, cx) {
            Ok(v) => {
                if let Some(d) = check_range(&v, prop, key, lines, cx.path) {
                    cx.diags.push(d);
                }
                node.props.insert(key.to_string(), v);
            }
            Err(d) => cx.diags.push(d.with_field("id", uid.to_text())),
        }
    }

    // Fill in defaults for anything the file left out.
    //
    // Without this, a node's property set depends on which keys the author
    // happened to write, and two scenes that behave identically hash
    // differently — which means `scene fmt`, whose whole job is to omit
    // defaults, would change a replay. Defaults belong in the model; the file
    // is free to leave them out.
    for prop in &schema.properties {
        if let Some(default) = &prop.default {
            if !node.props.contains_key(&prop.name) {
                node.props.insert(prop.name.clone(), default.clone());
            }
        }
    }

    for prop in &schema.properties {
        if prop.required && !node.props.contains_key(&prop.name) {
            cx.diags.push(
                Diagnostic::new(
                    Code::MISSING_REQUIRED,
                    format!("{kind} requires {:?}", prop.name),
                )
                .with_span(span_at(lines, "kind", cx.path))
                .with_field("kind", kind.clone())
                .with_field("property", prop.name.clone())
                .with_field("id", uid.to_text()),
            );
        }
    }

    Some((node, parent))
}

/// The exact literal text of a scalar-ish TOML value.
///
/// This is the load-bearing detail of the whole format: `as_float` would hand
/// back an `f64` that has already rounded, and the engine would never know the
/// author wrote something it cannot represent.
fn raw_repr(v: &TomlValue) -> Option<String> {
    match v {
        TomlValue::Float(f) => Some(f.display_repr().to_string()),
        TomlValue::Integer(i) => Some(i.display_repr().to_string()),
        _ => None,
    }
}

fn read_scalar(v: &TomlValue, key: &str, lines: &KeyLines, path: &str) -> Result<Fx, Diagnostic> {
    let Some(text) = raw_repr(v) else {
        return Err(type_error(key, "scalar", v, lines, path));
    };
    Fx::parse_exact(&text).map_err(|e| {
        Diagnostic::new(
            Code::NOT_REPRESENTABLE,
            format!("{key} = {text} is not exactly representable in fixed-point: {e}"),
        )
        .with_span(span_at(lines, key, path))
        .with_field("property", key.to_string())
        .with_field("literal", text)
    })
}

fn read_typed(
    v: &TomlValue,
    ty: &PropertyType,
    key: &str,
    lines: &KeyLines,
    cx: &mut Cx,
) -> Result<Value, Diagnostic> {
    let path = cx.path;
    match ty {
        PropertyType::Scalar => read_scalar(v, key, lines, path).map(Value::Scalar),
        PropertyType::Int => v
            .as_integer()
            .map(Value::Int)
            .ok_or_else(|| type_error(key, "int", v, lines, path)),
        PropertyType::Bool => v
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| type_error(key, "bool", v, lines, path)),
        PropertyType::Str => v
            .as_str()
            .map(|s| Value::Str(s.to_string()))
            .ok_or_else(|| type_error(key, "string", v, lines, path)),
        PropertyType::Vec2 => {
            let parts = fixed_array(v, 2, key, "vec2", lines, path)?;
            Ok(Value::Vec2(Vec2Fx::new(
                read_scalar(parts[0], key, lines, path)?,
                read_scalar(parts[1], key, lines, path)?,
            )))
        }
        PropertyType::Vec2i => {
            let parts = fixed_array(v, 2, key, "vec2i", lines, path)?;
            let one = |p: &TomlValue| {
                p.as_integer()
                    .map(|i| i as i32)
                    .ok_or_else(|| type_error(key, "vec2i", v, lines, path))
            };
            Ok(Value::Vec2i([one(parts[0])?, one(parts[1])?]))
        }
        PropertyType::Rect => {
            let p = fixed_array(v, 4, key, "rect", lines, path)?;
            Ok(Value::Rect(Rect::new(
                Vec2Fx::new(
                    read_scalar(p[0], key, lines, path)?,
                    read_scalar(p[1], key, lines, path)?,
                ),
                Vec2Fx::new(
                    read_scalar(p[2], key, lines, path)?,
                    read_scalar(p[3], key, lines, path)?,
                ),
            )))
        }
        PropertyType::Angle => {
            let Some(text) = raw_repr(v) else {
                return Err(type_error(key, "angle", v, lines, path));
            };
            Angle::from_degrees_str(&text)
                .map(Value::Angle)
                .map_err(|e| {
                    Diagnostic::new(Code::TYPE_MISMATCH, format!("{key} = {text}: {e}"))
                        .with_span(span_at(lines, key, path))
                        .with_field("property", key.to_string())
                })
        }
        PropertyType::Color => {
            let s = v
                .as_str()
                .ok_or_else(|| type_error(key, "color", v, lines, path))?;
            Color::parse(s).map(Value::Color).map_err(|e| {
                Diagnostic::new(Code::TYPE_MISMATCH, format!("{key} = {s:?}: {e}"))
                    .with_span(span_at(lines, key, path))
                    .with_field("property", key.to_string())
            })
        }
        PropertyType::Enum(names) => {
            let s = v
                .as_str()
                .ok_or_else(|| type_error(key, "enum", v, lines, path))?;
            if names.iter().any(|n| n == s) {
                Ok(Value::Enum(s.to_string()))
            } else {
                Err(Diagnostic::new(
                    Code::OUT_OF_RANGE,
                    format!("{key} = {s:?} is not one of {}", names.join(", ")),
                )
                .with_span(span_at(lines, key, path))
                .with_field("property", key.to_string())
                .with_field("allowed", names.join(", ")))
            }
        }
        PropertyType::AssetRef
        | PropertyType::SceneRef
        | PropertyType::NodeRef
        | PropertyType::ScriptRef => {
            let s = v
                .as_str()
                .ok_or_else(|| type_error(key, "reference", v, lines, path))?;
            let want = match ty {
                PropertyType::AssetRef => "asset:",
                PropertyType::SceneRef => "scene:",
                PropertyType::NodeRef => "node:",
                _ => "script:",
            };
            let r = Reference::parse(s).map_err(|e| {
                Diagnostic::new(Code::BAD_REFERENCE, e.to_string())
                    .with_span(span_at(lines, key, path))
                    .with_field("property", key.to_string())
            })?;
            if r.prefix() != want {
                return Err(Diagnostic::new(
                    Code::BAD_REFERENCE,
                    format!("{key} expects a {want} reference, found {}", r.prefix()),
                )
                .with_span(span_at(lines, key, path))
                .with_field("property", key.to_string())
                .with_field("expected", want));
            }
            Ok(Value::Ref(r))
        }
        PropertyType::List(inner) => {
            let arr = v
                .as_array()
                .ok_or_else(|| type_error(key, "list", v, lines, path))?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr.iter() {
                out.push(read_typed(item, inner, key, lines, cx)?);
            }
            Ok(Value::List(out))
        }
        PropertyType::Map(inner) => {
            let t = v
                .as_inline_table()
                .ok_or_else(|| type_error(key, "map", v, lines, path))?;
            let mut out = IndexMap::new();
            for (k, item) in t.iter() {
                out.insert(k.to_string(), read_typed(item, inner, key, lines, cx)?);
            }
            Ok(Value::Map(out))
        }
    }
}

fn fixed_array<'a>(
    v: &'a TomlValue,
    n: usize,
    key: &str,
    what: &str,
    lines: &KeyLines,
    path: &str,
) -> Result<Vec<&'a TomlValue>, Diagnostic> {
    let arr = v
        .as_array()
        .ok_or_else(|| type_error(key, what, v, lines, path))?;
    if arr.len() != n {
        return Err(Diagnostic::new(
            Code::TYPE_MISMATCH,
            format!(
                "{key} is a {what}: expected {n} numbers, found {}",
                arr.len()
            ),
        )
        .with_span(span_at(lines, key, path))
        .with_field("property", key.to_string()));
    }
    Ok(arr.iter().collect())
}

fn type_error(
    key: &str,
    want: &str,
    found: &TomlValue,
    lines: &KeyLines,
    path: &str,
) -> Diagnostic {
    Diagnostic::new(
        Code::TYPE_MISMATCH,
        format!("{key} should be a {want}, found a {}", found.type_name()),
    )
    .with_span(span_at(lines, key, path))
    .with_field("property", key.to_string())
    .with_field("expected", want.to_string())
    .with_field("found", found.type_name().to_string())
}

fn check_range(
    value: &Value,
    prop: &crate::schema::PropertySchema,
    key: &str,
    lines: &KeyLines,
    path: &str,
) -> Option<Diagnostic> {
    let (lo, hi) = prop.range?;
    let v = value.as_scalar()?;
    if v < lo || v > hi {
        Some(
            Diagnostic::new(
                Code::OUT_OF_RANGE,
                format!("{key} = {v} is outside {lo} ..= {hi}"),
            )
            .with_span(span_at(lines, key, path))
            .with_field("property", key.to_string())
            .with_field("min", lo.to_exact_string())
            .with_field("max", hi.to_exact_string()),
        )
    } else {
        None
    }
}

fn required_str(table: &Table, key: &str, lines: &KeyLines, cx: &mut Cx) -> Option<String> {
    match table
        .get(key)
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str)
    {
        Some(s) => Some(s.to_string()),
        None => {
            cx.diags.push(
                Diagnostic::new(Code::MISSING_KEY, format!("every node needs a {key}"))
                    .with_span(span_at(lines, key, cx.path))
                    .with_field("key", key.to_string()),
            );
            None
        }
    }
}

/// Read a reserved integer key, reporting a mismatch rather than defaulting.
///
/// Reserved keys are as much a part of the schema as kind properties are, so
/// `z = 1.5` has to be `DIM0201` — quietly reading it as zero is the class of
/// silence this format exists to avoid.
fn reserved_int(table: &Table, key: &str, lines: &KeyLines, cx: &mut Cx) -> i32 {
    let Some(v) = table.get(key).and_then(Item::as_value) else {
        return 0;
    };
    match v.as_integer() {
        Some(i) => i as i32,
        None => {
            cx.diags.push(type_error(key, "int", v, lines, cx.path));
            0
        }
    }
}

/// Read a reserved boolean key, reporting a mismatch rather than defaulting.
fn reserved_bool(table: &Table, key: &str, default: bool, lines: &KeyLines, cx: &mut Cx) -> bool {
    let Some(v) = table.get(key).and_then(Item::as_value) else {
        return default;
    };
    match v.as_bool() {
        Some(b) => b,
        None => {
            cx.diags.push(type_error(key, "bool", v, lines, cx.path));
            default
        }
    }
}

fn read_vec2(item: &Item, key: &str, lines: &KeyLines, cx: &mut Cx) -> Option<Vec2Fx> {
    let v = item.as_value()?;
    match read_typed(v, &PropertyType::Vec2, key, lines, cx) {
        Ok(Value::Vec2(v)) => Some(v),
        Ok(_) => None,
        Err(d) => {
            cx.diags.push(d);
            None
        }
    }
}

fn read_angle(item: &Item, key: &str, lines: &KeyLines, cx: &mut Cx) -> Option<Angle> {
    let v = item.as_value()?;
    match read_typed(v, &PropertyType::Angle, key, lines, cx) {
        Ok(Value::Angle(a)) => Some(a),
        Ok(_) => None,
        Err(d) => {
            cx.diags.push(d);
            None
        }
    }
}

fn read_reference(
    table: &Table,
    key: &str,
    want: &str,
    lines: &KeyLines,
    cx: &mut Cx,
) -> Option<Reference> {
    let s = table
        .get(key)
        .and_then(Item::as_value)
        .and_then(TomlValue::as_str)?;
    match Reference::parse(s) {
        Ok(r) if r.prefix() == want => Some(r),
        Ok(r) => {
            cx.diags.push(
                Diagnostic::new(
                    Code::BAD_REFERENCE,
                    format!("{key} expects a {want} reference, found {}", r.prefix()),
                )
                .with_span(span_at(lines, key, cx.path)),
            );
            None
        }
        Err(e) => {
            cx.diags.push(
                Diagnostic::new(Code::BAD_REFERENCE, e.to_string())
                    .with_span(span_at(lines, key, cx.path)),
            );
            None
        }
    }
}

fn parse_overrides(doc: &DocumentMut, lines: &LineIndex, scene: &mut Scene, cx: &mut Cx) {
    let Some(tables) = doc.get("override").and_then(Item::as_array_of_tables) else {
        return;
    };
    for (index, table) in tables.iter().enumerate() {
        let key_lines = lines
            .get("override")
            .and_then(|v| v.get(index))
            .cloned()
            .unwrap_or_default();
        let (Some(instance), Some(target)) = (
            read_uid(table, "instance", &key_lines, cx),
            read_uid(table, "target", &key_lines, cx),
        ) else {
            continue;
        };
        let mut block = Override::new(instance, target);
        block.removed = table
            .get("removed")
            .and_then(Item::as_value)
            .and_then(TomlValue::as_bool)
            .unwrap_or(false);
        for (key, item) in table.iter() {
            if matches!(key, "instance" | "target" | "removed") {
                continue;
            }
            // Override values are read without a schema: the property belongs
            // to a node in the *source* scene, whose kind is only known once
            // the prefab is loaded. Typing happens in `instance::resolve`.
            if let Some(v) = item.as_value() {
                block.props.insert(key.to_string(), untyped_value(v));
            }
        }
        scene.overrides.push(block);
    }
}

/// Best-effort typing for a value whose schema is not yet known.
fn untyped_value(v: &TomlValue) -> Value {
    match v {
        TomlValue::String(s) => match Reference::parse(s.value()) {
            Ok(r) => Value::Ref(r),
            Err(_) => match Color::parse(s.value()) {
                Ok(c) => Value::Color(c),
                Err(_) => Value::Str(s.value().clone()),
            },
        },
        TomlValue::Integer(i) => Value::Int(*i.value()),
        TomlValue::Boolean(b) => Value::Bool(*b.value()),
        TomlValue::Float(f) => Fx::parse_exact(&f.display_repr())
            .map(Value::Scalar)
            .unwrap_or_else(|_| Value::Str(f.display_repr().to_string())),
        TomlValue::Array(a) => {
            let items: Vec<Value> = a.iter().map(untyped_value).collect();
            match items.as_slice() {
                [Value::Scalar(x), Value::Scalar(y)] => Value::Vec2(Vec2Fx::new(*x, *y)),
                [Value::Int(x), Value::Int(y)] => Value::Vec2i([*x as i32, *y as i32]),
                _ => Value::List(items),
            }
        }
        TomlValue::InlineTable(t) => Value::Map(
            t.iter()
                .map(|(k, v)| (k.to_string(), untyped_value(v)))
                .collect(),
        ),
        TomlValue::Datetime(d) => Value::Str(d.to_string()),
    }
}

fn parse_connections(doc: &DocumentMut, lines: &LineIndex, scene: &mut Scene, cx: &mut Cx) {
    let Some(tables) = doc.get("connect").and_then(Item::as_array_of_tables) else {
        return;
    };
    for (index, table) in tables.iter().enumerate() {
        let key_lines = lines
            .get("connect")
            .and_then(|v| v.get(index))
            .cloned()
            .unwrap_or_default();
        let (Some(from), Some(to)) = (
            read_uid(table, "from", &key_lines, cx),
            read_uid(table, "to", &key_lines, cx),
        ) else {
            continue;
        };
        let (Some(signal), Some(method)) = (
            required_str(table, "signal", &key_lines, cx),
            required_str(table, "method", &key_lines, cx),
        ) else {
            continue;
        };
        for (uid, what) in [(from, "from"), (to, "to")] {
            if scene.by_uid(uid).is_none() {
                cx.diags.push(
                    Diagnostic::new(
                        Code::DANGLING_PARENT,
                        format!("connection {what} names {uid}, which is not in this file"),
                    )
                    .with_span(span_at(&key_lines, what, cx.path))
                    .with_field("id", uid.to_text()),
                );
            }
        }
        scene.connections.push(Connection {
            from,
            signal,
            to,
            method,
        });
    }
}

fn parse_chunks(doc: &DocumentMut, lines: &LineIndex, scene: &mut Scene, cx: &mut Cx) {
    let Some(tables) = doc.get("chunk").and_then(Item::as_array_of_tables) else {
        return;
    };
    for (index, table) in tables.iter().enumerate() {
        let key_lines = lines
            .get("chunk")
            .and_then(|v| v.get(index))
            .cloned()
            .unwrap_or_default();
        let Some(layer) = read_uid(table, "layer", &key_lines, cx) else {
            continue;
        };
        let at = table
            .get("at")
            .and_then(Item::as_value)
            .and_then(TomlValue::as_array)
            .map(|a| {
                let mut it = a.iter().filter_map(TomlValue::as_integer);
                [it.next().unwrap_or(0) as i32, it.next().unwrap_or(0) as i32]
            })
            .unwrap_or([0, 0]);

        let inline = table
            .get("data")
            .and_then(Item::as_value)
            .and_then(TomlValue::as_str);
        let external = table
            .get("src")
            .and_then(Item::as_value)
            .and_then(TomlValue::as_str);
        let data = match (inline, external) {
            (Some(text), None) => match decode_rle(text) {
                Ok(cells) => ChunkData::Inline(cells),
                Err(d) => {
                    cx.diags.push(
                        d.with_span(span_at(&key_lines, "data", cx.path))
                            .with_field("layer", layer.to_text()),
                    );
                    continue;
                }
            },
            (None, Some(src)) => ChunkData::External(src.to_string()),
            _ => {
                cx.diags.push(
                    Diagnostic::new(
                        Code::MISSING_KEY,
                        "a chunk carries exactly one of `data` or `src`",
                    )
                    .with_span(span_at(&key_lines, "layer", cx.path))
                    .with_field("layer", layer.to_text()),
                );
                continue;
            }
        };
        scene.chunks.push(Chunk { layer, at, data });
    }
}

fn read_uid(table: &Table, key: &str, lines: &KeyLines, cx: &mut Cx) -> Option<NodeUid> {
    let s = required_str(table, key, lines, cx)?;
    match NodeUid::parse(&s) {
        Ok(u) => Some(u),
        Err(e) => {
            cx.diags.push(
                Diagnostic::new(Code::BAD_ID_FORM, e.to_string())
                    .with_span(span_at(lines, key, cx.path))
                    .with_field("key", key.to_string()),
            );
            None
        }
    }
}

/// Line numbers for every key of every block, keyed by block name.
type LineIndex = BTreeMap<String, Vec<KeyLines>>;

/// Build a line index from the source, so diagnostics can cite a line.
///
/// Taken from a span-preserving parse of the same text; `DocumentMut` discards
/// spans when it takes ownership for editing.
fn collect_lines(source: &str) -> LineIndex {
    let mut index = LineIndex::new();
    let Ok(im) = toml_edit::ImDocument::parse(source) else {
        return index;
    };
    let starts: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| -> u32 { starts.partition_point(|s| *s <= offset) as u32 };

    for block in ["node", "override", "connect", "chunk"] {
        let Some(tables) = im.get(block).and_then(Item::as_array_of_tables) else {
            continue;
        };
        let mut per_table = Vec::new();
        for table in tables.iter() {
            let mut keys = KeyLines::new();
            for (key, item) in table.iter() {
                if let Some(span) = item.span() {
                    keys.insert(key.to_string(), line_of(span.start));
                }
            }
            per_table.push(keys);
        }
        index.insert(block.to_string(), per_table);
    }
    index
}

fn span_at(lines: &KeyLines, key: &str, path: &str) -> Span {
    match lines.get(key) {
        Some(line) => Span::at(path, *line),
        None => Span::file(path),
    }
}

/// Closest match by edit distance, for "did you mean" suggestions.
///
/// This is the payoff of making unknown properties a hard error: the typo is
/// caught, and the message names the property that was probably meant.
fn nearest<'a>(needle: &str, options: impl Iterator<Item = &'a str>) -> Option<String> {
    options
        .map(|o| (edit_distance(needle, o), o))
        // Within two edits, or under two fifths of the word's length. Written
        // as an integer ratio rather than 0.4, because a float here would be
        // the only one in the crate and would have to be argued about.
        .filter(|(d, o)| *d <= 2 || *d * 5 < o.len() * 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, o)| o.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (prev[j + 1] + 1).min(current[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut current);
    }
    prev[b.len()]
}
