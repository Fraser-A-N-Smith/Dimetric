//! Tile animation: the slices a painted tile id cycles through.
//!
//! A tileset is a sheet of slices and a map cell holds one id that picks one of
//! them. That is enough for a wall and not enough for water, a torch or a
//! portal, which are one tile to the map and several slices to the eye.
//!
//! # Why none of this is state
//!
//! The obvious implementation is to advance the map: step every animated cell
//! to its next id each tick and let the existing draw path do the rest. It is
//! also the wrong one, twice over. A rollback would have to undo a ripple,
//! every snapshot would carry a decoration, and the state hash of a replay
//! would then depend on the *art* — change a `.meta`, and a recorded run that
//! played identically no longer verifies.
//!
//! So the cycle is a fact about the sheet, declared in the `.meta`, baked here,
//! and consulted at draw time. [`Animations::frame_at`] is a pure function of a
//! tile id and a tick, which is what makes it safe: the renderer asks what to
//! draw, nothing writes anything down, and `dim frame capture --tick N` gets
//! the same answer on every machine because the tick is the same integer.
//!
//! # Ticks, not milliseconds
//!
//! The sidecar's timings are in milliseconds, because that is what an artist
//! thinks in. They become whole ticks at import, against the project's tick
//! rate, for exactly the reason [`crate::clip::ms_to_ticks`] gives: a duration
//! resolved at runtime would make the animation depend on whatever tick rate
//! that session happened to be configured with.

use std::collections::BTreeMap;

use dimetric_core::Tick;

use crate::meta::TileAnimation;

/// One tile id's cycle, in ticks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cycle {
    /// Tile ids to show in turn. Never empty.
    pub frames: Vec<u32>,
    /// How long each frame is held, in ticks. Never zero.
    pub ticks: u32,
}

impl Cycle {
    /// How many ticks one pass through the cycle takes.
    pub fn duration_ticks(&self) -> u32 {
        self.ticks.saturating_mul(self.frames.len() as u32)
    }
}

/// Every animated tile on one tileset.
///
/// Keyed by tile id and ordered, so walking it is reproducible — a `BTreeMap`
/// rather than a hash map for the reason I4 gives, even though this side of the
/// engine is presentation: a diagnostic that lists the animated tiles should
/// list them the same way twice.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Animations {
    cycles: BTreeMap<u32, Cycle>,
}

impl Animations {
    /// Bake the sidecar's declarations against a tick rate.
    ///
    /// `frame_ms` is the sheet's own default, which a block may override.
    pub fn bake(declared: &[TileAnimation], frame_ms: u32, tick_rate: u32) -> Animations {
        let mut cycles = BTreeMap::new();
        for tile in declared {
            if tile.frames.is_empty() {
                continue;
            }
            cycles.insert(
                tile.id,
                Cycle {
                    frames: tile.frames.clone(),
                    ticks: crate::clip::ms_to_ticks(tile.frame_ms.unwrap_or(frame_ms), tick_rate),
                },
            );
        }
        Animations { cycles }
    }

    /// Which tile id to draw for `tile` at `tick`.
    ///
    /// A tile with no cycle answers with itself, so a caller can ask
    /// unconditionally — the same bargain [`crate::sheet`] makes by reporting
    /// one frame for a still.
    pub fn frame_at(&self, tile: u32, tick: Tick) -> u32 {
        let Some(cycle) = self.cycles.get(&tile) else {
            return tile;
        };
        // Integer throughout, and the modulus is taken before the index so a
        // run long enough to overflow a `u32` of ticks still lands in range.
        let step = (tick.0 / cycle.ticks as u64) % cycle.frames.len() as u64;
        cycle.frames[step as usize]
    }

    /// The cycle declared for a tile, if any.
    pub fn cycle(&self, tile: u32) -> Option<&Cycle> {
        self.cycles.get(&tile)
    }

    /// Every animated tile id, in order.
    pub fn tiles(&self) -> impl Iterator<Item = u32> + '_ {
        self.cycles.keys().copied()
    }

    /// How many tiles animate.
    pub fn len(&self) -> usize {
        self.cycles.len()
    }

    /// True when nothing on this sheet animates.
    pub fn is_empty(&self) -> bool {
        self.cycles.is_empty()
    }
}
