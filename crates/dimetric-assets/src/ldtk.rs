//! LDtk levels, read for baking.
//!
//! LDtk is an authoring front-end and nothing more. Import bakes its tile
//! layers into native chunk data and the scene never references the `.ldtk`
//! again, which is what keeps native storage canonical: the tile CLI works the
//! same whether a level came from LDtk or from `dim tile fill`, and a painter
//! added later is purely additive because it edits chunks that already exist.
//!
//! The boundary is deliberate. LDtk owns tile layers and the collision grid;
//! `.dim` owns every entity. Import is one-way and never written back. If LDtk
//! owned entities too, `.dim` would be a generated file that could not be
//! edited in its own editor.
//!
//! This module only reads. Turning what it returns into chunks needs the
//! command bus, which lives above it.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A level read out of an LDtk project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Level {
    /// Identifier from the project.
    pub name: String,
    /// Tile layers, in draw order: the first is furthest back.
    pub layers: Vec<TileLayer>,
}

/// One tile layer, flattened to a list of placed tiles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TileLayer {
    /// Identifier from the project.
    pub name: String,
    /// Edge of one cell, in pixels.
    pub grid_size: u32,
    /// Width in cells.
    pub width: i32,
    /// Height in cells.
    pub height: i32,
    /// Tileset this layer draws from, if it has one.
    pub tileset: Option<String>,
    /// Placed tiles, sorted by row then column.
    pub tiles: Vec<Tile>,
}

/// One placed tile.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Tile {
    /// Cell row.
    pub y: i32,
    /// Cell column.
    pub x: i32,
    /// Tile index within the tileset, plus one.
    ///
    /// Chunk data reserves zero for "empty", so every LDtk index is shifted up
    /// by one on the way in. Shifting here rather than at the call site keeps
    /// the reader and the baker from disagreeing about whose job it was.
    pub tile: u32,
    /// Horizontal flip.
    pub flip_x: bool,
    /// Vertical flip.
    pub flip_y: bool,
}

/// Why an LDtk file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum LdtkError {
    /// The file could not be opened.
    #[error("cannot read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The JSON was not an LDtk project this build understands.
    #[error("cannot read {path} as an LDtk project: {detail}")]
    Malformed {
        /// Path that failed.
        path: String,
        /// What went wrong.
        detail: String,
    },
    /// External levels are stored in separate files.
    #[error(
        "{path} stores its levels in separate files; \
         turn off \"Save levels to separate files\" in the LDtk project settings and save again"
    )]
    ExternalLevels {
        /// Path that failed.
        path: String,
    },
}

/// Read every level in an LDtk project.
pub fn read(path: &std::path::Path) -> Result<Vec<Level>, LdtkError> {
    let text = std::fs::File::open(path)
        .and_then(|f| std::io::read_to_string(std::io::BufReader::new(f)))
        .map_err(|e| LdtkError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
    parse(&text, path)
}

/// Read an LDtk project from text already in hand.
pub fn parse(text: &str, path: &std::path::Path) -> Result<Vec<Level>, LdtkError> {
    let project: Project = serde_json::from_str(text).map_err(|e| LdtkError::Malformed {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    if project.external_levels {
        return Err(LdtkError::ExternalLevels {
            path: path.display().to_string(),
        });
    }

    let tilesets: BTreeMap<i64, String> = project
        .defs
        .tilesets
        .iter()
        .map(|t| (t.uid, t.identifier.clone()))
        .collect();

    Ok(project
        .levels
        .iter()
        .map(|level| Level {
            name: level.identifier.clone(),
            // LDtk lists layers front to back; chunks are drawn in the order
            // their layer nodes sit in the scene, so reverse on the way in.
            layers: level
                .layer_instances
                .iter()
                .rev()
                .filter(|l| l.layer_type != "Entities")
                .map(|l| layer_of(l, &tilesets))
                .collect(),
        })
        .collect())
}

fn layer_of(layer: &LayerInstance, tilesets: &BTreeMap<i64, String>) -> TileLayer {
    // A layer places tiles through one of two lists depending on whether the
    // artist painted them or a rule did. Both mean the same thing here.
    let mut placed: Vec<(usize, Tile)> = layer
        .grid_tiles
        .iter()
        .chain(layer.auto_layer_tiles.iter())
        .enumerate()
        .map(|(order, t)| {
            let grid = layer.grid_size.max(1);
            (
                order,
                Tile {
                    x: (t.px.first().copied().unwrap_or(0) / grid) as i32,
                    y: (t.px.get(1).copied().unwrap_or(0) / grid) as i32,
                    tile: (t.t + 1) as u32,
                    flip_x: t.f & 1 != 0,
                    flip_y: t.f & 2 != 0,
                },
            )
        })
        .collect();

    // Row-major, and where a layer stacks several tiles in one cell the last
    // one painted wins — that is what LDtk itself draws, since it paints the
    // list in order, and a chunk holds one tile per cell. Sorting the position
    // in the list descending puts that winner first, so the dedup keeps it.
    placed.sort_by_key(|(order, t)| (t.y, t.x, std::cmp::Reverse(*order)));
    let mut tiles: Vec<Tile> = placed.into_iter().map(|(_, t)| t).collect();
    tiles.dedup_by(|a, b| a.x == b.x && a.y == b.y);

    TileLayer {
        name: layer.identifier.clone(),
        grid_size: layer.grid_size.max(1) as u32,
        width: layer.c_wid as i32,
        height: layer.c_hei as i32,
        tileset: layer
            .tileset_def_uid
            .and_then(|u| tilesets.get(&u).cloned()),
        tiles,
    }
}

// -- the shape of the file ----------------------------------------------
//
// Only the fields that matter for baking. LDtk's JSON carries a great deal
// besides — editor state, entity definitions, level backgrounds — and naming
// all of it would mean a new field breaking the import every time LDtk ships.

#[derive(Deserialize)]
struct Project {
    #[serde(default)]
    defs: Defs,
    #[serde(default)]
    levels: Vec<LevelJson>,
    #[serde(rename = "externalLevels", default)]
    external_levels: bool,
}

#[derive(Deserialize, Default)]
struct Defs {
    #[serde(default)]
    tilesets: Vec<TilesetDef>,
}

#[derive(Deserialize)]
struct TilesetDef {
    uid: i64,
    identifier: String,
}

#[derive(Deserialize)]
struct LevelJson {
    identifier: String,
    #[serde(rename = "layerInstances", default)]
    layer_instances: Vec<LayerInstance>,
}

#[derive(Deserialize)]
struct LayerInstance {
    #[serde(rename = "__identifier")]
    identifier: String,
    #[serde(rename = "__type")]
    layer_type: String,
    #[serde(rename = "__gridSize")]
    grid_size: i64,
    #[serde(rename = "__cWid", default)]
    c_wid: i64,
    #[serde(rename = "__cHei", default)]
    c_hei: i64,
    #[serde(rename = "__tilesetDefUid", default)]
    tileset_def_uid: Option<i64>,
    #[serde(rename = "gridTiles", default)]
    grid_tiles: Vec<TileJson>,
    #[serde(rename = "autoLayerTiles", default)]
    auto_layer_tiles: Vec<TileJson>,
}

#[derive(Deserialize)]
struct TileJson {
    #[serde(default)]
    px: Vec<i64>,
    #[serde(default)]
    t: i64,
    #[serde(default)]
    f: i64,
}
