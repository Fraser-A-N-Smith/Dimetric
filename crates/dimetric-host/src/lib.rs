//! The command bus, the project model, the run loop and the replay harness.
//!
//! # Why the bus is the engine
//!
//! Every mutation is a serializable, invertible [`Command`]. The GUI editor,
//! the CLI and an agent are all clients of this one layer, and none of them has
//! a private path into engine state (invariant I1). Undo falls out for free,
//! and — because there is no second way in — it cannot desynchronize.

#![warn(missing_docs)]

pub mod bus;
pub mod command;
pub mod ldtk;
pub mod project;
pub mod reload;
pub mod render;
pub mod replay;
pub mod run;

pub use bus::CommandBus;
pub use command::{apply, Applied, Command};
pub use project::{DiskScenes, Project};
pub use render::{capture, CaptureRequest, CapturedFrame};
pub use replay::{Divergence, Probe, Replay, ReplayReport};
pub use run::{Accumulator, RunMode};
