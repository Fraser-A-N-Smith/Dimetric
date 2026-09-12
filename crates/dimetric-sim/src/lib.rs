//! The deterministic simulation: tick loop, physics, scripting and snapshots.
//!
//! # What determinism costs, and what it buys
//!
//! Nothing in here reads a clock, iterates a `HashMap`, or touches a float.
//! In exchange, a seed plus an input log reproduces a run byte-for-byte, on any
//! machine, forever. That is what lets an agent verify its own work, and it is
//! most of what rollback netcode would need later.
//!
//! The rules are not suggestions. They are invariants I3 through I8, and
//! `cargo xtask lint-sim` rejects the common ways of breaking them.

#![warn(missing_docs)]

pub mod input;
pub mod phase;
pub mod script;
pub mod shape;
pub mod state;
pub mod sweep;
pub mod tick;
pub mod world;

pub use input::{InputFrame, InputLog, PlayerInput};
pub use phase::{Phase, PHASE_ORDER};
pub use script::LuaHost;
pub use shape::{Contact, Shape};
pub use state::{AnimState, CollisionEvent, SignalEvent, SimState};
pub use sweep::{sweep_aabb, Hit};
pub use tick::{Hook, NoScripts, ScriptHost, Sim, SimConfig};
pub use world::{Body, ContactEvent, PhysicsWorld};
