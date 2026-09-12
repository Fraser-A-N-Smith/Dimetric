//! Foundation types for the Dimetric engine: fixed-point math, stable
//! identity, seeded randomness and structured diagnostics.
//!
//! This crate depends on no other engine crate, and everything above it
//! depends on this one. If a type is shared between the simulation and the
//! renderer, it lives here.
//!
//! # The rule that matters
//!
//! Simulation code uses [`Fx`], never `f32` or `f64` (invariant I3). The float
//! conversions that exist — [`Fx::to_f32`], [`Fx::from_f64_lossy`] — are for
//! the render and asset-import boundaries, and `cargo xtask lint-sim` rejects
//! them anywhere else.

#![warn(missing_docs)]

pub mod angle;
pub mod diag;
pub mod fx;
pub mod hash;
pub mod id;
pub mod rect;
pub mod rng;
mod trig_table;
pub mod vec;

pub use angle::Angle;
pub use diag::{Code, Diagnostic, Diagnostics, Severity, Span};
pub use fx::{Fx, FxWide};
pub use hash::{HashState, StateHash, StateHasher};
pub use id::{AssetId, NodeId, NodeUid};
pub use rect::Rect;
pub use rng::{Rng, RngState, RngStreams};
pub use vec::{vec2, Vec2Fx};

/// A tick number. Simulation sees these and never a wall clock (I5).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u64);

impl Tick {
    /// The tick before any simulation has run.
    pub const ZERO: Tick = Tick(0);

    /// The next tick.
    #[inline]
    pub const fn next(self) -> Tick {
        Tick(self.0 + 1)
    }
}

impl core::fmt::Display for Tick {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}
