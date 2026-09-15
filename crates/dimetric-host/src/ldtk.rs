//! Baking an LDtk level into native chunks.
//!
//! LDtk is an authoring front-end and the import is one way. What comes out of
//! this module is a list of [`Command`]s, which is the point: an agent that
//! imports a level goes through the same bus as an agent that runs `tile fill`,
//! with the same validation and the same undo. Writing chunk tables directly
//! would be a second mutation path, and a second mutation path is how undo
//! starts disagreeing with the file.
//!
//! # What it touches
//!
//! Tile layers, and nothing else. LDtk owns those; `.dim` owns every entity.
//! A layer becomes a `TileLayer` node whose id is derived from the level and
//! layer names, so importing the same level twice updates the layer in place
//! rather than stacking a second copy beside it.
//!
//! # Undo
//!
//! Every command is individually invertible, so an import can be undone — one
//! `dim undo` per command. The bus has no notion of a transaction, and
//! inventing one just for import would mean a second kind of history entry.

use dimetric_assets::ldtk::{Level, TileLayer};
use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid};
use dimetric_scene::chunk::EMPTY_TILE;
use dimetric_scene::{Scene, Value};

use crate::command::Command;

/// What an import would do.
pub struct Bake {
    /// Commands to apply, in order.
    pub commands: Vec<Command>,
    /// Layers that will be created rather than updated.
    pub created: Vec<String>,
    /// Anything wrong with the level that did not stop the bake.
    pub diagnostics: Diagnostics,
}

/// Plan the commands that bake one level into a scene.
///
/// `parent` is the node the layers hang off. Nothing is applied here — the
/// caller runs the commands through the bus, which is what gives them undo.
pub fn bake(scene: &Scene, level: &Level, parent: NodeUid, tileset: Option<&str>) -> Bake {
    let mut commands = Vec::new();
    let mut created = Vec::new();
    let mut diagnostics = Diagnostics::new();

    for layer in &level.layers {
        let uid = layer_uid(&level.name, &layer.name);
        let existing = scene.by_uid(uid).and_then(|id| scene.get(id));

        match existing {
            Some(node) if node.base != "TileLayer" => {
                diagnostics.push(
                    Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        format!(
                            "{} is a {} in this scene, so layer {:?} has nowhere to bake to",
                            uid.to_text(),
                            node.kind,
                            layer.name
                        ),
                    )
                    .with_field("layer", layer.name.clone()),
                );
                continue;
            }
            Some(_) => {}
            None => {
                created.push(layer.name.clone());
                commands.push(Command::CreateNode {
                    id: uid,
                    kind: "TileLayer".to_string(),
                    name: layer.name.clone(),
                    parent: Some(parent),
                    props: layer_props(layer, tileset),
                });
            }
        }

        // Clearing first is what makes a reimport an update rather than a
        // merge: a tile the artist erased in LDtk has to disappear here too,
        // and an erased tile is absent from the file rather than present as a
        // zero.
        if let Some(rect) = occupied(scene, uid, layer) {
            commands.push(Command::FillTiles {
                layer: uid,
                rect,
                tile: EMPTY_TILE,
            });
        }

        let mut tiles = Vec::with_capacity(layer.tiles.len());
        for tile in &layer.tiles {
            match u16::try_from(tile.tile) {
                Ok(index) => tiles.push((tile.x, tile.y, index)),
                Err(_) => diagnostics.push(
                    Diagnostic::new(
                        Code::OUT_OF_RANGE,
                        format!(
                            "tile index {} in layer {:?} does not fit in a chunk cell; \
                             a tileset can hold {} tiles",
                            tile.tile - 1,
                            layer.name,
                            u16::MAX - 1
                        ),
                    )
                    .with_field("layer", layer.name.clone()),
                ),
            }
        }
        if !tiles.is_empty() {
            commands.push(Command::SetTiles { layer: uid, tiles });
        }
    }

    Bake {
        commands,
        created,
        diagnostics,
    }
}

/// The rectangle a reimport has to clear before writing.
///
/// The union of what the layer already covers in the scene and what the level
/// is about to cover, so tiles outside the new level's bounds are erased too.
/// `None` when there is nothing to clear.
fn occupied(scene: &Scene, uid: NodeUid, layer: &TileLayer) -> Option<[i32; 4]> {
    let mut bounds: Option<[i32; 4]> = if layer.width > 0 && layer.height > 0 {
        Some([0, 0, layer.width, layer.height])
    } else {
        None
    };
    for chunk in scene.chunks.iter().filter(|c| c.layer == uid) {
        let size = dimetric_scene::CHUNK_SIZE;
        let chunk_box = [chunk.at[0] * size, chunk.at[1] * size, size, size];
        bounds = Some(match bounds {
            None => chunk_box,
            Some(b) => union(b, chunk_box),
        });
    }
    bounds
}

fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    let right = (a[0] + a[2]).max(b[0] + b[2]);
    let bottom = (a[1] + a[3]).max(b[1] + b[3]);
    [x, y, right - x, bottom - y]
}

fn layer_props(layer: &TileLayer, tileset: Option<&str>) -> indexmap::IndexMap<String, Value> {
    let mut props = indexmap::IndexMap::new();
    // `tileset` is required on the kind, so a layer whose tileset LDtk did not
    // name still gets one: a broken reference reports itself where it is drawn,
    // whereas a missing required property refuses to load the scene at all.
    let name = tileset
        .or(layer.tileset.as_deref())
        .unwrap_or("tilesets/missing");
    props.insert(
        "tileset".to_string(),
        Value::Ref(dimetric_scene::Reference::Asset(name.to_string())),
    );
    props.insert(
        "cell".to_string(),
        Value::Vec2i([layer.grid_size as i32, layer.grid_size as i32]),
    );
    props
}

/// The id a level's layer node gets.
///
/// Derived from the two names rather than drawn from the RNG, so importing the
/// same level twice writes to the same node. An import that invented a fresh id
/// each time would stack a new layer on the old one every reimport.
pub fn layer_uid(level: &str, layer: &str) -> NodeUid {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut h = blake3::Hasher::new();
    h.update(b"dimetric.ldtk.layer");
    h.update(level.as_bytes());
    h.update(&[0]);
    h.update(layer.as_bytes());
    let mut bits = u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().expect("8 bytes"));
    let mut body = String::from("n_");
    for _ in 0..8 {
        body.push(ALPHABET[(bits & 31) as usize] as char);
        bits >>= 5;
    }
    NodeUid::parse(&body).expect("derived ids use the generation alphabet")
}
