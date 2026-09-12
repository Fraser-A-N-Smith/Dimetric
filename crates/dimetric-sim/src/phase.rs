//! The tick's phase order.
//!
//! Fixed, and checked by a test, because reordering these silently breaks every
//! recorded replay. A replay that fails loudly is a bug report; a replay that
//! diverges three weeks later is a week of bisecting.

/// One stage of a tick, in the order it runs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Phase {
    /// Latch this tick's input.
    Input,
    /// `on_tick` on every scripted node.
    ScriptsTick,
    /// Apply velocity to intended positions.
    PhysicsIntegrate,
    /// Build the spatial hash.
    CollisionBroadphase,
    /// Sweep, slide and depenetrate.
    CollisionResolve,
    /// `on_collide` for every contact.
    CollisionCallbacks,
    /// `on_post_tick` on every scripted node.
    ScriptsPostTick,
    /// Deliver queued signals to their connections.
    SignalFlush,
    /// Advance the tick counter.
    TickIncrement,
}

/// Every phase, in order. This is the contract.
pub const PHASE_ORDER: &[Phase] = &[
    Phase::Input,
    Phase::ScriptsTick,
    Phase::PhysicsIntegrate,
    Phase::CollisionBroadphase,
    Phase::CollisionResolve,
    Phase::CollisionCallbacks,
    Phase::ScriptsPostTick,
    Phase::SignalFlush,
    Phase::TickIncrement,
];

impl Phase {
    /// The phase's name, as it appears in traces and in the documentation.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Input => "input",
            Phase::ScriptsTick => "scripts on_tick",
            Phase::PhysicsIntegrate => "physics integrate",
            Phase::CollisionBroadphase => "collision broadphase",
            Phase::CollisionResolve => "collision resolve",
            Phase::CollisionCallbacks => "collision callbacks",
            Phase::ScriptsPostTick => "scripts on_post_tick",
            Phase::SignalFlush => "signal flush",
            Phase::TickIncrement => "tick counter increment",
        }
    }
}
