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
//! # Why there are two ways to write one
//!
//! `app.suspend()` quits; `app.checkpoint()` does not. They write the same
//! bytes to the same slot, replacing whatever was there, and `app.suspended()`
//! answers true for either. What separates them is only whether the session
//! ends, and that is a design decision about reloading rather than anything to
//! do with determinism — see [`SuspendRequest::Checkpoint`] for the argument.
//!
//! Both are ignored by a replay and by a headless run, which is what keeps a
//! recording independent of the files beside it. A `checkpoint` in a recorded
//! session is therefore a no-op line, exactly as `quit` is.
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
    /// Carries the quit with it rather than leaving the game to ask for both,
    /// so "save and quit" is one call and cannot be half-done.
    ///
    /// This is the shape that cannot be scummed: a player who suspends cannot
    /// play past their own save, because the save *is* the end of the session.
    /// That is a property of this variant and not a rule of the engine — see
    /// [`SuspendRequest::Checkpoint`], which deliberately does not have it.
    Suspend,
    /// Write the run out and carry on playing.
    ///
    /// An autosave. Identical to [`SuspendRequest::Suspend`] in what it writes
    /// and where — one slot, replaced — and different in the two things that
    /// matter: it does not quit, and it may happen many times in a session.
    ///
    /// # Why this does not have the anti-scumming property, and why that is
    /// right
    ///
    /// `Suspend` quits, so a player cannot play past their own save. A
    /// checkpoint does not, so they can: checkpoint, play on, die, resume, and
    /// the last minute is undone. This was the stated reason for refusing a
    /// save-without-stopping when it was first asked for.
    ///
    /// The refusal was wrong about whose decision it is. Nothing about
    /// determinism is at stake — the write is between ticks either way, the
    /// request is not hashed either way, and a replay ignores both. What was
    /// being enforced was a *design* choice about reloading, in the one place
    /// a game cannot argue with it. A turn-based game that loses an hour to a
    /// power cut has a worse problem than one whose player can undo a turn,
    /// and which of those a game would rather have is the game's call.
    ///
    /// So the default is unchanged — `suspend()` still quits, and a game that
    /// wants the strict shape simply never calls this — and the opt-in is
    /// explicit and named for what it is.
    Checkpoint,
    /// Replace the run with the suspended one, and consume it.
    Resume,
    /// Throw the suspended run away, for a player starting a new one.
    Discard,
}
