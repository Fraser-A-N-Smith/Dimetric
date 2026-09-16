//! Asking for a different scene, from inside a running game.
//!
//! # Why this does not break I8
//!
//! `ENGINE-GAPS.md` recorded scene loading as "probably right as it stands",
//! on the grounds that loading a scene mid-tick would mean the tick was not a
//! pure function of the state it started from. That reasoning is correct and
//! this does not do it.
//!
//! A script *requests* a load. The request is state, like a spawn: nothing is
//! created, nothing is swapped, and the tick that made the request finishes
//! over exactly the tree it started with. The swap happens **between** ticks,
//! in the host, which is the only layer that has a filesystem to read a scene
//! from. Tick N is still `(State, Inputs) -> State`; tick N+1 simply starts
//! from a different state.
//!
//! # What crosses, and what does not
//!
//! The tree is replaced. The **run** is not: the tick counter keeps counting
//! and the RNG streams keep their positions, because a roguelike descending to
//! its second floor is still in the same run — resetting the streams would
//! generate every floor from the same numbers.
//!
//! Everything derived from the old tree goes: velocities, script variables,
//! animation, tweens, queued spawns. They name nodes that no longer exist, and
//! keeping them would leave a tween writing to a rewound position in a scene
//! that never had it.
//!
//! So anything the game needs to survive the boundary has to be said
//! explicitly, which is what `carry` is for. An adventurer's classes, stones
//! and health go through it as a `Value`, which is hashed, snapshotted and
//! rewindable — as opposed to a global somewhere, which is the state-hiding the
//! frozen-module work closed off.

use dimetric_core::{HashState, StateHasher};
use dimetric_scene::Value;

/// A scene a script asked for, waiting for the end of the tick.
#[derive(Clone, PartialEq, Debug)]
pub struct SceneLoad {
    /// Project-relative path, as the script gave it.
    pub path: String,
    /// What to hand the new scene.
    pub carry: Value,
}

impl HashState for SceneLoad {
    fn hash_state(&self, h: &mut StateHasher) {
        h.tag("load").str(&self.path);
        self.carry.hash_state(h);
    }
}
