//! Stopping a run so it can be continued later, and continuing it.
//!
//! # Why this is a request and not an action
//!
//! The same reason `scene.request_load` is. A tick that wrote its own state to
//! disk, or replaced it with somebody else's, would stop being a pure function
//! of the state it started from (I8) — and the simulation has no filesystem to
//! do either with. So a script *asks*, the tick finishes over exactly the state
//! it started with, and the host acts **between** ticks.
//!
//! Tick N is still `(State, Inputs) -> State`. Tick N+1 simply starts from a
//! state that came off a disk instead of from tick N.
//!
//! # Why the request is not simulation state
//!
//! It sits beside the quit flag on the script host rather than in `SimState`,
//! and for the same reason: a run in which somebody chose "Save and quit" has
//! to hash identically to one where they closed the window. If the request
//! were hashed, how a session *ended* would change what it replayed to, and a
//! rollback would un-ask. Kept out structurally, so there is no field for a
//! later change to start hashing by accident.
//!
//! The `suspended` answer is the mirror image: an input the host supplies, like
//! the profile and the fonts. It is not in `SimState` either, and a replay is
//! told `false` — a recorded session must not depend on files beside it, so a
//! replay that read a real save would reproduce only on the machine that made
//! one.
//!
//! # Why resuming is exact and not a fast-forward
//!
//! Two shapes were considered and refused when this was first asked for.
//! Writing the run into the profile cannot restore the RNG streams, so the
//! "resumed" run would roll different numbers from the one that stopped.
//! Replaying the input log from tick zero costs time proportional to the run's
//! length and invalidates every save the moment the game's balance changes.
//! Only the state itself is the run.

/// What a script asked the runtime to do with the suspended-run slot.
///
/// One request per tick, last call winning, exactly as a scene load does: a
/// script that asks twice in one tick has made a mistake, and the alternative
/// is a queue whose order is a second thing to reason about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuspendRequest {
    /// Write the run out and stop.
    ///
    /// Carries the quit with it rather than leaving the game to ask for both.
    /// A run that wrote its save and kept playing would let a player suspend,
    /// play on, die, and then resume the saved run — which is the opposite of
    /// "exactly once per save", and would be the game's bug to discover rather
    /// than the engine's to prevent.
    Suspend,
    /// Replace the run with the suspended one, and consume it.
    Resume,
    /// Throw the suspended run away, for a player starting a new one.
    Discard,
}
