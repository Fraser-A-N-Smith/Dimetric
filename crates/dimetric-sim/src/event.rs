//! What the simulation tells the host.
//!
//! Steam achievements, a volume change, rich presence: in every case a script
//! knows something and the process around it needs to hear about it. The
//! sandbox has no `package`, no FFI and no `io`, which is correct and stays
//! correct — so the simulation cannot call a platform SDK and should not be
//! able to.
//!
//! # Why this is the sound list's argument again
//!
//! If emitting an achievement consumed a random number or wrote something
//! hashed, a build with Steam disabled would diverge from one with it on. That
//! is word for word the reasoning that shaped [`crate::sound`], and it lands
//! the same way: the simulation *says what it would tell the host*, and
//! something on the other side decides what that means.
//!
//! # Why this is not on `SimState`, and the sound list is
//!
//! The engine has made this choice twice and the second one was better. Sounds
//! are a field on `SimState` that the hasher skips. Log lines are not on
//! `SimState` at all, because "kept out entirely" is one fewer thing to get
//! wrong than "a field the hash skips" — a later change cannot start hashing a
//! field that does not exist. Events take the stronger form.
//!
//! It also settles the rollback question, which is worth deciding rather than
//! discovering:
//!
//! **A rollback re-emits.** Events are not snapshotted, so restoring a
//! snapshot does not put drained events back, and re-running a tick emits
//! whatever that tick emits again. The alternative — snapshotting them — would
//! mean a restore could resurrect events the host had already acted on, which
//! is worse in every case. So the contract is that this is a record of what
//! the simulation *said as it ran*, not part of what it *is*, and a host that
//! cares about exactly-once must deduplicate. For achievements that costs
//! nothing: Steam deduplicates, and so can the game.

use dimetric_core::Tick;
use dimetric_scene::Value;

/// Something the simulation told the host.
#[derive(Clone, PartialEq, Debug)]
pub struct GameEvent {
    /// The tick it was emitted on.
    ///
    /// Carried rather than left implicit because a host draining after several
    /// ticks, or after a rollback re-ran some, otherwise cannot tell where one
    /// tick's events end and the next one's begin.
    pub tick: Tick,
    /// What kind of thing happened, as the game names it.
    ///
    /// Free text on purpose. The engine cannot know a game's achievement
    /// names, and a fixed enum would mean every game that wanted a new kind
    /// of event editing the engine.
    pub kind: String,
    /// The details, as the `Value` scripts already store.
    ///
    /// Ordered, so a host reading it gets the same thing every time, and no
    /// second serialisation to keep in step.
    pub payload: Value,
}

/// How many events one tick may emit.
///
/// A script looping over a thousand monsters and emitting per monster is a
/// plausible mistake, and an unbounded list fills memory quietly. The cap is
/// generous enough that no sane tick reaches it and small enough that a
/// runaway loop is reported rather than fatal.
pub const MAX_EVENTS_PER_TICK: usize = 4096;
