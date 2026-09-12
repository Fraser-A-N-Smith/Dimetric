//! Canonical `.dim` output.
//!
//! This module defines one rendering of a scene, the way `rustfmt` defines one
//! rendering of Rust. `dim scene fmt` applies it and CI checks it.
//!
//! Ordinary engine edits do **not** go through here. They rewrite a value in
//! the `toml_edit` document the scene was parsed from, which leaves comments,
//! key order and whitespace exactly as the author left them. Formatting is a
//! deliberate, reviewable step rather than something that fires on every save
//! and buries the real change in noise.

use std::collections::BTreeMap;

use dimetric_core::{NodeId, NodeUid};

use crate::chunk::{encode_rle, ChunkData};
use crate::node::ParentRef;
use crate::schema::KindRegistry;
use crate::tree::Scene;
use crate::value::{Reference, Value};

/// Column that regenerated path comments start at.
const COMMENT_COLUMN: usize = 31;

/// Reserved keys, in the order canonical form writes them.
const RESERVED_ORDER: &[&str] = &[
    "id", "kind", "name", "parent", "scene", "script", "pos", "rot", "scale", "visible", "z",
    "layer", "tags",
];

/// Resolves a node id inside a prefab to a readable path, for the comments on
/// override blocks. Supplied by whatever has the project's other scenes loaded.
pub type SourceResolver<'a> = dyn Fn(&Reference, NodeUid) -> Option<String> + 'a;

/// Render a scene in canonical form.
pub fn to_canonical_text(
    scene: &Scene,
    registry: &KindRegistry,
    resolve_source: Option<&SourceResolver<'_>>,
) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("format = \"dimetric\"\nversion = 1\n");

    if let Some(root) = scene.root() {
        let uid = scene.get(root).expect("root exists").uid;
        out.push_str("\n[scene]\n");
        out.push_str(&format!("root = {}\n", quote(&uid.to_text())));
    }

    // Depth first, parents before children, so file order matches tree order.
    // The flat list costs the tree's visual shape; canonical ordering gives it
    // back without nesting anything.
    for id in scene.walk() {
        write_node(&mut out, scene, registry, id);
    }

    write_overrides(&mut out, scene, resolve_source);
    write_connections(&mut out, scene);
    write_chunks(&mut out, scene);
    out
}

fn write_node(out: &mut String, scene: &Scene, registry: &KindRegistry, id: NodeId) {
    let Some(node) = scene.get(id) else { return };
    out.push_str("\n[[node]]\n");
    line(out, "id", &quote(&node.uid.to_text()), None);
    line(out, "kind", &quote(&node.kind), None);
    line(out, "name", &quote(&node.name), None);

    if let Some(parent) = node.parent() {
        let parent_uid = scene.get(parent).expect("parent exists").uid;
        let text = match node.inner_parent {
            Some(inner) => ParentRef::Inner {
                instance: parent_uid,
                inner,
            },
            None => ParentRef::Node(parent_uid),
        };
        if let Some(rendered) = text.to_text() {
            line(out, "parent", &quote(&rendered), scene.path_of(parent).as_deref());
        }
    }
    if let Some(scene_ref) = &node.scene {
        line(out, "scene", &quote(&scene_ref.to_text()), None);
    }
    if let Some(script) = &node.script {
        line(out, "script", &quote(&script.to_text()), None);
    }

    // Defaults are omitted entirely, so adding a property to a node kind does
    // not rewrite every scene that already exists.
    let t = node.transform;
    if t.pos != dimetric_core::Vec2Fx::ZERO {
        line(out, "pos", &render(&Value::Vec2(t.pos)), None);
    }
    if t.rot != dimetric_core::Angle::ZERO {
        line(out, "rot", &render(&Value::Angle(t.rot)), None);
    }
    if t.scale != dimetric_core::Vec2Fx::ONE {
        line(out, "scale", &render(&Value::Vec2(t.scale)), None);
    }
    if !node.visible {
        line(out, "visible", "false", None);
    }
    if node.z != 0 {
        line(out, "z", &node.z.to_string(), None);
    }
    if node.layer != 0 {
        line(out, "layer", &node.layer.to_string(), None);
    }
    if !node.tags.is_empty() {
        let items: Vec<String> = node.tags.iter().map(|t| quote(t)).collect();
        line(out, "tags", &format!("[{}]", items.join(", ")), None);
    }

    // Kind properties alphabetically, so two people adding different
    // properties to the same node touch different lines.
    let schema = registry.get(&node.kind);
    let mut keys: Vec<&String> = node.props.keys().collect();
    keys.sort();
    for key in keys {
        let value = &node.props[key];
        if let Some(default) = schema.and_then(|s| s.property(key)).and_then(|p| p.default.as_ref()) {
            if default == value {
                continue;
            }
        }
        let comment = match value {
            Value::Ref(Reference::Node(uid)) => NodeUid::parse(uid)
                .ok()
                .and_then(|u| scene.by_uid(u))
                .and_then(|n| scene.path_of(n)),
            _ => None,
        };
        line(out, key, &render(value), comment.as_deref());
    }
}

fn write_overrides(out: &mut String, scene: &Scene, resolve: Option<&SourceResolver<'_>>) {
    if scene.overrides.is_empty() {
        return;
    }
    // Grouped by instance, in node order: an instance's overrides read as one
    // block rather than being scattered through the file.
    let order: BTreeMap<NodeUid, usize> = scene
        .walk()
        .iter()
        .enumerate()
        .filter_map(|(i, id)| scene.get(*id).map(|n| (n.uid, i)))
        .collect();
    let mut blocks: Vec<&crate::node::Override> = scene.overrides.iter().collect();
    blocks.sort_by_key(|b| {
        (
            order.get(&b.instance).copied().unwrap_or(usize::MAX),
            b.target.to_text(),
        )
    });

    for block in blocks {
        out.push_str("\n[[override]]\n");
        let instance_path = scene.by_uid(block.instance).and_then(|id| scene.path_of(id));
        line(
            out,
            "instance",
            &quote(&block.instance.to_text()),
            instance_path.as_deref(),
        );
        let target_path = scene
            .by_uid(block.instance)
            .and_then(|id| scene.get(id))
            .and_then(|n| n.scene.as_ref())
            .zip(resolve)
            .and_then(|(source, r)| r(source, block.target));
        line(out, "target", &quote(&block.target.to_text()), target_path.as_deref());
        if block.removed {
            line(out, "removed", "true", None);
        }
        let mut keys: Vec<&String> = block.props.keys().collect();
        keys.sort();
        for key in keys {
            line(out, key, &render(&block.props[key]), None);
        }
    }
}

fn write_connections(out: &mut String, scene: &Scene) {
    if scene.connections.is_empty() {
        return;
    }
    let mut sorted: Vec<&crate::node::Connection> = scene.connections.iter().collect();
    sorted.sort_by(|a, b| {
        a.from
            .to_text()
            .cmp(&b.from.to_text())
            .then(a.signal.cmp(&b.signal))
            .then(a.to.to_text().cmp(&b.to.to_text()))
            .then(a.method.cmp(&b.method))
    });
    for c in sorted {
        out.push_str("\n[[connect]]\n");
        let from_path = scene.by_uid(c.from).and_then(|id| scene.path_of(id));
        let to_path = scene.by_uid(c.to).and_then(|id| scene.path_of(id));
        line(out, "from", &quote(&c.from.to_text()), from_path.as_deref());
        line(out, "signal", &quote(&c.signal), None);
        line(out, "to", &quote(&c.to.to_text()), to_path.as_deref());
        line(out, "method", &quote(&c.method), None);
    }
}

fn write_chunks(out: &mut String, scene: &Scene) {
    if scene.chunks.is_empty() {
        return;
    }
    let mut sorted: Vec<&crate::chunk::Chunk> = scene.chunks.iter().collect();
    sorted.sort_by_key(|c| (c.layer.to_text(), c.at[1], c.at[0]));
    for chunk in sorted {
        // An all-empty chunk carries no information; dropping it keeps a file
        // from growing every time someone pans over blank space.
        if chunk.is_empty() {
            continue;
        }
        out.push_str("\n[[chunk]]\n");
        let layer_path = scene.by_uid(chunk.layer).and_then(|id| scene.path_of(id));
        line(out, "layer", &quote(&chunk.layer.to_text()), layer_path.as_deref());
        line(
            out,
            "at",
            &format!("[{}, {}]", chunk.at[0], chunk.at[1]),
            None,
        );
        match &chunk.data {
            ChunkData::Inline(cells) => {
                line(out, "data", &quote(&encode_rle(cells.as_slice())), None)
            }
            ChunkData::External(src) => line(out, "src", &quote(src), None),
        }
    }
}

/// Write `key = value`, with a regenerated path comment when one applies.
///
/// The id is authoritative and the comment is derived, rewritten on every
/// format. A stale comment is therefore a reliable sign that a file was
/// hand-edited without being formatted.
fn line(out: &mut String, key: &str, value: &str, comment: Option<&str>) {
    let text = format!("{key} = {value}");
    match comment {
        Some(path) => {
            let pad = COMMENT_COLUMN.saturating_sub(text.chars().count()).max(1);
            out.push_str(&format!("{text}{:pad$}# {path}\n", "", pad = pad));
        }
        None => {
            out.push_str(&text);
            out.push('\n');
        }
    }
}

/// Render a value as a TOML literal.
pub fn render(value: &Value) -> String {
    match value {
        Value::Scalar(v) => v.to_exact_string(),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Str(s) => quote(s),
        Value::Enum(s) => quote(s),
        Value::Color(c) => quote(&c.to_hex()),
        Value::Ref(r) => quote(&r.to_text()),
        Value::Angle(a) => a.to_degrees_string(),
        Value::Vec2(v) => format!("[{}, {}]", v.x.to_exact_string(), v.y.to_exact_string()),
        Value::Vec2i([x, y]) => format!("[{x}, {y}]"),
        Value::Rect(r) => format!(
            "[{}, {}, {}, {}]",
            r.pos.x.to_exact_string(),
            r.pos.y.to_exact_string(),
            r.size.x.to_exact_string(),
            r.size.y.to_exact_string()
        ),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(render).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Map(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{} = {}", bare_or_quoted(k), render(&map[*k])))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

/// Quote a string as a TOML basic string, escaping through `toml_edit` rather
/// than by hand.
pub fn quote(s: &str) -> String {
    toml_edit::Value::from(s).to_string().trim().to_string()
}

fn bare_or_quoted(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        key.to_string()
    } else {
        quote(key)
    }
}

/// The canonical key order for a node table, exposed for tooling that wants to
/// sort keys without re-rendering a whole file.
pub fn reserved_key_order() -> &'static [&'static str] {
    RESERVED_ORDER
}
