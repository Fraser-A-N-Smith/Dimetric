//! The `dim` command layer.
//!
//! Two front-ends over one set of commands: the CLI in `main.rs` and the MCP
//! server in [`mcp`]. Both call [`run`], both are clients of
//! [`dimetric_host::Command`] exactly as the GUI editor is (invariant I1), and
//! neither reaches past the bus.

#![warn(missing_docs)]

pub mod cli;
pub mod mcp;
pub mod output;

use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_host::{Command, Project};
use dimetric_scene::Value;
use serde_json::json;

use cli::*;
use output::Output;

/// Carry out one command and return what it produced.
pub fn run(cli: Cli) -> Result<Output, Diagnostics> {
    // The API group describes the engine rather than a project, so it runs
    // before any scene is opened — but it is told where the project is, because
    // a project declares node kinds of its own and an agent asking what it can
    // build should be told about those too.
    if let Top::Api(cmd) = &cli.command {
        return api(cmd, cli.project.as_deref());
    }

    // Creating a project cannot require one to already be open.
    if let Top::New(args) = &cli.command {
        return new_command(args);
    }

    // The server opens whatever project each call names, so it must not need
    // one to start — an agent connects first and decides what to work on after.
    if let Top::Mcp = &cli.command {
        mcp::serve()?;
        return Ok(Output::new(json!({ "served": true }), ""));
    }

    let root = cli.project.clone().unwrap_or_else(|| ".".to_string());
    let mut project = Project::open(&root, cli.id_seed);

    // Every command below the API group needs a scene open.
    let scene_path = cli
        .scene
        .clone()
        .or_else(|| default_scene(&project))
        .ok_or_else(|| {
            one(Diagnostic::new(
                Code::ASSET_MISSING,
                "no scene given and none found; pass --scene",
            ))
        })?;
    let load_diags = project.load_scene(&scene_path)?;

    let mut out = match cli.command {
        Top::Scene(cmd) => scene_command(&mut project, cmd)?,
        Top::Node(cmd) => node_command(&mut project, cmd)?,
        Top::Prefab(cmd) => prefab_command(&mut project, cmd)?,
        Top::Override(cmd) => override_command(&mut project, cmd)?,
        Top::Signal(cmd) => signal_command(&mut project, cmd)?,
        Top::Script(cmd) => script_command(&mut project, cmd)?,
        Top::Tile(cmd) => tile_command(&mut project, cmd)?,
        Top::Asset(cmd) => asset_command(&mut project, cmd)?,
        Top::Run(args) => run_command(&mut project, args)?,
        Top::State(cmd) => state_command(&mut project, cmd)?,
        Top::Frame(cmd) => frame_command(&mut project, cmd)?,
        Top::Replay(args) => replay_command(&mut project, args)?,
        Top::Build(args) => build_command(&mut project, &scene_path, args)?,
        Top::Api(_) | Top::Mcp | Top::New(_) => unreachable!("handled above"),
    };
    out.warnings.extend(load_diags.0);
    Ok(out)
}

/// The first `.dim` in the project root, so a one-scene project needs no flag.
fn default_scene(project: &Project) -> Option<String> {
    let mut candidates: Vec<String> = std::fs::read_dir(&project.root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".dim"))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn one(d: Diagnostic) -> Diagnostics {
    Diagnostics(vec![d])
}

fn uid_of(project: &Project, path: &str) -> Result<dimetric_core::NodeUid, Diagnostics> {
    let doc = project
        .open
        .as_ref()
        .ok_or_else(|| one(Diagnostic::new(Code::COMMAND_REJECTED, "no scene is open")))?;
    // Accept a path or a raw id, because an agent holding an id from a previous
    // command should not have to translate it back into a path.
    if let Ok(uid) = dimetric_core::NodeUid::parse(path) {
        if doc.scene.contains_uid(uid) {
            return Ok(uid);
        }
    }
    doc.scene
        .resolve_path(path)
        .and_then(|id| doc.scene.get(id))
        .map(|n| n.uid)
        .ok_or_else(|| {
            one(
                Diagnostic::new(Code::NO_SUCH_NODE, format!("no node at {path}"))
                    .with_field("path", path.to_string()),
            )
        })
}

fn kind_of(project: &Project, uid: dimetric_core::NodeUid) -> Result<String, Diagnostics> {
    let doc = project.open.as_ref().expect("scene is open");
    doc.scene
        .by_uid(uid)
        .and_then(|id| doc.scene.get(id))
        .map(|n| n.kind.clone())
        .ok_or_else(|| {
            one(Diagnostic::new(
                Code::NO_SUCH_NODE,
                format!("no node {uid}"),
            ))
        })
}

/// Apply a command and write the scene back.
fn apply_and_save(project: &mut Project, command: Command) -> Result<(), Diagnostics> {
    project.apply(command)?;
    project.save_scene(None).map_err(one)?;
    Ok(())
}

// -- scene --------------------------------------------------------------

fn scene_command(project: &mut Project, cmd: SceneCmd) -> Result<Output, Diagnostics> {
    let doc = project.open.as_ref().expect("scene is open");
    match cmd {
        SceneCmd::Tree => {
            let mut text = String::new();
            let mut nodes = Vec::new();
            for id in doc.scene.walk() {
                let node = doc.scene.get(id).expect("walk yields live nodes");
                let depth = doc
                    .scene
                    .path_of(id)
                    .unwrap_or_default()
                    .matches('/')
                    .count()
                    - 1;
                text.push_str(&format!(
                    "{}{} [{}] {}\n",
                    "  ".repeat(depth),
                    node.name,
                    node.kind,
                    node.uid
                ));
                nodes.push(json!({
                    "id": node.uid.to_text(),
                    "kind": node.kind,
                    "name": node.name,
                    "path": doc.scene.path_of(id),
                    "depth": depth,
                }));
            }
            Ok(Output::new(json!({ "nodes": nodes }), text))
        }
        SceneCmd::Query { path } => {
            let uid = uid_of(project, &path)?;
            node_json(project, uid)
        }
        SceneCmd::Fmt { check } => {
            // Canonicalise the parsed document in place rather than rendering a
            // fresh one: a table carries its own comments, so moving it moves
            // them too. Regenerating the text would format the file by deleting
            // everything the author wrote in it.
            let mut formatted = doc.doc.clone();
            dimetric_scene::write::format_in_place(
                &mut formatted,
                &doc.scene,
                &project.registry,
                None,
            );
            let canonical = formatted.to_string();
            let current = doc.to_text();
            if check {
                let canonical_already = current == canonical;
                let text = if canonical_already {
                    "already canonical".to_string()
                } else {
                    "not canonical; run `dim scene fmt`".to_string()
                };
                let out = Output::new(
                    json!({ "canonical": canonical_already, "path": doc.source_path }),
                    text,
                );
                return if canonical_already {
                    Ok(out)
                } else {
                    Err(one(Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        format!("{} is not in canonical form", doc.source_path),
                    )
                    .with_field("path", doc.source_path.clone())))
                };
            }
            let path = doc.source_path.clone();
            std::fs::write(&path, &canonical).map_err(|e| {
                one(Diagnostic::new(
                    Code::COMMAND_REJECTED,
                    format!("cannot write {path}: {e}"),
                ))
            })?;
            Ok(Output::new(
                json!({ "written": path, "changed": current != canonical }),
                format!("formatted {path}"),
            ))
        }
        SceneCmd::Check => Ok(Output::new(
            json!({ "ok": true, "nodes": doc.scene.len() }),
            format!("{} nodes, no errors", doc.scene.len()),
        )),
        SceneCmd::Resolve => {
            let (flat, diags) = project.runtime_scene()?;
            let mut text = String::new();
            let mut nodes = Vec::new();
            for id in flat.walk() {
                let node = flat.get(id).expect("walk yields live nodes");
                let path = flat.path_of(id).unwrap_or_default();
                text.push_str(&format!("{path} [{}]\n", node.kind));
                nodes.push(json!({ "path": path, "kind": node.kind, "id": node.uid.to_text() }));
            }
            let mut out = Output::new(json!({ "nodes": nodes }), text);
            out.warnings = diags.0;
            Ok(out)
        }
    }
}

fn node_json(project: &Project, uid: dimetric_core::NodeUid) -> Result<Output, Diagnostics> {
    let doc = project.open.as_ref().expect("scene is open");
    let id = doc.scene.by_uid(uid).expect("resolved uid");
    let node = doc.scene.get(id).expect("resolved node");
    let mut props = serde_json::Map::new();
    for (key, value) in &node.props {
        props.insert(key.clone(), json!(dimetric_scene::write::render(value)));
    }
    let body = json!({
        "id": node.uid.to_text(),
        "kind": node.kind,
        "name": node.name,
        "path": doc.scene.path_of(id),
        "parent": node.parent().and_then(|p| doc.scene.get(p)).map(|p| p.uid.to_text()),
        "pos": [node.transform.pos.x.to_exact_string(), node.transform.pos.y.to_exact_string()],
        "rot": node.transform.rot.to_degrees_string(),
        "scale": [node.transform.scale.x.to_exact_string(), node.transform.scale.y.to_exact_string()],
        "visible": node.visible,
        "z": node.z,
        "layer": node.layer,
        "tags": node.tags,
        "script": node.script.as_ref().map(|r| r.to_text()),
        "scene": node.scene.as_ref().map(|r| r.to_text()),
        "properties": props,
        "children": doc.scene.children(id).filter_map(|c| doc.scene.get(c)).map(|c| c.name.clone()).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&body).unwrap_or_default();
    Ok(Output::new(body, text))
}

// -- nodes --------------------------------------------------------------

fn node_command(project: &mut Project, cmd: NodeCmd) -> Result<Output, Diagnostics> {
    match cmd {
        NodeCmd::Get { path } => {
            let uid = uid_of(project, &path)?;
            node_json(project, uid)
        }
        NodeCmd::Create {
            kind,
            name,
            parent,
            id,
            set,
        } => {
            let parent_uid = uid_of(project, &parent)?;
            let uid = match id {
                Some(text) => dimetric_core::NodeUid::parse(&text)
                    .map_err(|e| one(Diagnostic::new(Code::BAD_ID_FORM, e.to_string())))?,
                None => project.new_node_id(),
            };
            let mut props = indexmap::IndexMap::new();
            for assignment in &set {
                let (key, value) = split_assignment(assignment)?;
                let parsed =
                    dimetric_scene::parse_property_literal(&project.registry, &kind, &key, &value)
                        .map_err(one)?;
                props.insert(key, parsed);
            }
            apply_and_save(
                project,
                Command::CreateNode {
                    id: uid,
                    kind: kind.clone(),
                    name: name.clone(),
                    parent: Some(parent_uid),
                    props,
                },
            )?;
            Ok(Output::new(
                json!({ "created": uid.to_text(), "kind": kind, "name": name }),
                format!("created {uid} ({kind}) as {parent}/{name}"),
            ))
        }
        NodeCmd::Set { path, key, value } => {
            let uid = uid_of(project, &path)?;
            let kind = kind_of(project, uid)?;
            let parsed =
                dimetric_scene::parse_property_literal(&project.registry, &kind, &key, &value)
                    .map_err(one)?;
            apply_and_save(
                project,
                Command::SetProperty {
                    id: uid,
                    key: key.clone(),
                    value: Some(parsed),
                },
            )?;
            Ok(Output::new(
                json!({ "id": uid.to_text(), "key": key, "value": value }),
                format!("{path}.{key} = {value}"),
            ))
        }
        NodeCmd::Clear { path, key } => {
            let uid = uid_of(project, &path)?;
            apply_and_save(
                project,
                Command::SetProperty {
                    id: uid,
                    key: key.clone(),
                    value: None,
                },
            )?;
            Ok(Output::new(
                json!({ "id": uid.to_text(), "cleared": key }),
                format!("cleared {path}.{key}"),
            ))
        }
        NodeCmd::Reparent { path, parent } => {
            let uid = uid_of(project, &path)?;
            let parent_uid = uid_of(project, &parent)?;
            apply_and_save(
                project,
                Command::Reparent {
                    id: uid,
                    new_parent: parent_uid,
                },
            )?;
            Ok(Output::new(
                json!({ "id": uid.to_text(), "parent": parent_uid.to_text() }),
                format!("moved {path} under {parent}"),
            ))
        }
        NodeCmd::Rename { path, name } => {
            let uid = uid_of(project, &path)?;
            apply_and_save(
                project,
                Command::RenameNode {
                    id: uid,
                    name: name.clone(),
                },
            )?;
            Ok(Output::new(
                json!({ "id": uid.to_text(), "name": name }),
                format!("renamed {path} to {name}"),
            ))
        }
        NodeCmd::Delete { path } => {
            let uid = uid_of(project, &path)?;
            apply_and_save(project, Command::DeleteNode { id: uid })?;
            Ok(Output::new(
                json!({ "deleted": uid.to_text() }),
                format!("deleted {path}"),
            ))
        }
    }
}

fn split_assignment(text: &str) -> Result<(String, String), Diagnostics> {
    text.split_once('=')
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .ok_or_else(|| {
            one(Diagnostic::new(
                Code::BAD_ARGUMENT,
                format!("expected key=value, found {text:?}"),
            )
            .with_field("argument", text.to_string()))
        })
}

// -- prefabs and overrides ----------------------------------------------

fn prefab_command(project: &mut Project, cmd: PrefabCmd) -> Result<Output, Diagnostics> {
    match cmd {
        PrefabCmd::Instance {
            source,
            parent,
            name,
            pos,
            id,
        } => {
            let parent_uid = uid_of(project, &parent)?;
            let uid = match id {
                Some(text) => dimetric_core::NodeUid::parse(&text)
                    .map_err(|e| one(Diagnostic::new(Code::BAD_ID_FORM, e.to_string())))?,
                None => project.new_node_id(),
            };
            let position = match &pos {
                Some(text) => Some(parse_vec2(text)?),
                None => None,
            };
            apply_and_save(
                project,
                Command::InstancePrefab {
                    id: uid,
                    scene: source.clone(),
                    parent: parent_uid,
                    name: name.clone(),
                    pos: position,
                },
            )?;
            Ok(Output::new(
                json!({ "created": uid.to_text(), "scene": source, "name": name }),
                format!("instanced {source} as {parent}/{name}"),
            ))
        }
    }
}

fn override_command(project: &mut Project, cmd: OverrideCmd) -> Result<Output, Diagnostics> {
    match cmd {
        OverrideCmd::Set {
            instance,
            target,
            key,
            value,
        } => {
            let instance_uid = uid_of(project, &instance)?;
            let target_uid = dimetric_core::NodeUid::parse(&target)
                .map_err(|e| one(Diagnostic::new(Code::BAD_ID_FORM, e.to_string())))?;
            // The target lives in the source scene, whose kind is only known
            // once the prefab is loaded, so the literal is parsed loosely here
            // and typed when the instance resolves.
            let parsed =
                dimetric_scene::parse_value_literal(&value, &dimetric_scene::PropertyType::Str)
                    .or_else(|_| {
                        dimetric_scene::parse_value_literal(
                            &value,
                            &dimetric_scene::PropertyType::Int,
                        )
                    })
                    .or_else(|_| {
                        dimetric_scene::parse_value_literal(
                            &value,
                            &dimetric_scene::PropertyType::Scalar,
                        )
                    })
                    .or_else(|_| {
                        dimetric_scene::parse_value_literal(
                            &value,
                            &dimetric_scene::PropertyType::Bool,
                        )
                    })
                    .map_err(one)?;
            apply_and_save(
                project,
                Command::SetOverride {
                    instance: instance_uid,
                    target: target_uid,
                    key: key.clone(),
                    value: parsed,
                },
            )?;
            Ok(Output::new(
                json!({ "instance": instance_uid.to_text(), "target": target, "key": key }),
                format!("override {instance}/{target}.{key} = {value}"),
            ))
        }
        OverrideCmd::Clear {
            instance,
            target,
            key,
        } => {
            let instance_uid = uid_of(project, &instance)?;
            let target_uid = dimetric_core::NodeUid::parse(&target)
                .map_err(|e| one(Diagnostic::new(Code::BAD_ID_FORM, e.to_string())))?;
            apply_and_save(
                project,
                Command::ClearOverride {
                    instance: instance_uid,
                    target: target_uid,
                    key: key.clone(),
                },
            )?;
            Ok(Output::new(
                json!({ "instance": instance_uid.to_text(), "target": target, "cleared": key }),
                format!("cleared override {instance}/{target}.{key}"),
            ))
        }
        OverrideCmd::List { instance } => {
            let instance_uid = uid_of(project, &instance)?;
            let doc = project.open.as_ref().expect("scene is open");
            let blocks: Vec<serde_json::Value> = doc
                .scene
                .overrides
                .iter()
                .filter(|o| o.instance == instance_uid)
                .map(|o| {
                    json!({
                        "target": o.target.to_text(),
                        "removed": o.removed,
                        "properties": o.props.iter()
                            .map(|(k, v)| (k.clone(), json!(dimetric_scene::write::render(v))))
                            .collect::<serde_json::Map<String, serde_json::Value>>(),
                    })
                })
                .collect();
            let text = serde_json::to_string_pretty(&blocks).unwrap_or_default();
            Ok(Output::new(json!({ "overrides": blocks }), text))
        }
    }
}

fn parse_vec2(text: &str) -> Result<dimetric_core::Vec2Fx, Diagnostics> {
    let (x, y) = text.split_once(',').ok_or_else(|| {
        one(Diagnostic::new(
            Code::BAD_ARGUMENT,
            format!("expected `x,y`, found {text:?}"),
        ))
    })?;
    let parse = |s: &str| {
        dimetric_core::Fx::parse_exact(s.trim()).map_err(|e| {
            one(Diagnostic::new(Code::NOT_REPRESENTABLE, e.to_string())
                .with_field("literal", s.trim().to_string()))
        })
    };
    Ok(dimetric_core::Vec2Fx::new(parse(x)?, parse(y)?))
}

fn parse_ints(text: &str, n: usize) -> Result<Vec<i32>, Diagnostics> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != n {
        return Err(one(Diagnostic::new(
            Code::BAD_ARGUMENT,
            format!("expected {n} comma-separated integers, found {text:?}"),
        )));
    }
    parts
        .iter()
        .map(|p| {
            p.parse::<i32>().map_err(|_| {
                one(Diagnostic::new(
                    Code::BAD_ARGUMENT,
                    format!("{p:?} is not an integer"),
                ))
            })
        })
        .collect()
}

// -- signals, scripts, tiles, assets ------------------------------------

fn signal_command(project: &mut Project, cmd: SignalCmd) -> Result<Output, Diagnostics> {
    match cmd {
        SignalCmd::Connect {
            from,
            signal,
            to,
            method,
        } => {
            let from_uid = uid_of(project, &from)?;
            let to_uid = uid_of(project, &to)?;
            apply_and_save(
                project,
                Command::Connect {
                    from: from_uid,
                    signal: signal.clone(),
                    to: to_uid,
                    method: method.clone(),
                },
            )?;
            Ok(Output::new(
                json!({ "from": from_uid.to_text(), "signal": signal, "to": to_uid.to_text(), "method": method }),
                format!("connected {from}.{signal} to {to}.{method}"),
            ))
        }
        SignalCmd::Disconnect {
            from,
            signal,
            to,
            method,
        } => {
            let from_uid = uid_of(project, &from)?;
            let to_uid = uid_of(project, &to)?;
            apply_and_save(
                project,
                Command::Disconnect {
                    from: from_uid,
                    signal: signal.clone(),
                    to: to_uid,
                    method: method.clone(),
                },
            )?;
            Ok(Output::new(
                json!({ "disconnected": true }),
                format!("disconnected {from}.{signal} from {to}.{method}"),
            ))
        }
        SignalCmd::List => {
            let doc = project.open.as_ref().expect("scene is open");
            let list: Vec<serde_json::Value> = doc
                .scene
                .connections
                .iter()
                .map(|c| {
                    json!({
                        "from": c.from.to_text(),
                        "signal": c.signal,
                        "to": c.to.to_text(),
                        "method": c.method,
                    })
                })
                .collect();
            let text = serde_json::to_string_pretty(&list).unwrap_or_default();
            Ok(Output::new(json!({ "connections": list }), text))
        }
    }
}

fn script_command(project: &mut Project, cmd: ScriptCmd) -> Result<Output, Diagnostics> {
    match cmd {
        ScriptCmd::Write { path, source } => {
            let text = match source {
                Some(s) => s,
                None => std::io::read_to_string(std::io::stdin()).map_err(|e| {
                    one(Diagnostic::new(
                        Code::BAD_ARGUMENT,
                        format!("cannot read the script from standard input: {e}"),
                    ))
                })?,
            };
            // Check before writing: a syntax error should be reported with a
            // code, not discovered the next time the game is run.
            let mut host = dimetric_sim::LuaHost::new(60).map_err(one)?;
            host.load(&path, &text).map_err(one)?;
            project.apply(Command::WriteScript {
                path: path.clone(),
                source: text.clone(),
            })?;
            Ok(Output::new(
                json!({ "written": path, "bytes": text.len() }),
                format!("wrote {path} ({} bytes)", text.len()),
            ))
        }
        ScriptCmd::Check { path } => {
            let full = project.path_of(&path);
            let text = std::fs::read_to_string(&full).map_err(|e| {
                one(Diagnostic::new(
                    Code::ASSET_MISSING,
                    format!("cannot read {}: {e}", full.display()),
                ))
            })?;
            let mut host = dimetric_sim::LuaHost::new(60).map_err(one)?;
            host.load(&path, &text).map_err(one)?;
            Ok(Output::new(
                json!({ "ok": true, "path": path }),
                format!("{path} parses"),
            ))
        }
        ScriptCmd::List => {
            project.load_scripts();
            let names: Vec<&String> = project.scripts.keys().collect();
            let text = names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output::new(json!({ "scripts": names }), text))
        }
    }
}

fn tile_command(project: &mut Project, cmd: TileCmd) -> Result<Output, Diagnostics> {
    match cmd {
        TileCmd::Fill { layer, rect, tile } => {
            let layer_uid = uid_of(project, &layer)?;
            let r = parse_ints(&rect, 4)?;
            apply_and_save(
                project,
                Command::FillTiles {
                    layer: layer_uid,
                    rect: [r[0], r[1], r[2], r[3]],
                    tile,
                },
            )?;
            Ok(Output::new(
                json!({ "layer": layer_uid.to_text(), "rect": r, "tile": tile }),
                format!("filled {}x{} tiles with {tile}", r[2], r[3]),
            ))
        }
        TileCmd::Set { layer, at, tile } => {
            let layer_uid = uid_of(project, &layer)?;
            let p = parse_ints(&at, 2)?;
            apply_and_save(
                project,
                Command::SetTiles {
                    layer: layer_uid,
                    tiles: vec![(p[0], p[1], tile)],
                },
            )?;
            Ok(Output::new(
                json!({ "layer": layer_uid.to_text(), "at": p, "tile": tile }),
                format!("set ({}, {}) to {tile}", p[0], p[1]),
            ))
        }
        TileCmd::Get { layer, at } => {
            let layer_uid = uid_of(project, &layer)?;
            let p = parse_ints(&at, 2)?;
            let (chunk_at, cell) = dimetric_scene::chunk::split_coord(p[0], p[1]);
            let doc = project.open.as_ref().expect("scene is open");
            let tile = doc
                .scene
                .chunks
                .iter()
                .find(|c| c.layer == layer_uid && c.at == chunk_at)
                .and_then(|c| c.get(cell[0], cell[1]))
                .unwrap_or(0);
            Ok(Output::new(
                json!({ "layer": layer_uid.to_text(), "at": p, "tile": tile, "chunk": chunk_at }),
                tile.to_string(),
            ))
        }
        TileCmd::ImportLdtk {
            path,
            level,
            into,
            tileset,
            dry_run,
        } => import_ldtk(
            project,
            &path,
            level.as_deref(),
            into.as_deref(),
            tileset.as_deref(),
            dry_run,
        ),
    }
}

/// Bake an LDtk level into the open scene.
fn import_ldtk(
    project: &mut Project,
    path: &str,
    level: Option<&str>,
    into: Option<&str>,
    tileset: Option<&str>,
    dry_run: bool,
) -> Result<Output, Diagnostics> {
    let full = project.root.join(path);
    let levels = dimetric_assets::ldtk::read(&full).map_err(|e| {
        one(Diagnostic::new(Code::ASSET_MISSING, e.to_string())
            .with_field("path", path.to_string()))
    })?;

    let chosen = match level {
        Some(name) => levels.iter().find(|l| l.name == name).ok_or_else(|| {
            let available: Vec<&str> = levels.iter().map(|l| l.name.as_str()).collect();
            one(Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!(
                    "{path} has no level {name:?}; it has {}",
                    available.join(", ")
                ),
            )
            .with_field("level", name.to_string()))
        })?,
        None => levels.first().ok_or_else(|| {
            one(Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("{path} has no levels in it"),
            ))
        })?,
    };

    let parent = match into {
        Some(node) => uid_of(project, node)?,
        None => {
            let doc = project.open.as_ref().expect("scene is open");
            doc.scene
                .root()
                .and_then(|id| doc.scene.get(id))
                .map(|n| n.uid)
                .ok_or_else(|| {
                    one(Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        "this scene has no root to hang the layers off",
                    ))
                })?
        }
    };

    let doc = project.open.as_ref().expect("scene is open");
    let plan = dimetric_host::ldtk::bake(&doc.scene, chosen, parent, tileset);
    if plan.diagnostics.has_errors() {
        return Err(plan.diagnostics);
    }

    let layers: Vec<&str> = chosen.layers.iter().map(|l| l.name.as_str()).collect();
    let tiles: usize = chosen.layers.iter().map(|l| l.tiles.len()).sum();
    let summary = json!({
        "level": chosen.name,
        "layers": layers,
        "created": plan.created,
        "tiles": tiles,
        "commands": plan.commands.len(),
        "applied": !dry_run,
    });

    if dry_run {
        return Ok(Output::new(
            summary,
            format!(
                "would bake {} into {} layer(s), creating {}",
                chosen.name,
                layers.len(),
                plan.created.len()
            ),
        ));
    }

    for command in plan.commands {
        project.apply(command)?;
    }
    project.save_scene(None).map_err(one)?;
    Ok(Output::new(
        summary,
        format!(
            "baked {} tiles from {} into {} layer(s)",
            tiles,
            chosen.name,
            layers.len()
        ),
    ))
}

fn asset_command(project: &mut Project, cmd: AssetCmd) -> Result<Output, Diagnostics> {
    match cmd {
        AssetCmd::List { stale } => {
            project.scan_assets();
            let mut rows = Vec::new();
            let mut text = String::new();
            for entry in project.catalog().entries() {
                if stale && !entry.is_stale() {
                    continue;
                }
                text.push_str(&format!(
                    "{:<28} {:<9} {} {}\n",
                    entry.name,
                    format!("{:?}", entry.kind).to_lowercase(),
                    entry.settings.id,
                    if entry.is_stale() { "stale" } else { "" }
                ));
                rows.push(json!({
                    "name": entry.name,
                    "path": entry.path,
                    "kind": format!("{:?}", entry.kind).to_lowercase(),
                    "id": entry.settings.id.to_string(),
                    "hash": entry.hash,
                    "stale": entry.is_stale(),
                }));
            }
            Ok(Output::new(
                json!({ "assets": rows }),
                text.trim_end().to_string(),
            ))
        }
        AssetCmd::Import { path } => {
            project.apply(Command::ImportAsset { path: path.clone() })?;
            let name = dimetric_assets::cache::asset_name(&path);
            Ok(Output::new(
                json!({ "imported": name, "path": path }),
                format!("imported {name}"),
            ))
        }
        AssetCmd::Reimport { all } => {
            project.scan_assets();
            let before: Vec<String> = if all {
                project
                    .catalog()
                    .entries()
                    .map(|e| e.name.clone())
                    .collect()
            } else {
                project
                    .stale_assets()
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            };
            let imported = project.import_assets();
            let failures: Vec<_> = imported
                .failures
                .iter()
                .map(|(name, why)| json!({ "asset": name, "error": why }))
                .collect();
            let sheet = (imported.sheet.width, imported.sheet.height);
            let text = if before.is_empty() {
                "everything is up to date".to_string()
            } else {
                format!(
                    "imported {} asset(s) into a {}x{} sheet",
                    before.len(),
                    sheet.0,
                    sheet.1
                )
            };
            Ok(Output::new(
                json!({
                    "imported": before,
                    "failures": failures,
                    "sheet": { "width": sheet.0, "height": sheet.1 },
                }),
                text,
            ))
        }
        AssetCmd::Info { name } => {
            project.scan_assets();
            let entry = project.catalog().get(&name).cloned().ok_or_else(|| {
                one(Diagnostic::new(
                    Code::ASSET_MISSING,
                    format!("no asset named {name} in this project"),
                )
                .with_field("asset", name.clone()))
            })?;
            let imported = project.import_assets();
            let clips: Vec<_> = imported
                .clips(&name)
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "frames": c.frames.len(),
                        "ticks": c.duration_ticks(),
                        "looping": c.looping,
                    })
                })
                .collect();
            let placement = imported
                .sheet
                .placements
                .get(&name)
                .map(|p| json!({ "x": p.x, "y": p.y, "width": p.width, "height": p.height }));

            let mut text = format!(
                "{}\n  id     {}\n  kind   {}\n  source {}\n  hash   {}\n",
                entry.name,
                entry.settings.id,
                format!("{:?}", entry.kind).to_lowercase(),
                entry.path,
                entry.hash
            );
            if let Some(p) = &placement {
                text.push_str(&format!(
                    "  atlas  {}x{} at ({}, {})\n",
                    p["width"], p["height"], p["x"], p["y"]
                ));
            }
            for clip in &clips {
                text.push_str(&format!(
                    "  clip   {} — {} frames, {} ticks\n",
                    clip["name"], clip["frames"], clip["ticks"]
                ));
            }
            Ok(Output::new(
                json!({
                    "name": entry.name,
                    "id": entry.settings.id.to_string(),
                    "kind": format!("{:?}", entry.kind).to_lowercase(),
                    "path": entry.path,
                    "hash": entry.hash,
                    "stale": entry.is_stale(),
                    "atlas": placement,
                    "clips": clips,
                }),
                text.trim_end().to_string(),
            ))
        }
    }
}

// -- running ------------------------------------------------------------

/// Build a simulation over the project's resolved scene, with scripts loaded.
fn build_sim(
    project: &mut Project,
    seed: u64,
) -> Result<(dimetric_sim::Sim, Diagnostics), Diagnostics> {
    // Import first: clips carry tick counts baked at import, and a simulation
    // handed no clips animates nothing.
    project.import_assets();
    let clips = project.clips();
    let (templates, template_diags) = project.templates();
    let (scene, mut diags) = project.runtime_scene()?;
    diags.extend(template_diags);
    diags.extend(project.load_scripts());
    // The project's own settings, not the defaults: the tick rate and the
    // canvas are both part of what a recorded run means, so a run started from
    // the CLI has to use the same ones the game will.
    let settings = project.settings.clone();
    diags.extend(project.settings_diagnostics.clone());
    let mut host = dimetric_sim::LuaHost::new(settings.tick_rate).map_err(one)?;
    diags.extend(Diagnostics(load_project_scripts(&mut host, project)));
    let config = dimetric_sim::SimConfig {
        tick_rate: settings.tick_rate,
        canvas: settings.canvas,
    };
    Ok((
        dimetric_sim::Sim::new(scene, seed, Box::new(host), config)
            .with_clips(clips)
            .with_templates(templates),
        diags,
    ))
}

/// Hand a host the project's whole script set at once.
///
/// All of it rather than one at a time, so that `require` resolves against
/// every script rather than the ones that happened to sort earlier.
fn load_project_scripts(host: &mut dimetric_sim::LuaHost, project: &Project) -> Vec<Diagnostic> {
    host.load_all(
        project
            .scripts
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str())),
    )
}

fn read_log(project: &Project, path: &str) -> Result<dimetric_sim::InputLog, Diagnostics> {
    let full = project.path_of(path);
    let text = std::fs::read_to_string(&full)
        .or_else(|_| std::fs::read_to_string(path))
        .map_err(|e| {
            one(Diagnostic::new(
                Code::ASSET_MISSING,
                format!("cannot read input log {path}: {e}"),
            ))
        })?;
    dimetric_sim::InputLog::parse(&text)
        .map_err(|e| one(Diagnostic::new(Code::LOG_MISMATCH, e.to_string())))
}

fn run_command(project: &mut Project, args: RunArgs) -> Result<Output, Diagnostics> {
    let log = match &args.input {
        Some(path) => read_log(project, path)?,
        None => dimetric_sim::InputLog::new(args.seed, env!("CARGO_PKG_VERSION"), 1),
    };
    let seed = args.input.as_ref().map(|_| log.seed).unwrap_or(args.seed);
    // A recorded log knows how long the run was. Truncating it to a default
    // sixty ticks and recording *that* as the run's hashes is a fixture that
    // silently covers the first second of a five-minute game.
    let ticks = args.ticks.unwrap_or(match args.input.is_some() {
        true => log.frames.len() as u64,
        false => 60,
    });
    let (mut sim, diags) = build_sim(project, seed)?;
    let mut warnings = diags.0;

    let mut reloader = args
        .watch
        .then(|| dimetric_host::reload::Reloader::new(dimetric_host::RunMode::Headless, project));
    let mut reloaded = Vec::new();

    let mut hashes = Vec::with_capacity(ticks as usize);
    // Sounds are presentation and a headless run has nowhere to put them, but
    // counting them is how an agent checks that a scene makes a noise without
    // owning a sound card.
    let mut sounds = 0usize;
    // Likewise the lines scripts logged. They are output, not state, so they
    // are drained rather than accumulated in the simulation: a script that
    // logged into the hash would make a run with logging on a different game
    // from one with it off.
    let mut logged: Vec<serde_json::Value> = Vec::new();
    for tick in 0..ticks {
        // Between ticks, never inside one: a tick that picked up a new script
        // half way through would hash to something nobody could reproduce.
        if let Some(reloader) = reloader.as_mut() {
            if reloader.poll(project) > 0 {
                let (applied, diagnostics) = reloader.apply(project, &mut sim);
                warnings.extend(diagnostics.0);
                for script in &applied.scripts {
                    reloaded.push(json!({ "tick": tick, "script": script }));
                }
                for asset in &applied.assets {
                    reloaded.push(json!({ "tick": tick, "asset": asset }));
                }
            }
        }
        sim.step(log.frame(tick));
        sounds += sim.state().sounds.len();
        for line in sim.take_log() {
            logged.push(json!({ "tick": tick, "line": line }));
        }
        hashes.push(sim.hash());
    }
    warnings.extend(sim.take_diagnostics().0);

    if let Some(path) = &args.record {
        let record = dimetric_host::replay::HashLog {
            seed,
            hashes: hashes.clone(),
        };
        let full = project.path_of(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&full, record.to_text()).map_err(|e| {
            one(Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("cannot write {}: {e}", full.display()),
            ))
        })?;
    }

    let final_hash = hashes.last().copied();
    let mut out = Output::new(
        json!({
            "ticks": ticks,
            "seed": seed,
            "hash": final_hash.map(|h| h.to_hex()),
            "recorded": args.record,
            "reloaded": reloaded,
            "sounds": sounds,
            "log": logged,
        }),
        format!(
            "ran {} ticks from seed {seed}; final state {}{}",
            ticks,
            final_hash.map(|h| h.to_hex()).unwrap_or_default(),
            match sounds {
                0 => String::new(),
                1 => "; 1 sound".to_string(),
                n => format!("; {n} sounds"),
            }
        ) + &match logged.len() {
            0 => String::new(),
            _ => logged
                .iter()
                .map(|e| {
                    format!(
                        "\n  tick {}: {}",
                        e["tick"],
                        e["line"].as_str().unwrap_or_default()
                    )
                })
                .collect::<String>(),
        },
    );
    out.warnings = warnings;
    Ok(out)
}

fn state_command(project: &mut Project, cmd: StateCmd) -> Result<Output, Diagnostics> {
    match cmd {
        StateCmd::Dump { tick, seed, input } => {
            let log = match &input {
                Some(path) => read_log(project, path)?,
                None => dimetric_sim::InputLog::new(seed, env!("CARGO_PKG_VERSION"), 1),
            };
            // A log carries the seed it was recorded against, and that wins:
            // dumping with a different one runs a different game and quietly
            // disagrees with `dim run` and `dim replay`, which both prefer it.
            let seed = if input.is_some() { log.seed } else { seed };
            let (mut sim, diags) = build_sim(project, seed)?;
            for t in 0..tick {
                sim.step(log.frame(t));
            }
            let state = sim.state();
            let nodes: Vec<serde_json::Value> = state
                .scene
                .walk()
                .into_iter()
                .filter_map(|id| {
                    let node = state.scene.get(id)?;
                    let vars: serde_json::Map<String, serde_json::Value> = state
                        .vars
                        .get(&node.uid)
                        .map(|v| {
                            v.iter()
                                .map(|(k, val)| {
                                    (k.clone(), json!(dimetric_scene::write::render(val)))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(json!({
                        "path": state.scene.path_of(id),
                        "id": node.uid.to_text(),
                        "kind": node.kind,
                        "pos": [
                            node.transform.pos.x.to_exact_string(),
                            node.transform.pos.y.to_exact_string(),
                        ],
                        "rot": node.transform.rot.to_degrees_string(),
                        "visible": node.visible,
                        "properties": node.props.iter()
                            .map(|(k, v)| (k.clone(), json!(dimetric_scene::write::render(v))))
                            .collect::<serde_json::Map<_, _>>(),
                        "vars": vars,
                    }))
                })
                .collect();
            let body = json!({
                "tick": state.tick.0,
                "seed": seed,
                "hash": state.hash().to_hex(),
                "nodes": nodes,
            });
            let text = serde_json::to_string_pretty(&body).unwrap_or_default();
            let mut out = Output::new(body, text);
            out.warnings = diags.0;
            Ok(out)
        }
        StateCmd::Hash { tick, seed } => {
            let (mut sim, _) = build_sim(project, seed)?;
            let log = dimetric_sim::InputLog::new(seed, env!("CARGO_PKG_VERSION"), 1);
            for t in 0..tick {
                sim.step(log.frame(t));
            }
            let hash = sim.hash();
            Ok(Output::new(
                json!({ "tick": tick, "seed": seed, "hash": hash.to_hex() }),
                hash.to_hex(),
            ))
        }
    }
}

fn frame_command(project: &mut Project, cmd: FrameCmd) -> Result<Output, Diagnostics> {
    let FrameCmd::Capture {
        tick,
        png,
        seed,
        input,
        width,
        height,
        internal,
        no_integer_upscale,
        ambient,
    } = cmd;

    let mut settings = dimetric_render::RenderSettings {
        integer_upscale: !no_integer_upscale,
        ..Default::default()
    };
    if let Some(text) = &internal {
        settings.internal_resolution = parse_size(text)?;
    }
    if let Some(text) = &ambient {
        settings.ambient = dimetric_scene::Color::parse(text).map_err(|e| {
            one(Diagnostic::new(
                Code::BAD_ARGUMENT,
                format!("--ambient {text:?}: {e}"),
            ))
        })?;
    }

    let log = match &input {
        Some(path) => Some(read_log(project, path)?),
        None => None,
    };
    let captured = dimetric_host::capture(
        project,
        dimetric_host::CaptureRequest {
            tick,
            seed,
            input: log,
            size: (width.max(1), height.max(1)),
            settings,
        },
    )?;

    let path = project.path_of(&png);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    dimetric_render::write_png(&path, captured.width, captured.height, &captured.pixels)
        .map_err(|e| one(Diagnostic::new(Code::COMMAND_REJECTED, e.to_string())))?;

    let mut out = Output::new(
        json!({
            "png": path.display().to_string(),
            "tick": tick,
            "seed": seed,
            "size": [captured.width, captured.height],
            "internal": [settings.internal_resolution.0, settings.internal_resolution.1],
            "sprites": captured.sprites,
            "draw_calls": captured.draw_calls,
            "adapter": captured.adapter,
        }),
        format!(
            "captured tick {tick} to {} ({}x{}, {} sprites in {} draw calls, via {})",
            path.display(),
            captured.width,
            captured.height,
            captured.sprites,
            captured.draw_calls,
            captured.adapter
        ),
    );
    out.warnings = captured.diagnostics.0;
    Ok(out)
}

/// Parse a `WxH` size argument.
fn parse_size(text: &str) -> Result<(u32, u32), Diagnostics> {
    let (w, h) = text.split_once(['x', 'X']).ok_or_else(|| {
        one(Diagnostic::new(
            Code::BAD_ARGUMENT,
            format!("expected a size like 480x270, found {text:?}"),
        ))
    })?;
    let parse = |s: &str| {
        s.trim().parse::<u32>().map_err(|_| {
            one(Diagnostic::new(
                Code::BAD_ARGUMENT,
                format!("{s:?} is not a number of pixels"),
            ))
        })
    };
    Ok((parse(w)?.max(1), parse(h)?.max(1)))
}

fn replay_command(project: &mut Project, args: ReplayArgs) -> Result<Output, Diagnostics> {
    let log = read_log(project, &args.input)?;
    let recorded = match &args.hashes {
        Some(path) => {
            let full = project.path_of(path);
            let text = std::fs::read_to_string(&full)
                .or_else(|_| std::fs::read_to_string(path))
                .map_err(|e| {
                    one(Diagnostic::new(
                        Code::ASSET_MISSING,
                        format!("cannot read {path}: {e}"),
                    ))
                })?;
            Some(dimetric_host::replay::HashLog::parse(&text).map_err(one)?)
        }
        None => None,
    };
    let probes = match &args.assert {
        Some(path) => {
            let full = project.path_of(path);
            let text = std::fs::read_to_string(&full)
                .or_else(|_| std::fs::read_to_string(path))
                .map_err(|e| {
                    one(Diagnostic::new(
                        Code::ASSET_MISSING,
                        format!("cannot read {path}: {e}"),
                    ))
                })?;
            dimetric_host::replay::parse_probes(&text).map_err(one)?
        }
        None => Vec::new(),
    };

    project.import_assets();
    let clips = project.clips();
    let (templates, template_diags) = project.templates();
    let (scene, mut diags) = project.runtime_scene()?;
    diags.extend(template_diags);
    diags.extend(project.load_scripts());
    let mut host = dimetric_sim::LuaHost::new(60).map_err(one)?;
    diags.extend(Diagnostics(load_project_scripts(&mut host, project)));

    let replay = dimetric_host::Replay {
        log: &log,
        ticks: args.ticks,
        expected: recorded.as_ref().map(|r| r.hashes.as_slice()),
        probes: &probes,
        clips,
        templates,
    };
    // Same settings the run used: replaying a log under a different tick rate
    // or canvas is not replaying it.
    let report = replay.run(
        scene,
        Box::new(host),
        dimetric_sim::SimConfig {
            tick_rate: project.settings.tick_rate,
            canvas: project.settings.canvas,
        },
    );

    let mut text = format!("replayed {} ticks from seed {}", report.ticks, report.seed);
    for probe in &report.probes {
        text.push_str(&format!("\n  {probe}"));
    }
    if let Some(d) = &report.divergence {
        text.push_str(&format!("\n{}", dimetric_host::replay::describe(d)));
    }

    let body = serde_json::to_value(&report).unwrap_or(json!({}));
    if !report.passed() {
        let mut failures = Diagnostics::new();
        if let Some(d) = &report.divergence {
            failures.push(
                Diagnostic::new(Code::REPLAY_DIVERGED, dimetric_host::replay::describe(d))
                    .with_field("tick", d.tick as i64)
                    .with_field("expected", d.expected.to_hex())
                    .with_field("actual", d.actual.to_hex()),
            );
        }
        for probe in report.probes.iter().filter(|p| !p.passed) {
            failures.push(
                Diagnostic::new(Code::PROBE_FAILED, probe.to_string())
                    .with_field("tick", probe.probe.tick as i64)
                    .with_field("path", probe.probe.path.clone())
                    .with_field("field", probe.probe.field.clone())
                    .with_field("expected", probe.probe.value.clone())
                    .with_field("found", probe.found.clone()),
            );
        }
        failures.extend(report.diagnostics.clone());
        return Err(failures);
    }

    let mut out = Output::new(body, text);
    out.warnings = diags.0;
    // A replay that passed can still have had something to say — an engine
    // version that does not match the log's, say. `passed()` only looks for
    // errors, so without this a warning raised during a successful replay went
    // nowhere at all.
    out.warnings.extend(report.diagnostics.0);
    Ok(out)
}

fn build_command(
    project: &mut Project,
    scene: &str,
    args: BuildArgs,
) -> Result<Output, Diagnostics> {
    use dimetric_host::package;

    let platform = package::platform(&args.target).ok_or_else(|| {
        let known: Vec<&str> = package::PLATFORMS.iter().map(|p| p.name).collect();
        one(Diagnostic::new(
            Code::BAD_ARGUMENT,
            format!(
                "no target called {:?}; this build packages for {}",
                args.target,
                known.join(", ")
            ),
        )
        .with_field("target", args.target.clone()))
    })?;

    // The scene the CLI already opened, so a project that does not load is
    // refused before anything is copied rather than packaged broken.
    let scene = if scene.ends_with(".dim") {
        scene.to_string()
    } else {
        format!("{scene}.dim")
    };

    let staged = package::stage(
        project,
        package::PackageRequest {
            platform,
            scene: scene.clone(),
            seed: args.seed,
            out: args.out.as_ref().map(std::path::PathBuf::from),
            runtime: args.runtime.as_ref().map(std::path::PathBuf::from),
        },
    )?;

    let text = format!(
        "staged {} files ({:.1} MB) for {} in {}",
        staged.files.len(),
        staged.bytes as f64 / 1_048_576.0,
        platform.name,
        staged.out.display()
    );
    let mut out = Output::new(
        json!({
            "target": platform.name,
            "triple": platform.triple,
            "out": staged.out.display().to_string(),
            "scene": scene,
            "seed": args.seed,
            "files": staged.files,
            "bytes": staged.bytes,
            "runtime": staged.runtime.as_ref().map(|p| p.display().to_string()),
        }),
        text,
    );
    out.warnings.extend(staged.diagnostics.0);
    Ok(out)
}

/// Write a new project to start from.
fn new_command(args: &NewArgs) -> Result<Output, Diagnostics> {
    let path = std::path::PathBuf::from(&args.path);
    let name = match &args.name {
        Some(name) => name.clone(),
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Game")
            .to_string(),
    };
    let created = dimetric_host::template::create(&path, &name)?;
    let text = format!(
        "wrote {} files to {}\n\n  dim --project {} run --headless --ticks 60\n  dim-play {}",
        created.files.len(),
        created.root.display(),
        created.root.display(),
        created.root.display(),
    );
    Ok(Output::new(
        json!({
            "root": created.root.display().to_string(),
            "name": name,
            "files": created.files,
        }),
        text,
    ))
}

// -- generated reference ------------------------------------------------

fn api(cmd: &ApiCmd, project: Option<&str>) -> Result<Output, Diagnostics> {
    match cmd {
        ApiCmd::Codes => {
            let list: Vec<serde_json::Value> = dimetric_core::diag::CODES
                .iter()
                .map(|c| {
                    json!({
                        "code": c.code.0,
                        "severity": c.severity.to_string(),
                        "summary": c.summary,
                    })
                })
                .collect();
            let text = dimetric_core::diag::CODES
                .iter()
                .map(|c| format!("{} {:8} {}", c.code, c.severity.to_string(), c.summary))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output::new(json!({ "codes": list }), text))
        }
        ApiCmd::Kinds => {
            // The project's registry when there is one, so `kinds.toml` shows
            // up beside the built-ins rather than an agent having to know that
            // the two lists exist separately.
            let registry = match project {
                Some(root) => Project::open(root, 0).registry,
                None => dimetric_scene::KindRegistry::with_builtins(),
            };
            let list: Vec<serde_json::Value> = registry
                .iter()
                .map(|k| {
                    json!({
                        "kind": k.kind,
                        "doc": k.doc,
                        "properties": k.properties.iter().map(|p| json!({
                            "name": p.name,
                            "type": p.ty.name(),
                            "required": p.required,
                            "default": p.default.as_ref().map(dimetric_scene::write::render),
                            "doc": p.doc,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let text = registry
                .iter()
                .map(|k| {
                    let props = k
                        .properties
                        .iter()
                        .map(|p| format!("    {} : {}", p.name, p.ty.name()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("{}\n{props}", k.kind)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output::new(json!({ "kinds": list }), text))
        }
        ApiCmd::Commands => {
            let names = command_names();
            Ok(Output::new(json!({ "commands": names }), names.join("\n")))
        }
        ApiCmd::Schema => {
            let schema = command_schema();
            let text = serde_json::to_string_pretty(&schema).unwrap_or_default();
            Ok(Output::new(schema, text))
        }
        ApiCmd::Tools => {
            let tools = mcp::tools();
            let mut text = String::new();
            for tool in &tools {
                text.push_str(&format!("{}\n", tool.name));
            }
            let list: Vec<serde_json::Value> = tools.iter().map(mcp::Tool::describe).collect();
            Ok(Output::new(json!({ "tools": list }), text))
        }
    }
}

/// Every command name, taken from the enum itself so the list cannot go stale.
pub fn command_names() -> Vec<&'static str> {
    sample_commands().iter().map(Command::name).collect()
}

/// The command set as a JSON schema, generated from real values.
///
/// Generated rather than hand-written, so `docs/API.md` and the schemas cannot
/// drift from the code an agent is actually talking to (I10).
pub fn command_schema() -> serde_json::Value {
    let variants: Vec<serde_json::Value> = sample_commands()
        .iter()
        .map(|c| {
            let value = serde_json::to_value(c).unwrap_or(json!({}));
            let fields: Vec<String> = value
                .as_object()
                .map(|o| o.keys().filter(|k| *k != "command").cloned().collect())
                .unwrap_or_default();
            json!({
                "name": c.name(),
                "fields": fields,
                "example": value,
                "undoable": c.is_scene_edit(),
            })
        })
        .collect();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commands": variants,
    })
}

/// One of each command, used to generate the schema and the command list.
fn sample_commands() -> Vec<Command> {
    let node = dimetric_core::NodeUid::parse("n_example0").expect("valid sample id");
    vec![
        Command::CreateNode {
            id: node,
            kind: "Sprite2D".into(),
            name: "Hero".into(),
            parent: Some(node),
            props: indexmap::IndexMap::new(),
        },
        Command::DeleteNode { id: node },
        Command::SetProperty {
            id: node,
            key: "radius".into(),
            value: Some(Value::Scalar(dimetric_core::Fx::from_int(72))),
        },
        Command::Reparent {
            id: node,
            new_parent: node,
        },
        Command::RenameNode {
            id: node,
            name: "Hero".into(),
        },
        Command::InstancePrefab {
            id: node,
            scene: "prefabs/skeleton".into(),
            parent: node,
            name: "Skeleton_01".into(),
            pos: None,
        },
        Command::SetOverride {
            instance: node,
            target: node,
            key: "health".into(),
            value: Value::Int(40),
        },
        Command::ClearOverride {
            instance: node,
            target: node,
            key: "health".into(),
        },
        Command::Connect {
            from: node,
            signal: "died".into(),
            to: node,
            method: "on_enemy_died".into(),
        },
        Command::Disconnect {
            from: node,
            signal: "died".into(),
            to: node,
            method: "on_enemy_died".into(),
        },
        Command::WriteScript {
            path: "scripts/enemy.lua".into(),
            source: String::new(),
        },
        Command::ImportAsset {
            path: "assets/sprites/hero.png".into(),
        },
        Command::SetTiles {
            layer: node,
            tiles: vec![(0, 0, 1)],
        },
        Command::FillTiles {
            layer: node,
            rect: [0, 0, 8, 8],
            tile: 1,
        },
        Command::LoadScene {
            path: "arena01".into(),
        },
        Command::SaveScene { path: None },
    ]
}
