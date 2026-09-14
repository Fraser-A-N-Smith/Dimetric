//! Canonical `.dim` output.
//!
//! This module defines one rendering of a scene, the way `rustfmt` defines one
//! rendering of Rust. `dim scene fmt` applies it and CI checks it.
//!
//! There are two ways in. [`format_in_place`] canonicalises a document that was
//! parsed from a file, moving tables rather than rewriting them so comments
//! travel with the node they were written above; this is the one the command
//! uses. [`to_canonical_text`] renders a scene with no source document.
//!
//! Ordinary engine edits do **not** go through here. They rewrite a value in
//! the `toml_edit` document the scene was parsed from, which leaves comments,
//! key order and whitespace exactly as the author left them. Formatting is a
//! deliberate, reviewable step rather than something that fires on every save
//! and buries the real change in noise.

use std::collections::BTreeMap;

use dimetric_core::{NodeId, NodeUid};
use toml_edit::{DocumentMut, Item, Table};

use crate::chunk::{encode_rle, ChunkData};
use crate::node::{Node, ParentRef};
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
            line(
                out,
                "parent",
                &quote(&rendered),
                scene.path_of(parent).as_deref(),
            );
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
        if let Some(default) = schema
            .and_then(|s| s.property(key))
            .and_then(|p| p.default.as_ref())
        {
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
        let instance_path = scene
            .by_uid(block.instance)
            .and_then(|id| scene.path_of(id));
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
        line(
            out,
            "target",
            &quote(&block.target.to_text()),
            target_path.as_deref(),
        );
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
        line(
            out,
            "layer",
            &quote(&chunk.layer.to_text()),
            layer_path.as_deref(),
        );
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

// ---------------------------------------------------------------------------
// Formatting in place
// ---------------------------------------------------------------------------

/// Rewrite a parsed document into canonical form, keeping its comments.
///
/// [`to_canonical_text`] renders a scene that has no source document. This is
/// what `dim scene fmt` uses, and the difference matters: regenerating a file
/// from the model throws away every comment in it, and §6.1 lists comment
/// preservation as one of the four reasons this format is TOML in the first
/// place. A formatter that silently deletes a designer's notes is worse than no
/// formatter.
///
/// Tables are reordered by moving them, not by rewriting them, so the comment
/// block above a node travels with that node. Within a table, keys are sorted,
/// values are re-rendered canonically, and defaults are dropped — but a
/// trailing comment on a line is kept unless it is a path comment, which is
/// derived and therefore regenerated.
pub fn format_in_place(
    doc: &mut DocumentMut,
    scene: &Scene,
    registry: &KindRegistry,
    resolve_source: Option<&SourceResolver<'_>>,
) {
    format_prologue(doc);
    format_nodes(doc, scene, registry);
    reorder_blocks(doc, "override", |t| override_sort_key(t, scene));
    reorder_blocks(doc, "connect", connection_sort_key);
    reorder_blocks(doc, "chunk", chunk_sort_key);
    order_blocks(doc);
    regenerate_reference_comments(doc, scene, resolve_source);
}

/// Put the header keys in order and the `[scene]` table right after them.
fn format_prologue(doc: &mut DocumentMut) {
    doc.as_table_mut().sort_values_by(|a, _, b, _| {
        fn rank(key: &str) -> (usize, String) {
            match key {
                "format" => (0, String::new()),
                "version" => (1, String::new()),
                other => (2, other.to_string()),
            }
        }
        rank(a.get()).cmp(&rank(b.get()))
    });
    if let Some(table) = doc.get_mut("scene").and_then(Item::as_table_mut) {
        table.decor_mut().set_prefix("\n");
    }
}

/// Lay the blocks out in canonical order.
///
/// `toml_edit` emits tables by their recorded position, which is where they sat
/// in the file it parsed. Reordering the tables within a block is not enough on
/// its own: a file with its chunks above its nodes would keep them there.
fn order_blocks(doc: &mut DocumentMut) {
    let mut next = 0;
    if let Some(table) = doc.get_mut("scene").and_then(Item::as_table_mut) {
        table.set_position(next);
        next += 1;
    }
    for block in ["node", "override", "connect", "chunk"] {
        let Some(tables) = doc.get_mut(block).and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        for table in tables.iter_mut() {
            table.set_position(next);
            next += 1;
        }
    }
}

/// Reorder `[[node]]` tables into depth-first order and canonicalise each.
fn format_nodes(doc: &mut DocumentMut, scene: &Scene, registry: &KindRegistry) {
    let Some(existing) = doc.get("node").and_then(Item::as_array_of_tables) else {
        return;
    };
    // Index the tables that are there by the id they carry.
    let mut by_id: BTreeMap<String, Table> = BTreeMap::new();
    for table in existing.iter() {
        if let Some(id) = string_field(table, "id") {
            by_id.insert(id, table.clone());
        }
    }

    let mut ordered = toml_edit::ArrayOfTables::new();
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        let key = node.uid.to_text();
        let mut table = by_id.remove(&key).unwrap_or_default();
        canonicalise_node(&mut table, node, scene, registry);
        ordered.push(table);
    }
    // Anything the model does not have is dropped: the model is the authority
    // on what the scene contains, and a table with no node behind it could not
    // have loaded.
    doc["node"] = Item::ArrayOfTables(ordered);
    normalise_block_spacing(doc, "node");
}

/// Sort one node table's keys, re-render its values, and drop its defaults.
fn canonicalise_node(table: &mut Table, node: &Node, scene: &Scene, registry: &KindRegistry) {
    let schema = registry.get(&node.kind);

    set_field(table, "id", &quote(&node.uid.to_text()));
    set_field(table, "kind", &quote(&node.kind));
    set_field(table, "name", &quote(&node.name));

    match node.parent().and_then(|p| scene.get(p)) {
        Some(parent) => {
            let text = match node.inner_parent {
                Some(inner) => format!("{}/{}", parent.uid.to_text(), inner.to_text()),
                None => parent.uid.to_text(),
            };
            set_field(table, "parent", &quote(&text));
        }
        None => {
            table.remove("parent");
        }
    }
    optional_field(
        table,
        "scene",
        node.scene.as_ref().map(|r| quote(&r.to_text())),
    );
    optional_field(
        table,
        "script",
        node.script.as_ref().map(|r| quote(&r.to_text())),
    );

    // Reserved fields equal to their default are left out entirely, so adding a
    // property to a kind does not rewrite every scene that already exists.
    let t = node.transform;
    optional_field(
        table,
        "pos",
        (t.pos != dimetric_core::Vec2Fx::ZERO).then(|| render(&Value::Vec2(t.pos))),
    );
    optional_field(
        table,
        "rot",
        (t.rot != dimetric_core::Angle::ZERO).then(|| render(&Value::Angle(t.rot))),
    );
    optional_field(
        table,
        "scale",
        (t.scale != dimetric_core::Vec2Fx::ONE).then(|| render(&Value::Vec2(t.scale))),
    );
    optional_field(
        table,
        "visible",
        (!node.visible).then(|| "false".to_string()),
    );
    optional_field(table, "z", (node.z != 0).then(|| node.z.to_string()));
    optional_field(
        table,
        "layer",
        (node.layer != 0).then(|| node.layer.to_string()),
    );
    optional_field(
        table,
        "tags",
        (!node.tags.is_empty()).then(|| {
            let items: Vec<String> = node.tags.iter().map(|t| quote(t)).collect();
            format!("[{}]", items.join(", "))
        }),
    );

    // Kind properties: alphabetical, defaults omitted.
    let mut keys: Vec<&String> = node.props.keys().collect();
    keys.sort();
    let expected: std::collections::BTreeSet<&str> = keys.iter().map(|k| k.as_str()).collect();
    let stale: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !crate::schema::is_reserved(k) && !expected.contains(k.as_str()))
        .collect();
    for key in stale {
        table.remove(&key);
    }
    for key in keys {
        let value = &node.props[key];
        let is_default = schema
            .and_then(|s| s.property(key))
            .and_then(|p| p.default.as_ref())
            .is_some_and(|d| d == value);
        if is_default {
            table.remove(key);
        } else {
            set_field(table, key, &render(value));
        }
    }

    table.sort_values_by(|a, _, b, _| key_rank(a.get()).cmp(&key_rank(b.get())));
}

/// Where a key sorts within a node table.
fn key_rank(key: &str) -> (usize, String) {
    match RESERVED_ORDER.iter().position(|r| *r == key) {
        Some(i) => (i, String::new()),
        None => (usize::MAX, key.to_string()),
    }
}

/// Overwrite a key's value, keeping everything after it on the line.
///
/// `toml_edit` stores a line's comment in the *value's* decor, so assigning a
/// fresh item to a key takes the comment with it. An edit that quietly deleted
/// whatever the author wrote beside a value would be the same bug this module
/// exists to avoid, one layer down.
///
/// Derived path comments are kept too, stale and all, and that is deliberate.
/// They cannot be regenerated correctly from a single edit — the comment beside
/// a `parent` key spells out a path built from *other* nodes' names, so a
/// rename makes comments elsewhere in the file wrong. Regenerating just the one
/// being written would make an undo land on different bytes from the ones it
/// started with, which costs more than a comment that is briefly out of date.
/// `dim scene fmt` rewrites them all at once, which is the only point they can
/// all be right.
pub fn set_preserving_comment(table: &mut Table, key: &str, item: Item) {
    let suffix = table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|v| v.decor().suffix())
        .and_then(|s| s.as_str())
        .map(str::to_string);
    table[key] = item;
    if let Some(value) = table[key].as_value_mut() {
        value.decor_mut().set_prefix(" ");
        value.decor_mut().set_suffix(suffix.unwrap_or_default());
    }
}

/// Set a key to a rendered literal, keeping any trailing comment on the line.
///
/// The comment is the author's; the value is the engine's. Replacing the whole
/// item would take both.
fn set_field(table: &mut Table, key: &str, rendered: &str) {
    let suffix = trailing_comment(table, key);
    table[key] = parse_item(rendered);
    if let Some(value) = table[key].as_value_mut() {
        value.decor_mut().set_prefix(" ");
        value.decor_mut().set_suffix(suffix.unwrap_or_default());
    }
}

/// Set a key when the value is present, remove it when it is not.
fn optional_field(table: &mut Table, key: &str, rendered: Option<String>) {
    match rendered {
        Some(text) => set_field(table, key, &text),
        None => {
            table.remove(key);
        }
    }
}

/// The trailing comment on a key's line, if it has one that is not a path
/// comment.
///
/// Path comments are derived from ids and regenerated on every format, so
/// keeping the old one would leave a stale note next to a correct id — which is
/// exactly the signal a stale comment is supposed to give.
fn trailing_comment(table: &Table, key: &str) -> Option<String> {
    let raw = table
        .get(key)?
        .as_value()?
        .decor()
        .suffix()?
        .as_str()?
        .to_string();
    let comment = raw.trim();
    if comment.is_empty() || comment.starts_with("# /") || comment.starts_with("# .") {
        return None;
    }
    Some(format!("  {comment}"))
}

/// Parse a rendered literal back into an item.
fn parse_item(rendered: &str) -> Item {
    let text = format!("x = {rendered}");
    let parsed: DocumentMut = text.parse().expect("canonical rendering is valid TOML");
    parsed["x"].clone()
}

fn string_field(table: &Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Reorder an array-of-tables block by a sort key, keeping each table intact.
fn reorder_blocks<K: Ord>(doc: &mut DocumentMut, block: &str, key: impl Fn(&Table) -> K) {
    let Some(existing) = doc.get(block).and_then(Item::as_array_of_tables) else {
        return;
    };
    let mut tables: Vec<Table> = existing.iter().cloned().collect();
    tables.sort_by_key(&key);
    let mut ordered = toml_edit::ArrayOfTables::new();
    for table in tables {
        ordered.push(table);
    }
    doc[block] = Item::ArrayOfTables(ordered);
    normalise_block_spacing(doc, block);
}

/// Overrides group by instance, in node order, then by target.
fn override_sort_key(table: &Table, scene: &Scene) -> (usize, String) {
    let instance = string_field(table, "instance").unwrap_or_default();
    let position = scene
        .walk()
        .iter()
        .position(|id| scene.get(*id).is_some_and(|n| n.uid.to_text() == instance))
        .unwrap_or(usize::MAX);
    (position, string_field(table, "target").unwrap_or_default())
}

/// Connections sort by emitter, then signal, then receiver, then method.
fn connection_sort_key(table: &Table) -> (String, String, String, String) {
    (
        string_field(table, "from").unwrap_or_default(),
        string_field(table, "signal").unwrap_or_default(),
        string_field(table, "to").unwrap_or_default(),
        string_field(table, "method").unwrap_or_default(),
    )
}

/// Chunks sort by layer, then by coordinate.
fn chunk_sort_key(table: &Table) -> (String, i64, i64) {
    let at = table
        .get("at")
        .and_then(Item::as_value)
        .and_then(|v| v.as_array())
        .map(|a| {
            let mut it = a.iter().filter_map(|v| v.as_integer());
            (it.next().unwrap_or(0), it.next().unwrap_or(0))
        })
        .unwrap_or((0, 0));
    (string_field(table, "layer").unwrap_or_default(), at.1, at.0)
}

/// Put exactly one blank line before each table in a block, keeping comments.
///
/// The prefix of a table header holds whatever sits between the previous item
/// and this one, which is where a comment block above a node lives. Comments
/// are kept and the blank lines around them are normalised, so formatting
/// settles instead of drifting.
fn normalise_block_spacing(doc: &mut DocumentMut, block: &str) {
    let Some(tables) = doc.get_mut(block).and_then(Item::as_array_of_tables_mut) else {
        return;
    };
    for table in tables.iter_mut() {
        let existing = table
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        let comments: Vec<&str> = existing
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('#'))
            .collect();
        let mut prefix = String::from("\n");
        for comment in comments {
            prefix.push_str(comment);
            prefix.push('\n');
        }
        table.decor_mut().set_prefix(prefix);
    }
}

/// Rewrite the path comment beside every id reference.
///
/// The id is authoritative and the comment is derived, so it is regenerated on
/// every format. A stale one is then a reliable sign that a file was hand-edited
/// and never formatted.
fn regenerate_reference_comments(
    doc: &mut DocumentMut,
    scene: &Scene,
    resolve_source: Option<&SourceResolver<'_>>,
) {
    let path_of = |uid: &str| -> Option<String> {
        let parsed = NodeUid::parse(uid).ok()?;
        scene.by_uid(parsed).and_then(|id| scene.path_of(id))
    };

    if let Some(tables) = doc.get_mut("node").and_then(Item::as_array_of_tables_mut) {
        for table in tables.iter_mut() {
            // A composite `<instance>/<inner>` parent names a node inside a
            // prefab, so only the instance half can be resolved here.
            let parent = string_field(table, "parent");
            if let Some(parent) = parent {
                let outer = parent.split('/').next().unwrap_or(&parent).to_string();
                comment_on(table, "parent", path_of(&outer));
            }
        }
    }

    if let Some(tables) = doc
        .get_mut("connect")
        .and_then(Item::as_array_of_tables_mut)
    {
        for table in tables.iter_mut() {
            for key in ["from", "to"] {
                let target = string_field(table, key);
                comment_on(table, key, target.and_then(|t| path_of(&t)));
            }
        }
    }

    if let Some(tables) = doc.get_mut("chunk").and_then(Item::as_array_of_tables_mut) {
        for table in tables.iter_mut() {
            let layer = string_field(table, "layer");
            comment_on(table, "layer", layer.and_then(|l| path_of(&l)));
        }
    }

    if let Some(tables) = doc
        .get_mut("override")
        .and_then(Item::as_array_of_tables_mut)
    {
        for table in tables.iter_mut() {
            let instance = string_field(table, "instance");
            let instance_path = instance.as_deref().and_then(path_of);
            comment_on(table, "instance", instance_path);

            // The target lives in the source scene, so resolving it needs the
            // prefab loaded. Without a resolver the comment is left off rather
            // than guessed at.
            let target = string_field(table, "target");
            let source = instance
                .as_deref()
                .and_then(|i| NodeUid::parse(i).ok())
                .and_then(|uid| scene.by_uid(uid))
                .and_then(|id| scene.get(id))
                .and_then(|n| n.scene.clone());
            let resolved = match (source, target.as_deref(), resolve_source) {
                (Some(source), Some(target), Some(resolve)) => NodeUid::parse(target)
                    .ok()
                    .and_then(|uid| resolve(&source, uid)),
                _ => None,
            };
            comment_on(table, "target", resolved);
        }
    }
}

/// Attach or remove the regenerated path comment on one key.
fn comment_on(table: &mut Table, key: &str, path: Option<String>) {
    let Some(item) = table.get_mut(key) else {
        return;
    };
    let Some(value) = item.as_value_mut() else {
        return;
    };
    match path {
        Some(path) => {
            // `Value::to_string` includes the decor, so the old comment has to
            // come off before the new one can be measured against the column.
            let mut bare = value.clone();
            bare.decor_mut().set_prefix("");
            bare.decor_mut().set_suffix("");
            let width = format!("{key} = {}", bare.to_string().trim())
                .chars()
                .count();
            let pad = COMMENT_COLUMN.saturating_sub(width).max(1);
            value
                .decor_mut()
                .set_suffix(format!("{:pad$}# {path}", "", pad = pad));
        }
        None => {
            // Leave a non-path comment alone; clear a stale path one.
            let existing = value
                .decor()
                .suffix()
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if existing.starts_with("# /") || existing.starts_with("# .") {
                value.decor_mut().set_suffix("");
            }
        }
    }
}
