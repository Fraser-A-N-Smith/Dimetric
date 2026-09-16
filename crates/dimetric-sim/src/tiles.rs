//! Reading and writing a tile grid while the simulation runs.
//!
//! # Why writes are deferred, and what that buys on the read side
//!
//! A tile write lands at the end of the tick, for the reason `scene.spawn`
//! does: a script that changed the grid mid-tick would be changing it under
//! every script that had not run yet, and which terrain a monster saw would
//! depend on where it happened to fall in the traversal.
//!
//! The pleasant consequence is that reads need no snapshot. `scene.near` has
//! to read a broadphase built before the scripts ran, because node positions
//! *do* change mid-tick. Tiles do not change mid-tick at all — the queue is
//! drained at a phase boundary — so a read during the tick is already the grid
//! as it stood when the tick began, for free and with nothing copied.
//!
//! # Why this is not the command bus
//!
//! The obvious reading of I1 is that a script painting a tile should issue a
//! `Command::SetTiles`, so that undo and the editor keep working. That is the
//! wrong layer, and the engine already answers it for nodes: `scene.spawn`
//! does not go through the bus either, and neither does a node moving.
//!
//! The bus is the authoring path. `SceneDoc` owns a TOML document alongside the
//! model, and a command keeps the two in step so a file can be written back and
//! undone. A running simulation owns a `Scene` and no document; its job is to
//! evolve state, and every tick already mutates that state without a command in
//! sight. Routing script writes through the bus would mean either maintaining a
//! TOML document for a scene nobody is going to save, or an undo stack that
//! rewinds a game the player is playing.
//!
//! What the request actually wanted from the bus is reproducibility — "a
//! generation bug arrives as a seed" — and that holds without it: the same seed
//! and input log produce the same tile writes in the same order, because the
//! script that made them is itself deterministic. What is shared with the bus
//! is the *chunk handling*, which now lives in `dimetric_scene::chunk` and has
//! exactly one implementation.

use dimetric_core::{HashState, NodeUid, StateHasher};

/// A tile edit waiting for the end of the tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TileEdit {
    /// One cell.
    Set {
        /// The `TileLayer` to write to.
        layer: NodeUid,
        /// Cell coordinate.
        x: i32,
        /// Cell coordinate.
        y: i32,
        /// Tile index.
        tile: u16,
    },
    /// A rectangle of cells, `[x, y, width, height]`.
    ///
    /// Kept as a rectangle rather than expanded into cells when it is queued,
    /// because a generator filling a 40×40 floor would otherwise put 1,600
    /// entries into the state and hash every one of them.
    Fill {
        /// The `TileLayer` to write to.
        layer: NodeUid,
        /// `[x, y, width, height]` in cells.
        rect: [i32; 4],
        /// Tile index.
        tile: u16,
    },
}

impl HashState for TileEdit {
    fn hash_state(&self, h: &mut StateHasher) {
        match self {
            TileEdit::Set { layer, x, y, tile } => {
                h.tag("set")
                    .node_uid(*layer)
                    .i64(*x as i64)
                    .i64(*y as i64)
                    .u64(*tile as u64);
            }
            TileEdit::Fill { layer, rect, tile } => {
                h.tag("fill").node_uid(*layer);
                for v in rect {
                    h.i64(*v as i64);
                }
                h.u64(*tile as u64);
            }
        }
    }
}

/// How many cells one queued fill is allowed to cover.
///
/// A generator writing a floor is the point of this API, so the ceiling is
/// generous; it exists because `rect` comes from a script and `[0, 0,
/// 2_000_000_000, 2_000_000_000]` would otherwise be an allocation the process
/// does not survive. A million cells is a 1000×1000 map, past where the chunk
/// format is honest anyway.
pub const MAX_FILL_CELLS: i64 = 1_000_000;
