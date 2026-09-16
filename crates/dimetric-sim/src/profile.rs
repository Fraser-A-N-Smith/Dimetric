//! What accumulates across runs, and why it is nowhere near the state.
//!
//! Knowledge, awards and unlocks are not part of a run. Two players playing
//! the same seed have different ones, so a profile inside the state hash would
//! make their replays diverge for a reason that has nothing to do with the
//! game.
//!
//! The engine has done this twice before and the lesson stuck both times. The
//! sound list is in `SimState` and skipped by the hasher; the log lines are not
//! in `SimState` at all, because "kept out entirely" is one fewer thing to get
//! wrong than "a field the hash skips". A profile is the second kind. It is
//! owned by the script host, beside the log lines, and there is no field on
//! `SimState` for a future change to start hashing by accident.
//!
//! # The hazard this cannot fix, stated plainly
//!
//! Keeping the profile out of the hash stops it *being* hashed. It does not
//! stop a script reading from it and writing what it read into state:
//!
//! ```lua
//! if profile.get("knows_fire") then self.spell = "fire" end   -- diverges
//! ```
//!
//! That line makes two players' simulations differ, and no amount of care on
//! this side of the boundary prevents it — the divergence is in the game, not
//! in the storage. Hashing the profile would not help either: it would turn a
//! silent divergence into a loud one, at the cost of making every replay
//! depend on who is playing.
//!
//! So the rule is a rule a game has to follow: **a value read from the profile
//! may decide what is drawn, offered or unlocked, and may not decide what the
//! simulation does.** Choose a starting loadout from it at the menu, before
//! the run begins, and carry the choice in through `scene.request_load`, where
//! it is hashed like everything else. `dim script check --determinism` reports
//! a profile read that flows into a state write, which is the only mechanical
//! help available.

use std::collections::BTreeMap;

use dimetric_scene::Value;

/// Everything a player has accumulated across runs.
///
/// A `BTreeMap` rather than a `HashMap` even though this never reaches a hash
/// (I4 does not apply here), because the file it serialises to should have a
/// stable key order and diff cleanly.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Profile {
    entries: BTreeMap<String, Value>,
    /// Whether anything changed since the host last wrote it out.
    dirty: bool,
}

impl Profile {
    /// An empty profile.
    pub fn new() -> Profile {
        Profile::default()
    }

    /// Build one from stored entries.
    pub fn from_entries(entries: BTreeMap<String, Value>) -> Profile {
        Profile {
            entries,
            dirty: false,
        }
    }

    /// What is stored under a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// Store a value, replacing what was there.
    pub fn put(&mut self, key: &str, value: Value) {
        self.entries.insert(key.to_string(), value);
        self.dirty = true;
    }

    /// Remove a key.
    pub fn clear(&mut self, key: &str) {
        if self.entries.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// Every entry, in key order.
    pub fn entries(&self) -> &BTreeMap<String, Value> {
        &self.entries
    }

    /// True when something changed since [`Profile::mark_clean`].
    ///
    /// The host writes the file on a change rather than every tick, because a
    /// profile that touched the disk sixty times a second would be a profile
    /// that eventually gets caught half-written by a power cut.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Record that the current contents have been written out.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}
