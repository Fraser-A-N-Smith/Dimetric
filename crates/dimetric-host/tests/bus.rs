//! The command bus: every mutation invertible, and undo that cannot drift.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_host::{Command, CommandBus};
use dimetric_scene::{KindRegistry, SceneDoc, Value};

const ROOM: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

# Lighting pass still to do.
[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena"

[[node]]
id = "n_floor001"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"
tileset = "asset:tilesets/dungeon"
cell = [16, 16]

[[node]]
id = "n_torch001"
kind = "Light2D"
name = "Torch"
parent = "n_root0000"
pos = [32.0, 16.0]
radius = 48.0
"##;

fn open() -> (SceneDoc, KindRegistry) {
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(ROOM, "room.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    (out.doc.unwrap(), registry)
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

#[test]
fn setting_a_property_and_undoing_restores_the_file_exactly() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::SetProperty {
            id: uid("n_torch001"),
            key: "radius".into(),
            value: Some(Value::Scalar(Fx::from_int(96))),
        },
    )
    .unwrap();
    assert!(doc.to_text().contains("radius = 96.0"));
    // The comment above the first node is untouched by an edit three blocks
    // away, which is the whole reason the document is kept around (I2).
    assert!(doc.to_text().contains("# Lighting pass still to do."));

    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(doc.to_text(), ROOM, "undo should restore the file byte for byte");
}

#[test]
fn creating_and_deleting_a_node_is_a_round_trip() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::CreateNode {
            id: uid("n_newlight"),
            kind: "Light2D".into(),
            name: "Extra".into(),
            parent: Some(uid("n_root0000")),
            props: Default::default(),
        },
    )
    .unwrap();
    assert!(doc.scene.resolve_path("/Arena/Extra").is_some());
    assert!(doc.to_text().contains("n_newlight"));

    bus.undo(&mut doc, &registry).unwrap();
    assert!(doc.scene.resolve_path("/Arena/Extra").is_none());
    assert_eq!(doc.to_text(), ROOM);
}

#[test]
fn deleting_a_subtree_is_undone_node_for_node() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    // Give the torch a child, so the delete has a subtree to capture.
    bus.apply(
        &mut doc,
        &registry,
        Command::CreateNode {
            id: uid("n_glow0001"),
            kind: "Light2D".into(),
            name: "Glow".into(),
            parent: Some(uid("n_torch001")),
            props: Default::default(),
        },
    )
    .unwrap();
    let with_child = doc.to_text();

    bus.apply(
        &mut doc,
        &registry,
        Command::DeleteNode {
            id: uid("n_torch001"),
        },
    )
    .unwrap();
    assert!(doc.scene.resolve_path("/Arena/Torch").is_none());
    assert!(doc.scene.resolve_path("/Arena/Torch/Glow").is_none());

    bus.undo(&mut doc, &registry).unwrap();
    assert!(doc.scene.resolve_path("/Arena/Torch/Glow").is_some());
    assert_eq!(
        doc.scene
            .get(doc.scene.resolve_path("/Arena/Torch").unwrap())
            .unwrap()
            .get("radius"),
        Some(&Value::Scalar(Fx::from_int(48))),
        "the restored node keeps its properties"
    );
    assert_eq!(doc.to_text(), with_child);
}

#[test]
fn redo_replays_what_undo_took_back() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    let rename = Command::RenameNode {
        id: uid("n_torch001"),
        name: "Brazier".into(),
    };
    bus.apply(&mut doc, &registry, rename.clone()).unwrap();
    let after = doc.to_text();

    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(doc.to_text(), ROOM);
    assert_eq!(bus.redo_depth(), 1);

    bus.redo(&mut doc, &registry).unwrap();
    assert_eq!(doc.to_text(), after);
    assert!(doc.scene.resolve_path("/Arena/Brazier").is_some());
}

#[test]
fn a_fresh_edit_clears_the_redo_branch() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::RenameNode {
            id: uid("n_torch001"),
            name: "Brazier".into(),
        },
    )
    .unwrap();
    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(bus.redo_depth(), 1);
    bus.apply(
        &mut doc,
        &registry,
        Command::RenameNode {
            id: uid("n_torch001"),
            name: "Sconce".into(),
        },
    )
    .unwrap();
    assert_eq!(bus.redo_depth(), 0, "you cannot redo into a future you left");
}

#[test]
fn reparenting_and_undoing_puts_the_node_back() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::Reparent {
            id: uid("n_torch001"),
            new_parent: uid("n_floor001"),
        },
    )
    .unwrap();
    assert!(doc.scene.resolve_path("/Arena/Floor/Torch").is_some());
    bus.undo(&mut doc, &registry).unwrap();
    assert!(doc.scene.resolve_path("/Arena/Torch").is_some());
    assert_eq!(doc.to_text(), ROOM);
}

#[test]
fn connections_are_added_and_removed_through_the_bus() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::Connect {
            from: uid("n_torch001"),
            signal: "lit".into(),
            to: uid("n_floor001"),
            method: "on_lit".into(),
        },
    )
    .unwrap();
    assert_eq!(doc.scene.connections.len(), 1);
    assert!(doc.to_text().contains("[[connect]]"));
    bus.undo(&mut doc, &registry).unwrap();
    assert!(doc.scene.connections.is_empty());
    assert_eq!(doc.to_text(), ROOM);
}

#[test]
fn tiles_are_written_per_chunk_and_undone_cell_by_cell() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::FillTiles {
            layer: uid("n_floor001"),
            rect: [0, 0, 4, 4],
            tile: 7,
        },
    )
    .unwrap();
    let text = doc.to_text();
    assert!(text.contains("[[chunk]]"), "a chunk block should appear");
    assert!(text.contains("4:7"), "runs should be encoded: {text}");

    // Paint one cell inside the filled region, then undo just that.
    bus.apply(
        &mut doc,
        &registry,
        Command::SetTiles {
            layer: uid("n_floor001"),
            tiles: vec![(1, 1, 12)],
        },
    )
    .unwrap();
    assert_eq!(tile_at(&doc, 1, 1), Some(12));
    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(tile_at(&doc, 1, 1), Some(7), "undo restores the covered tile");

    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(tile_at(&doc, 0, 0), Some(0));
    assert_eq!(doc.to_text(), ROOM, "an emptied layer writes no chunk blocks");
}

fn tile_at(doc: &SceneDoc, x: i32, y: i32) -> Option<u16> {
    let (at, cell) = dimetric_scene::chunk::split_coord(x, y);
    doc.scene
        .chunks
        .iter()
        .find(|c| c.at == at)
        .and_then(|c| c.get(cell[0], cell[1]))
}

#[test]
fn overrides_are_set_and_cleared_through_the_bus() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    bus.apply(
        &mut doc,
        &registry,
        Command::SetOverride {
            instance: uid("n_torch001"),
            target: uid("n_sk_stats"),
            key: "health".into(),
            value: Value::Int(40),
        },
    )
    .unwrap();
    assert_eq!(doc.scene.overrides.len(), 1);
    assert!(doc.to_text().contains("health = 40"));
    bus.undo(&mut doc, &registry).unwrap();
    assert_eq!(doc.to_text(), ROOM);
}

#[test]
fn a_command_naming_a_missing_node_is_refused_with_a_code() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    let err = bus
        .apply(
            &mut doc,
            &registry,
            Command::RenameNode {
                id: uid("n_nothere0"),
                name: "X".into(),
            },
        )
        .unwrap_err();
    assert_eq!(err.code, dimetric_core::Code::NO_SUCH_NODE);
    assert_eq!(bus.undo_depth(), 0, "a failed command leaves no undo entry");
}

#[test]
fn a_command_cannot_write_a_property_the_schema_does_not_have() {
    // Otherwise the command bus would happily produce a file that the loader
    // rejects with DIM0301, which just moves the failure to the next person.
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    let err = bus
        .apply(
            &mut doc,
            &registry,
            Command::SetProperty {
                id: uid("n_torch001"),
                key: "raduis".into(),
                value: Some(Value::Scalar(Fx::from_int(1))),
            },
        )
        .unwrap_err();
    assert_eq!(err.code, dimetric_core::Code::UNKNOWN_PROPERTY);
}

#[test]
fn a_long_edit_session_undoes_all_the_way_back() {
    let (mut doc, registry) = open();
    let mut bus = CommandBus::new();
    for i in 0..20i64 {
        bus.apply(
            &mut doc,
            &registry,
            Command::SetProperty {
                id: uid("n_torch001"),
                key: "z".into(),
                value: Some(Value::Int(i)),
            },
        )
        .unwrap();
    }
    bus.apply(
        &mut doc,
        &registry,
        Command::SetProperty {
            id: uid("n_torch001"),
            key: "pos".into(),
            value: Some(Value::Vec2(Vec2Fx::from_ints(64, 64))),
        },
    )
    .unwrap();
    assert_eq!(bus.history().len(), 21);
    while bus.undo_depth() > 0 {
        bus.undo(&mut doc, &registry).unwrap();
    }
    assert_eq!(doc.to_text(), ROOM);
}

#[test]
fn commands_round_trip_through_json() {
    // Agents send these over a wire, so they have to survive the trip.
    let commands = vec![
        Command::CreateNode {
            id: uid("n_aaaaaaaa"),
            kind: "Sprite2D".into(),
            name: "Hero".into(),
            parent: Some(uid("n_root0000")),
            props: Default::default(),
        },
        Command::SetProperty {
            id: uid("n_aaaaaaaa"),
            key: "pos".into(),
            value: Some(Value::Vec2(Vec2Fx::from_ints(10, 20))),
        },
        Command::FillTiles {
            layer: uid("n_floor001"),
            rect: [0, 0, 8, 8],
            tile: 3,
        },
    ];
    for command in commands {
        let json = serde_json::to_string(&command).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, command, "{json}");
    }
}
