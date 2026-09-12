//! The command bus.
//!
//! Invariant I1: every operation the editor can perform exists here first. The
//! GUI never mutates engine state directly — it reads state and emits commands,
//! exactly as the CLI and an agent do. One layer, three clients.
//!
//! Every command is serializable and invertible, which is what makes undo fall
//! out of the design rather than being bolted onto it. Because the bus is the
//! *only* mutation path, undo cannot desynchronize from the thing it is undoing.

use dimetric_core::{Code, Diagnostic, NodeUid, Vec2Fx};
use dimetric_scene::chunk::split_coord;
use dimetric_scene::node::{Connection, Override, ParentRef};
use dimetric_scene::value::Value;
use dimetric_scene::{Chunk, KindRegistry, Node, SceneDoc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use toml_edit::{Item, Table};

/// A single mutation.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Add a node.
    CreateNode {
        /// Permanent id. Supplied rather than generated, so that applying a
        /// recorded command log reproduces the same scene.
        id: NodeUid,
        /// Registered node kind.
        kind: String,
        /// Name, unique among siblings.
        name: String,
        /// Parent, or none for the scene root.
        parent: Option<NodeUid>,
        /// Kind properties.
        #[serde(default)]
        props: IndexMap<String, Value>,
    },
    /// Remove a node and its subtree.
    DeleteNode {
        /// Node to remove.
        id: NodeUid,
    },
    /// Set or clear one property.
    SetProperty {
        /// Node to change.
        id: NodeUid,
        /// Property key. Reserved keys are accepted and route to the node's
        /// built-in fields.
        key: String,
        /// New value, or none to clear the key.
        value: Option<Value>,
    },
    /// Move a node under a new parent.
    Reparent {
        /// Node to move.
        id: NodeUid,
        /// New parent.
        new_parent: NodeUid,
    },
    /// Rename a node.
    RenameNode {
        /// Node to rename.
        id: NodeUid,
        /// New name.
        name: String,
    },
    /// Add an `Instance` node pointing at another scene.
    InstancePrefab {
        /// Permanent id for the new instance node.
        id: NodeUid,
        /// Source scene path, without the `scene:` prefix.
        scene: String,
        /// Parent.
        parent: NodeUid,
        /// Name.
        name: String,
        /// Position.
        #[serde(default)]
        pos: Option<Vec2Fx>,
    },
    /// Set one property override on a node inside an instance.
    SetOverride {
        /// The instance node in this scene.
        instance: NodeUid,
        /// The node id in the *source* scene.
        target: NodeUid,
        /// Property key.
        key: String,
        /// New value.
        value: Value,
    },
    /// Clear one property override.
    ClearOverride {
        /// The instance node.
        instance: NodeUid,
        /// The source-scene node id.
        target: NodeUid,
        /// Property key.
        key: String,
    },
    /// Connect a signal to a method.
    Connect {
        /// Emitter.
        from: NodeUid,
        /// Signal name.
        signal: String,
        /// Receiver.
        to: NodeUid,
        /// Method on the receiver's script.
        method: String,
    },
    /// Remove a connection.
    Disconnect {
        /// Emitter.
        from: NodeUid,
        /// Signal name.
        signal: String,
        /// Receiver.
        to: NodeUid,
        /// Method.
        method: String,
    },
    /// Write a script file.
    WriteScript {
        /// Project-relative path.
        path: String,
        /// Full source text.
        source: String,
    },
    /// Import an asset.
    ImportAsset {
        /// Project-relative source path.
        path: String,
    },
    /// Set individual tiles.
    SetTiles {
        /// Target tile layer.
        layer: NodeUid,
        /// `(x, y, tile)` triples, in tile coordinates.
        tiles: Vec<(i32, i32, u16)>,
    },
    /// Fill a rectangle of tiles.
    FillTiles {
        /// Target tile layer.
        layer: NodeUid,
        /// `[x, y, width, height]`, in tile coordinates.
        rect: [i32; 4],
        /// Tile index to write.
        tile: u16,
    },
    /// Open a scene.
    LoadScene {
        /// Project-relative path.
        path: String,
    },
    /// Write the open scene to disk.
    SaveScene {
        /// Path, or none for where it was loaded from.
        #[serde(default)]
        path: Option<String>,
    },
}

impl Command {
    /// The command's name, as the CLI and JSON schemas spell it.
    pub fn name(&self) -> &'static str {
        match self {
            Command::CreateNode { .. } => "create_node",
            Command::DeleteNode { .. } => "delete_node",
            Command::SetProperty { .. } => "set_property",
            Command::Reparent { .. } => "reparent",
            Command::RenameNode { .. } => "rename_node",
            Command::InstancePrefab { .. } => "instance_prefab",
            Command::SetOverride { .. } => "set_override",
            Command::ClearOverride { .. } => "clear_override",
            Command::Connect { .. } => "connect",
            Command::Disconnect { .. } => "disconnect",
            Command::WriteScript { .. } => "write_script",
            Command::ImportAsset { .. } => "import_asset",
            Command::SetTiles { .. } => "set_tiles",
            Command::FillTiles { .. } => "fill_tiles",
            Command::LoadScene { .. } => "load_scene",
            Command::SaveScene { .. } => "save_scene",
        }
    }

    /// True when the command touches the open scene rather than the project.
    ///
    /// Scene commands are undoable; project-level ones such as `LoadScene` are
    /// not, because undoing them would mean restoring files outside the scene
    /// the stack describes.
    pub fn is_scene_edit(&self) -> bool {
        !matches!(
            self,
            Command::LoadScene { .. }
                | Command::SaveScene { .. }
                | Command::ImportAsset { .. }
                | Command::WriteScript { .. }
        )
    }
}

/// What a command changed, and how to change it back.
///
/// An inverse is a list rather than a single command because deleting a
/// subtree has to be undone by recreating every node in it, parents first.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Applied {
    /// The command that was applied.
    pub command: Command,
    /// Commands that, applied in order, undo it.
    pub inverse: Vec<Command>,
}

/// Apply a command to an open scene, returning how to undo it.
pub fn apply(
    doc: &mut SceneDoc,
    registry: &KindRegistry,
    command: &Command,
) -> Result<Vec<Command>, Diagnostic> {
    match command {
        Command::CreateNode {
            id,
            kind,
            name,
            parent,
            props,
        } => {
            if !registry.contains(kind) {
                return Err(Diagnostic::new(
                    Code::UNKNOWN_KIND,
                    format!("no node kind named {kind:?} is registered"),
                )
                .with_field("kind", kind.clone()));
            }
            let parent_id = match parent {
                Some(p) => Some(find(doc, *p)?),
                None => None,
            };
            let mut node = Node::new(*id, kind.clone(), name.clone());
            node.props = props.clone();
            doc.scene.insert(node, parent_id)?;
            write_node_table(doc, registry, *id, kind, name, *parent, props);
            Ok(vec![Command::DeleteNode { id: *id }])
        }

        Command::DeleteNode { id } => {
            let node_id = find(doc, *id)?;
            // Capture the subtree before it goes, so undo can rebuild it
            // exactly — same ids, same parents, same order.
            let order = doc.scene.descendants(node_id);
            let parents: Vec<Option<NodeUid>> = order
                .iter()
                .map(|n| {
                    doc.scene
                        .get(*n)
                        .and_then(|node| node.parent())
                        .and_then(|p| doc.scene.get(p))
                        .map(|p| p.uid)
                })
                .collect();

            // Capture which reserved keys were actually written, so undo
            // restores the file rather than an equivalent-looking one.
            let written: Vec<Vec<String>> = order
                .iter()
                .map(|n| {
                    doc.scene
                        .get(*n)
                        .map(|node| written_reserved_keys(doc, node.uid))
                        .unwrap_or_default()
                })
                .collect();

            let removed = doc.scene.remove_subtree(node_id);
            let mut inverse = Vec::new();
            for ((node, parent), keys) in removed.iter().zip(parents).zip(written) {
                inverse.push(Command::CreateNode {
                    id: node.uid,
                    kind: node.kind.clone(),
                    name: node.name.clone(),
                    parent,
                    props: node.props.clone(),
                });
                // CreateNode carries only the kind properties, so the reserved
                // fields a node had come back as explicit follow-up commands.
                // Keeping CreateNode small is worth the extra entries: it is
                // the command an agent writes by hand most often.
                for key in keys {
                    if let Some(value) = reserved_value(node, &key) {
                        inverse.push(Command::SetProperty {
                            id: node.uid,
                            key,
                            value: Some(value),
                        });
                    }
                }
            }
            // Connections touching a removed node go with it, and come back
            // with it.
            let gone: Vec<NodeUid> = removed.iter().map(|n| n.uid).collect();
            let (kept, dropped): (Vec<Connection>, Vec<Connection>) = doc
                .scene
                .connections
                .iter()
                .cloned()
                .partition(|c| !gone.contains(&c.from) && !gone.contains(&c.to));
            doc.scene.connections = kept;
            for c in dropped {
                inverse.push(Command::Connect {
                    from: c.from,
                    signal: c.signal,
                    to: c.to,
                    method: c.method,
                });
            }
            for uid in &gone {
                remove_node_table(doc, *uid);
            }
            Ok(inverse)
        }

        Command::SetProperty { id, key, value } => {
            let node_id = find(doc, *id)?;
            // Whether the key was *written* matters as much as what it held.
            // Undoing `z = 3` on a node that never had a `z` line must remove
            // the line, not write `z = 0` — otherwise undo leaves a diff.
            let was_written = has_key(doc, *id, key);
            let old = set_property(doc, node_id, key, value.clone(), registry)?;
            write_property(doc, *id, key, value.as_ref());
            Ok(vec![Command::SetProperty {
                id: *id,
                key: key.clone(),
                value: if was_written { old } else { None },
            }])
        }

        Command::Reparent { id, new_parent } => {
            let node_id = find(doc, *id)?;
            let parent_id = find(doc, *new_parent)?;
            let old = doc.scene.reparent(node_id, parent_id)?;
            let old_uid = old.and_then(|p| doc.scene.get(p)).map(|n| n.uid);
            write_raw(doc, *id, "parent", toml_edit::value(new_parent.to_text()));
            match old_uid {
                Some(p) => Ok(vec![Command::Reparent {
                    id: *id,
                    new_parent: p,
                }]),
                None => Err(Diagnostic::new(
                    Code::ILLEGAL_REPARENT,
                    "the scene root cannot be reparented",
                )),
            }
        }

        Command::RenameNode { id, name } => {
            let node_id = find(doc, *id)?;
            let old = doc.scene.rename(node_id, name.clone())?;
            write_raw(doc, *id, "name", toml_edit::value(name.clone()));
            Ok(vec![Command::RenameNode { id: *id, name: old }])
        }

        Command::InstancePrefab {
            id,
            scene,
            parent,
            name,
            pos,
        } => {
            let parent_id = find(doc, *parent)?;
            let mut node = Node::new(*id, "Instance", name.clone());
            node.scene = Some(dimetric_scene::Reference::Scene(scene.clone()));
            if let Some(p) = pos {
                node.transform.pos = *p;
            }
            doc.scene.insert(node, Some(parent_id))?;

            let mut table = Table::new();
            table["id"] = toml_edit::value(id.to_text());
            table["kind"] = toml_edit::value("Instance");
            table["name"] = toml_edit::value(name.clone());
            table["parent"] = toml_edit::value(parent.to_text());
            table["scene"] = toml_edit::value(format!("scene:{scene}"));
            if let Some(p) = pos {
                table["pos"] = vec2_item(*p);
            }
            push_node_table(doc, table);
            Ok(vec![Command::DeleteNode { id: *id }])
        }

        Command::SetOverride {
            instance,
            target,
            key,
            value,
        } => {
            let existing = doc
                .scene
                .overrides
                .iter()
                .position(|o| o.instance == *instance && o.target == *target);
            let old = match existing {
                Some(index) => doc.scene.overrides[index]
                    .props
                    .insert(key.clone(), value.clone()),
                None => {
                    let mut block = Override::new(*instance, *target);
                    block.props.insert(key.clone(), value.clone());
                    doc.scene.overrides.push(block);
                    None
                }
            };
            write_override(doc, *instance, *target, key, Some(value));
            Ok(vec![match old {
                Some(v) => Command::SetOverride {
                    instance: *instance,
                    target: *target,
                    key: key.clone(),
                    value: v,
                },
                None => Command::ClearOverride {
                    instance: *instance,
                    target: *target,
                    key: key.clone(),
                },
            }])
        }

        Command::ClearOverride {
            instance,
            target,
            key,
        } => {
            let mut old = None;
            if let Some(block) = doc
                .scene
                .overrides
                .iter_mut()
                .find(|o| o.instance == *instance && o.target == *target)
            {
                old = block.props.shift_remove(key);
            }
            write_override(doc, *instance, *target, key, None);
            Ok(match old {
                Some(v) => vec![Command::SetOverride {
                    instance: *instance,
                    target: *target,
                    key: key.clone(),
                    value: v,
                }],
                None => Vec::new(),
            })
        }

        Command::Connect {
            from,
            signal,
            to,
            method,
        } => {
            find(doc, *from)?;
            find(doc, *to)?;
            let connection = Connection {
                from: *from,
                signal: signal.clone(),
                to: *to,
                method: method.clone(),
            };
            if !doc.scene.connections.contains(&connection) {
                doc.scene.connections.push(connection);
                let mut table = Table::new();
                table["from"] = toml_edit::value(from.to_text());
                table["signal"] = toml_edit::value(signal.clone());
                table["to"] = toml_edit::value(to.to_text());
                table["method"] = toml_edit::value(method.clone());
                push_table(doc, "connect", table);
            }
            Ok(vec![Command::Disconnect {
                from: *from,
                signal: signal.clone(),
                to: *to,
                method: method.clone(),
            }])
        }

        Command::Disconnect {
            from,
            signal,
            to,
            method,
        } => {
            let before = doc.scene.connections.len();
            doc.scene.connections.retain(|c| {
                !(c.from == *from && c.signal == *signal && c.to == *to && c.method == *method)
            });
            remove_connection_table(doc, *from, signal, *to, method);
            if doc.scene.connections.len() == before {
                return Ok(Vec::new());
            }
            Ok(vec![Command::Connect {
                from: *from,
                signal: signal.clone(),
                to: *to,
                method: method.clone(),
            }])
        }

        Command::SetTiles { layer, tiles } => {
            let mut previous = Vec::with_capacity(tiles.len());
            for (x, y, tile) in tiles {
                let old = write_tile(doc, *layer, *x, *y, *tile)?;
                previous.push((*x, *y, old));
            }
            sync_chunk_tables(doc, *layer);
            Ok(vec![Command::SetTiles {
                layer: *layer,
                tiles: previous,
            }])
        }

        Command::FillTiles { layer, rect, tile } => {
            let [x0, y0, w, h] = *rect;
            let mut previous = Vec::new();
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    previous.push((x, y, write_tile(doc, *layer, x, y, *tile)?));
                }
            }
            sync_chunk_tables(doc, *layer);
            // The inverse of a fill is the tiles it covered, one by one. A fill
            // cannot be undone by another fill: the region was not uniform.
            Ok(vec![Command::SetTiles {
                layer: *layer,
                tiles: previous,
            }])
        }

        Command::WriteScript { .. }
        | Command::ImportAsset { .. }
        | Command::LoadScene { .. }
        | Command::SaveScene { .. } => Err(Diagnostic::new(
            Code::COMMAND_REJECTED,
            format!(
                "{} acts on the project, not the open scene; apply it through the project",
                command.name()
            ),
        )
        .with_field("command", command.name())),
    }
}

fn find(doc: &SceneDoc, uid: NodeUid) -> Result<dimetric_core::NodeId, Diagnostic> {
    doc.scene.by_uid(uid).ok_or_else(|| {
        Diagnostic::new(Code::NO_SUCH_NODE, format!("no node {uid} in this scene"))
            .with_field("id", uid.to_text())
    })
}

/// Write a property, routing reserved keys to the node's own fields.
fn set_property(
    doc: &mut SceneDoc,
    node_id: dimetric_core::NodeId,
    key: &str,
    value: Option<Value>,
    registry: &KindRegistry,
) -> Result<Option<Value>, Diagnostic> {
    let kind = doc
        .scene
        .get(node_id)
        .map(|n| n.kind.clone())
        .unwrap_or_default();

    if !dimetric_scene::schema::is_reserved(key) {
        // Unknown properties are a hard error on load; letting a command write
        // one would just move the failure to the next person who opens the file.
        match registry.get(&kind).and_then(|s| s.property(key)) {
            Some(_) => {}
            None => {
                return Err(Diagnostic::new(
                    Code::UNKNOWN_PROPERTY,
                    format!("{kind} has no property {key:?}"),
                )
                .with_field("kind", kind)
                .with_field("property", key.to_string()))
            }
        }
    }

    let node = doc
        .scene
        .node_mut_no_transform(node_id)
        .ok_or_else(|| Diagnostic::new(Code::NO_SUCH_NODE, "node vanished mid-command"))?;

    // Clearing a reserved key resets it to the value the loader would have
    // assumed had the key been absent.
    let old = match key {
        "pos" => {
            let old = Value::Vec2(node.transform.pos);
            node.transform.pos = value
                .as_ref()
                .and_then(Value::as_vec2)
                .unwrap_or(Vec2Fx::ZERO);
            Some(old)
        }
        "scale" => {
            let old = Value::Vec2(node.transform.scale);
            node.transform.scale = value
                .as_ref()
                .and_then(Value::as_vec2)
                .unwrap_or(Vec2Fx::ONE);
            Some(old)
        }
        "rot" => {
            let old = Value::Angle(node.transform.rot);
            node.transform.rot = value
                .as_ref()
                .and_then(Value::as_angle)
                .unwrap_or(dimetric_core::Angle::ZERO);
            Some(old)
        }
        "visible" => {
            let old = Value::Bool(node.visible);
            node.visible = value.as_ref().and_then(Value::as_bool).unwrap_or(true);
            Some(old)
        }
        "z" => {
            let old = Value::Int(node.z as i64);
            node.z = value.as_ref().and_then(Value::as_int).unwrap_or(0) as i32;
            Some(old)
        }
        "layer" => {
            let old = Value::Int(node.layer as i64);
            node.layer = value.as_ref().and_then(Value::as_int).unwrap_or(0) as i32;
            Some(old)
        }
        _ => match value {
            Some(v) => node.set(key, v),
            None => node.props.shift_remove(key),
        },
    };
    doc.scene.mark_subtree_dirty(node_id);
    Ok(old)
}

// -- document patching --------------------------------------------------
//
// Every edit rewrites the smallest possible region of the `toml_edit`
// document, which is what keeps comments, key order and whitespace intact
// across an edit (I2).

/// Which reserved keys a node's table actually carries, in canonical order.
fn written_reserved_keys(doc: &SceneDoc, uid: NodeUid) -> Vec<String> {
    dimetric_scene::write::reserved_key_order()
        .iter()
        .filter(|k| !matches!(**k, "id" | "kind" | "name" | "parent"))
        .filter(|k| has_key(doc, uid, k))
        .map(|k| k.to_string())
        .collect()
}

/// The current value of one reserved field.
fn reserved_value(node: &Node, key: &str) -> Option<Value> {
    Some(match key {
        "pos" => Value::Vec2(node.transform.pos),
        "rot" => Value::Angle(node.transform.rot),
        "scale" => Value::Vec2(node.transform.scale),
        "visible" => Value::Bool(node.visible),
        "z" => Value::Int(node.z as i64),
        "layer" => Value::Int(node.layer as i64),
        _ => return None,
    })
}

/// True when a node's table carries this key.
fn has_key(doc: &SceneDoc, uid: NodeUid, key: &str) -> bool {
    node_table_index(doc, uid)
        .and_then(|index| {
            doc.doc
                .get("node")?
                .as_array_of_tables()?
                .get(index)?
                .get(key)
                .map(|_| true)
        })
        .unwrap_or(false)
}

fn node_table_index(doc: &SceneDoc, uid: NodeUid) -> Option<usize> {
    let tables = doc.doc.get("node")?.as_array_of_tables()?;
    tables.iter().position(|t| {
        t.get("id")
            .and_then(Item::as_value)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == uid.to_text())
    })
}

fn write_raw(doc: &mut SceneDoc, uid: NodeUid, key: &str, item: Item) {
    let Some(index) = node_table_index(doc, uid) else {
        return;
    };
    let existing = has_key(doc, uid, key);
    if let Some(tables) = doc.doc["node"].as_array_of_tables_mut() {
        if let Some(table) = tables.get_mut(index) {
            table[key] = item;
            // Overwriting a key leaves the author's ordering alone. A *new* key
            // has no authored position, so it goes where `scene fmt` would put
            // it — which also means nodes the CLI writes pass `fmt --check`.
            if !existing {
                sort_canonically(table);
            }
        }
    }
}

/// Order a node table's keys the way canonical form does: the reserved keys in
/// their fixed order, then kind properties alphabetically.
fn sort_canonically(table: &mut Table) {
    table.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
}

fn rank(key: &str) -> (usize, String) {
    match dimetric_scene::write::reserved_key_order()
        .iter()
        .position(|r| *r == key)
    {
        Some(i) => (i, String::new()),
        None => (usize::MAX, key.to_string()),
    }
}

fn write_property(doc: &mut SceneDoc, uid: NodeUid, key: &str, value: Option<&Value>) {
    match value {
        Some(v) => write_raw(doc, uid, key, value_item(v)),
        None => {
            let Some(index) = node_table_index(doc, uid) else {
                return;
            };
            if let Some(tables) = doc.doc["node"].as_array_of_tables_mut() {
                if let Some(table) = tables.get_mut(index) {
                    table.remove(key);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_node_table(
    doc: &mut SceneDoc,
    registry: &KindRegistry,
    id: NodeUid,
    kind: &str,
    name: &str,
    parent: Option<NodeUid>,
    props: &IndexMap<String, Value>,
) {
    let mut table = Table::new();
    table["id"] = toml_edit::value(id.to_text());
    table["kind"] = toml_edit::value(kind);
    table["name"] = toml_edit::value(name);
    if let Some(p) = parent {
        table["parent"] = toml_edit::value(p.to_text());
    }
    let schema = registry.get(kind);
    let mut keys: Vec<&String> = props.keys().collect();
    keys.sort();
    for key in keys {
        // A property equal to its default is left out, exactly as canonical
        // form leaves it out. The loader fills defaults back in, so writing
        // them would only add noise to every diff.
        let is_default = schema
            .and_then(|s| s.property(key))
            .and_then(|p| p.default.as_ref())
            .is_some_and(|d| d == &props[key]);
        if !is_default {
            table[key.as_str()] = value_item(&props[key]);
        }
    }
    sort_canonically(&mut table);
    push_node_table(doc, table);
}

fn push_node_table(doc: &mut SceneDoc, table: Table) {
    push_table(doc, "node", table);
}

fn push_table(doc: &mut SceneDoc, block: &str, table: Table) {
    if doc.doc.get(block).is_none() {
        doc.doc[block] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    if let Some(tables) = doc.doc[block].as_array_of_tables_mut() {
        tables.push(table);
    }
}

fn remove_node_table(doc: &mut SceneDoc, uid: NodeUid) {
    let Some(index) = node_table_index(doc, uid) else {
        return;
    };
    if let Some(tables) = doc.doc["node"].as_array_of_tables_mut() {
        tables.remove(index);
    }
}

fn remove_connection_table(
    doc: &mut SceneDoc,
    from: NodeUid,
    signal: &str,
    to: NodeUid,
    method: &str,
) {
    let Some(tables) = doc.doc.get("connect").and_then(Item::as_array_of_tables) else {
        return;
    };
    let field = |t: &Table, k: &str| {
        t.get(k)
            .and_then(Item::as_value)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let Some(index) = tables.iter().position(|t| {
        field(t, "from") == from.to_text()
            && field(t, "signal") == signal
            && field(t, "to") == to.to_text()
            && field(t, "method") == method
    }) else {
        return;
    };
    if let Some(tables) = doc.doc["connect"].as_array_of_tables_mut() {
        tables.remove(index);
    }
}

fn write_override(
    doc: &mut SceneDoc,
    instance: NodeUid,
    target: NodeUid,
    key: &str,
    value: Option<&Value>,
) {
    let field = |t: &Table, k: &str| {
        t.get(k)
            .and_then(Item::as_value)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let existing = doc
        .doc
        .get("override")
        .and_then(Item::as_array_of_tables)
        .and_then(|tables| {
            tables.iter().position(|t| {
                field(t, "instance") == instance.to_text() && field(t, "target") == target.to_text()
            })
        });

    match (existing, value) {
        (Some(index), Some(v)) => {
            if let Some(tables) = doc.doc["override"].as_array_of_tables_mut() {
                if let Some(table) = tables.get_mut(index) {
                    table[key] = value_item(v);
                }
            }
        }
        (Some(index), None) => {
            if let Some(tables) = doc.doc["override"].as_array_of_tables_mut() {
                let empty = match tables.get_mut(index) {
                    Some(table) => {
                        table.remove(key);
                        table.len() <= 2
                    }
                    None => false,
                };
                if empty {
                    tables.remove(index);
                }
            }
        }
        (None, Some(v)) => {
            let mut table = Table::new();
            table["instance"] = toml_edit::value(instance.to_text());
            table["target"] = toml_edit::value(target.to_text());
            table[key] = value_item(v);
            push_table(doc, "override", table);
        }
        (None, None) => {}
    }
}

/// Write one tile, creating the chunk if it does not exist yet.
fn write_tile(
    doc: &mut SceneDoc,
    layer: NodeUid,
    x: i32,
    y: i32,
    tile: u16,
) -> Result<u16, Diagnostic> {
    find(doc, layer)?;
    let (chunk_at, cell) = split_coord(x, y);
    let index = match doc
        .scene
        .chunks
        .iter()
        .position(|c| c.layer == layer && c.at == chunk_at)
    {
        Some(i) => i,
        None => {
            doc.scene.chunks.push(Chunk::empty(layer, chunk_at));
            doc.scene.chunks.len() - 1
        }
    };
    doc.scene.chunks[index]
        .set(cell[0], cell[1], tile)
        .ok_or_else(|| {
            Diagnostic::new(
                Code::BAD_CHUNK_DATA,
                format!(
                    "chunk at {chunk_at:?} stores its cells externally and cannot be edited yet"
                ),
            )
        })
}

/// Rewrite every `[[chunk]]` block for one layer from the model.
///
/// Chunks are regenerated rather than patched in place: the run-length
/// encoding of a chunk changes shape when a tile in the middle of a run
/// changes, so there is no smaller edit to make.
fn sync_chunk_tables(doc: &mut SceneDoc, layer: NodeUid) {
    let target = layer.to_text();
    if let Some(tables) = doc
        .doc
        .get_mut("chunk")
        .and_then(Item::as_array_of_tables_mut)
    {
        tables.retain(|t| {
            t.get("layer")
                .and_then(Item::as_value)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                != target
        });
    }
    let mut chunks: Vec<(&Chunk, String)> = doc
        .scene
        .chunks
        .iter()
        .filter(|c| c.layer == layer && !c.is_empty())
        .filter_map(|c| {
            c.cells()
                .map(|cells| (c, dimetric_scene::chunk::encode_rle(cells)))
        })
        .collect();
    chunks.sort_by_key(|(c, _)| (c.at[1], c.at[0]));

    let rendered: Vec<Table> = chunks
        .into_iter()
        .map(|(chunk, data)| {
            let mut table = Table::new();
            table["layer"] = toml_edit::value(chunk.layer.to_text());
            let mut at = toml_edit::Array::new();
            at.push(chunk.at[0] as i64);
            at.push(chunk.at[1] as i64);
            table["at"] = toml_edit::value(at);
            table["data"] = toml_edit::value(data);
            table
        })
        .collect();
    for table in rendered {
        push_table(doc, "chunk", table);
    }
}

fn vec2_item(v: Vec2Fx) -> Item {
    value_item(&Value::Vec2(v))
}

/// Render a value as a TOML item, going through canonical rendering so that
/// what a command writes and what `scene fmt` writes cannot drift apart.
fn value_item(value: &Value) -> Item {
    let text = format!("x = {}", dimetric_scene::write::render(value));
    let parsed: toml_edit::DocumentMut = text.parse().expect("canonical rendering is valid TOML");
    parsed["x"].clone()
}

/// Build the parent reference a node should carry.
pub fn parent_ref_of(scene: &dimetric_scene::Scene, id: dimetric_core::NodeId) -> ParentRef {
    match scene.get(id).and_then(|n| n.parent()) {
        Some(p) => scene
            .get(p)
            .map(|n| ParentRef::Node(n.uid))
            .unwrap_or(ParentRef::None),
        None => ParentRef::None,
    }
}
