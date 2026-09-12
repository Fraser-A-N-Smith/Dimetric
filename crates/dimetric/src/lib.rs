//! Dimetric: a deterministic 2D game engine for top-down and isometric games.
//!
//! This is the umbrella crate. It re-exports the engine's parts so a game
//! depends on one crate rather than eight.
//!
//! # On the name
//!
//! What pixel-art games call "isometric" almost never is. True isometric
//! projection puts 120° between all three axes; the 2:1 tile ratio the genre
//! actually ships is *dimetric* projection, where one axis foreshortens
//! differently from the others. The engine is named for the projection it
//! really uses.
//!
//! # The three ideas
//!
//! 1. **The command bus is the engine.** Every mutation is a serializable,
//!    invertible command. The editor, the CLI and an agent are all clients of
//!    one layer, and undo falls out for free.
//! 2. **Determinism is an invariant, not a feature.** The same seed and input
//!    log produce byte-identical state, on every machine, forever.
//! 3. **Perspective is a camera matrix.** The world is free-form 2D. Top-down
//!    is the identity; isometric is a 2:1 shear at render time.
//!
//! # Where to start
//!
//! [`dimetric_core`] for the fixed-point types everything is built on,
//! [`dimetric_scene`] for the node tree and the `.dim` format,
//! [`dimetric_sim`] for the tick loop, and [`dimetric_host`] for the command
//! bus.

#![warn(missing_docs)]

pub use dimetric_assets as assets;
pub use dimetric_audio as audio;
pub use dimetric_core as core;
pub use dimetric_host as host;
pub use dimetric_platform as platform;
pub use dimetric_render as render;
pub use dimetric_scene as scene;
pub use dimetric_sim as sim;

pub use dimetric_core::{Angle, Fx, FxWide, NodeUid, Rect, Rng, Tick, Vec2Fx};
pub use dimetric_host::{Command, Project};
pub use dimetric_scene::{Scene, Value};
pub use dimetric_sim::{Sim, SimConfig, SimState};

/// The engine version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
