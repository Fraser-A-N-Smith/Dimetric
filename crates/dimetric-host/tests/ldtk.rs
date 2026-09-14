//! Baking LDtk levels into chunks, through the bus rather than around it.

use dimetric_assets::ldtk;
use dimetric_host::ldtk::{bake, layer_uid};
use dimetric_host::{Command, CommandBus};
use dimetric_scene::{KindRegistry, SceneDoc};

const EMPTY_ROOM: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"
"##;

fn open(text: &str) -> (SceneDoc, KindRegistry) {
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(text, "room.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    (out.doc.unwrap(), registry)
}

fn root() -> dimetric_core::NodeUid {
    dimetric_core::NodeUid::parse("n_root0000").unwrap()
}

/// One level: a Floor with four tiles and a Props layer with one.
fn level() -> ldtk::Level {
    let text = serde_json::json!({
        "defs": { "tilesets": [{ "uid": 1, "identifier": "Dungeon" }] },
        "levels": [{ "identifier": "Arena01", "layerInstances": [
            {
                "__identifier": "Props", "__type": "Tiles", "__gridSize": 16,
                "__cWid": 2, "__cHei": 2, "__tilesetDefUid": 1,
                "gridTiles": [{ "px": [16, 16], "t": 9, "f": 0 }]
            },
            {
                "__identifier": "Floor", "__type": "AutoLayer", "__gridSize": 16,
                "__cWid": 2, "__cHei": 2, "__tilesetDefUid": 1,
                "autoLayerTiles": [
                    { "px": [0, 0], "t": 0, "f": 0 },
                    { "px": [16, 0], "t": 1, "f": 0 },
                    { "px": [0, 16], "t": 2, "f": 0 },
                    { "px": [16, 16], "t": 3, "f": 0 }
                ]
            }
        ]}]
    })
    .to_string();
    ldtk::parse(&text, std::path::Path::new("arena.ldtk"))
        .expect("fixture parses")
        .remove(0)
}

/// Apply a plan and hand back the scene text it produced.
fn run(doc: &mut SceneDoc, registry: &KindRegistry, commands: Vec<Command>) -> CommandBus {
    let mut bus = CommandBus::new();
    for command in commands {
        bus.apply(doc, registry, command).expect("command applies");
    }
    bus
}

#[test]
fn a_level_becomes_tile_layers_and_chunks() {
    let (mut doc, registry) = open(EMPTY_ROOM);
    let plan = bake(&doc.scene, &level(), root(), None);
    assert_eq!(plan.created, ["Floor", "Props"], "back to front");
    run(&mut doc, &registry, plan.commands);

    let text = doc.to_text();
    assert!(text.contains("kind = \"TileLayer\""));
    assert_eq!(text.matches("[[chunk]]").count(), 2);
    let floor = doc
        .scene
        .resolve_path("/Room/Floor")
        .and_then(|id| doc.scene.get(id))
        .expect("the floor layer exists");
    let chunk = doc
        .scene
        .chunks
        .iter()
        .find(|c| c.layer == floor.uid)
        .expect("the floor has a chunk");
    assert_eq!(chunk.get(0, 0), Some(1), "LDtk tile 0 is chunk tile 1");
    assert_eq!(chunk.get(1, 1), Some(4));
}

#[test]
fn nothing_is_written_outside_the_command_bus() {
    // An import that wrote chunk tables directly would be a second mutation
    // path, and undo would stop matching the file.
    let plan = bake(&open(EMPTY_ROOM).0.scene, &level(), root(), None);
    assert!(plan.commands.iter().all(|c| matches!(
        c,
        Command::CreateNode { .. } | Command::SetTiles { .. } | Command::FillTiles { .. }
    )));
}

#[test]
fn an_import_can_be_undone_back_to_the_file_it_started_from() {
    let (mut doc, registry) = open(EMPTY_ROOM);
    let before = doc.to_text();
    let plan = bake(&doc.scene, &level(), root(), None);
    let mut bus = run(&mut doc, &registry, plan.commands);

    while bus.undo_depth() > 0 {
        bus.undo(&mut doc, &registry).expect("undo applies");
    }
    assert_eq!(doc.to_text(), before, "undo goes back to the byte");
}

#[test]
fn reimporting_updates_the_layers_it_made_rather_than_stacking_new_ones() {
    let (mut doc, registry) = open(EMPTY_ROOM);
    let plan = bake(&doc.scene, &level(), root(), None);
    run(&mut doc, &registry, plan.commands);
    let after_first = doc.to_text();

    let plan = bake(&doc.scene, &level(), root(), None);
    assert!(plan.created.is_empty(), "the layers are already there");
    run(&mut doc, &registry, plan.commands);

    assert_eq!(doc.to_text(), after_first, "an unchanged level is a no-op");
    assert_eq!(doc.scene.chunks.len(), 2);
}

#[test]
fn a_tile_erased_in_ldtk_is_erased_here() {
    // Reimport is an update, not a merge. Without the clear, a tile the artist
    // rubbed out would survive in the scene forever.
    let (mut doc, registry) = open(EMPTY_ROOM);
    let plan = bake(&doc.scene, &level(), root(), None);
    run(&mut doc, &registry, plan.commands);

    let mut thinner = level();
    thinner.layers[0].tiles.truncate(1);
    let plan = bake(&doc.scene, &thinner, root(), None);
    run(&mut doc, &registry, plan.commands);

    let floor = doc.scene.resolve_path("/Room/Floor").unwrap();
    let uid = doc.scene.get(floor).unwrap().uid;
    let chunk = doc.scene.chunks.iter().find(|c| c.layer == uid).unwrap();
    assert_eq!(chunk.get(0, 0), Some(1));
    assert_eq!(
        chunk.get(1, 1),
        Some(0),
        "erased in the source, erased here"
    );
}

#[test]
fn the_layer_id_is_derived_from_the_names_so_it_is_stable() {
    let first = layer_uid("Arena01", "Floor");
    assert_eq!(first, layer_uid("Arena01", "Floor"));
    assert_ne!(first, layer_uid("Arena02", "Floor"));
    assert_ne!(first, layer_uid("Arena01", "Props"));
    // And the separator means two names cannot run together into one.
    assert_ne!(layer_uid("ab", "c"), layer_uid("a", "bc"));
}

#[test]
fn the_tileset_can_be_overridden_because_ldtk_names_are_not_asset_paths() {
    let (mut doc, registry) = open(EMPTY_ROOM);
    let plan = bake(&doc.scene, &level(), root(), Some("tilesets/dungeon"));
    run(&mut doc, &registry, plan.commands);
    assert!(doc
        .to_text()
        .contains("tileset = \"asset:tilesets/dungeon\""));
}

#[test]
fn a_layer_whose_id_is_taken_by_something_else_is_refused_rather_than_overwritten() {
    let uid = layer_uid("Arena01", "Floor");
    let scene = format!(
        "{EMPTY_ROOM}\n[[node]]\nid = \"{}\"\nkind = \"Sprite2D\"\nname = \"Floor\"\nparent = \"n_root0000\"\ntexture = \"asset:sprites/x\"\n",
        uid.to_text()
    );
    let (doc, _) = open(&scene);
    let plan = bake(&doc.scene, &level(), root(), None);
    assert!(plan.diagnostics.has_errors());
    assert!(plan.diagnostics.to_string().contains("Sprite2D"));
    // The other layer still bakes: one clash does not abandon the import.
    assert!(plan.created.contains(&"Props".to_string()));
}

#[test]
fn entity_layers_never_reach_the_scene() {
    // The boundary that keeps `.dim` from becoming a generated file.
    let text = serde_json::json!({
        "defs": { "tilesets": [] },
        "levels": [{ "identifier": "L", "layerInstances": [
            { "__identifier": "Spawns", "__type": "Entities", "__gridSize": 16,
              "__cWid": 2, "__cHei": 2 }
        ]}]
    })
    .to_string();
    let level = ldtk::parse(&text, std::path::Path::new("l.ldtk"))
        .unwrap()
        .remove(0);
    let plan = bake(&open(EMPTY_ROOM).0.scene, &level, root(), None);
    assert!(plan.commands.is_empty());
    assert!(plan.created.is_empty());
}
