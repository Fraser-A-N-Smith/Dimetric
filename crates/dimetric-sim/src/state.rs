//! Simulation state, and what may and may not be in it.
//!
//! Everything a tick reads or writes lives here. Nothing else does — no
//! globals, no ambient clocks, no static caches. That is invariant I8 in
//! practice, and it is what makes [`SimState::snapshot`] a complete capture
//! rather than an approximate one.
//!
//! What is deliberately absent is as important as what is present. Audio
//! playback position, cosmetic tweens, particles and render interpolation are
//! presentation, and including any of them would make replay depend on frame
//! timing and audio device latency (I7).

use std::collections::BTreeMap;

use dimetric_core::{HashState, NodeUid, RngStreams, StateHash, StateHasher, Tick, Vec2Fx};
use dimetric_scene::{Scene, Value};
use indexmap::IndexMap;

use crate::input::InputFrame;

/// Where an animation clip has got to.
///
/// Counted in ticks, never in wall-clock milliseconds, because a hitbox
/// activation frame is gameplay. Import converts the artist's millisecond
/// timings to tick counts once, at import time.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AnimState {
    /// Clip currently playing.
    pub clip: String,
    /// Frame index within the clip.
    pub frame: u32,
    /// Ticks spent on the current frame.
    pub ticks_in_frame: u32,
    /// Whether frames are advancing.
    pub playing: bool,
    /// Set when a non-looping clip reaches its end.
    pub finished: bool,
}

impl AnimState {
    /// Feed into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.str(&self.clip)
            .u64(self.frame as u64)
            .u64(self.ticks_in_frame as u64)
            .bool(self.playing)
            .bool(self.finished);
    }
}

/// A signal waiting to be delivered.
#[derive(Clone, PartialEq, Debug)]
pub struct SignalEvent {
    /// Who emitted it.
    pub from: NodeUid,
    /// Signal name.
    pub name: String,
    /// Payload, ordered so delivery is reproducible.
    pub payload: IndexMap<String, Value>,
}

/// A contact waiting to be reported to scripts.
#[derive(Clone, PartialEq, Debug)]
pub struct CollisionEvent {
    /// The body that moved.
    pub node: NodeUid,
    /// What it touched.
    pub other: NodeUid,
    /// Surface normal, pointing back toward `node`.
    pub normal: Vec2Fx,
    /// True when the contact blocked nothing.
    pub trigger: bool,
}

/// The complete state of a running simulation.
#[derive(Clone, Debug)]
pub struct SimState {
    /// Ticks elapsed. The only notion of time the simulation has (I5).
    pub tick: Tick,
    /// The runtime tree, with every prefab instance already resolved.
    pub scene: Scene,
    /// Named random streams, seeded from the run seed (I6).
    pub rng: RngStreams,
    /// Per-body velocity, integrated during the physics phase.
    pub velocity: BTreeMap<NodeUid, Vec2Fx>,
    /// Per-node script variables.
    ///
    /// Script state lives in Rust rather than in Lua globals, because a Lua
    /// table cannot be snapshotted and restored bit-for-bit. This is what lets
    /// `self.hp = self.hp - damage` survive a rollback.
    pub vars: BTreeMap<NodeUid, IndexMap<String, Value>>,
    /// Per-node animation playback.
    pub anim: BTreeMap<NodeUid, AnimState>,
    /// Per-node cosmetic tweens.
    ///
    /// Simulation state, not presentation: a tween writes to node properties,
    /// so it is snapshotted and hashed like everything else that does. See
    /// [`crate::tween`] for why that is the right side of the line.
    pub tweens: crate::tween::Tweens,
    /// Input for the tick in progress.
    pub input: InputFrame,
    /// Input for the tick before this one.
    ///
    /// State rather than something derived from the log, because a rollback
    /// restores a snapshot and has to know what a button was doing before the
    /// tick it lands on — otherwise every edge-triggered action fires again.
    pub previous_input: InputFrame,
    /// Signals emitted this tick, flushed at the end of it.
    pub signals: Vec<SignalEvent>,
    /// Contacts found this tick.
    pub collisions: Vec<CollisionEvent>,
    /// The broadphase as it stood at the start of the tick, for scripts to
    /// query. Not hashed: it is derived from positions that already are.
    pub query: Option<crate::world::PhysicsWorld>,
    /// Nodes a script asked to create, applied at a phase boundary.
    ///
    /// Deferred for the same reason destroys are: inserting into a tree another
    /// script may be walking makes what it sees depend on traversal order.
    pub spawn_queue: Vec<crate::spawn::Spawn>,
    /// How many spawns have happened, which is what makes their ids
    /// reproducible without consuming the RNG.
    pub spawn_count: u64,
    /// Nodes a script asked to destroy, applied at a phase boundary.
    ///
    /// Deferred rather than immediate so a script cannot delete a node another
    /// script is mid-way through iterating.
    pub destroy_queue: Vec<NodeUid>,
    /// Nodes readied since the last tick, so `on_ready` fires exactly once.
    pub readied: Vec<NodeUid>,
}

impl SimState {
    /// A simulation over `scene`, seeded from `seed`.
    pub fn new(scene: Scene, seed: u64) -> SimState {
        SimState {
            tick: Tick::ZERO,
            scene,
            rng: RngStreams::new(seed),
            velocity: BTreeMap::new(),
            vars: BTreeMap::new(),
            anim: BTreeMap::new(),
            tweens: BTreeMap::new(),
            input: InputFrame::idle(1),
            previous_input: InputFrame::idle(1),
            signals: Vec::new(),
            collisions: Vec::new(),
            query: None,
            spawn_queue: Vec::new(),
            spawn_count: 0,
            destroy_queue: Vec::new(),
            readied: Vec::new(),
        }
    }

    /// Capture the whole state.
    ///
    /// A clone rather than a delta. Snapshot and restore are a day-one
    /// requirement here, not an optimisation problem: the replay harness runs
    /// on them, and they are the entire foundation rollback netcode would need.
    pub fn snapshot(&self) -> SimState {
        self.clone()
    }

    /// Restore a captured state.
    pub fn restore(&mut self, snapshot: SimState) {
        *self = snapshot;
    }

    /// Read a script variable.
    pub fn var(&self, node: NodeUid, key: &str) -> Option<&Value> {
        self.vars.get(&node)?.get(key)
    }

    /// Write a script variable.
    pub fn set_var(&mut self, node: NodeUid, key: impl Into<String>, value: Value) {
        self.vars.entry(node).or_default().insert(key.into(), value);
    }

    /// A body's velocity, zero when it has none.
    pub fn velocity_of(&self, node: NodeUid) -> Vec2Fx {
        self.velocity.get(&node).copied().unwrap_or(Vec2Fx::ZERO)
    }

    /// The hash of this state, compared tick by tick during replay.
    pub fn hash(&self) -> StateHash {
        self.state_hash()
    }
}

impl HashState for SimState {
    fn hash_state(&self, h: &mut StateHasher) {
        h.tag("tick").u64(self.tick.0);
        self.scene.hash_state(h);

        h.tag("seed").u64(self.rng.seed());
        h.tag("rng");
        let streams: Vec<(&str, dimetric_core::RngState)> =
            self.rng.iter().map(|(n, r)| (n, r.snapshot())).collect();
        h.len(streams.len());
        for (name, state) in streams {
            h.str(name).rng(state);
        }

        // Every map below is a BTreeMap, so iteration is by key and not by
        // hash seed (I4). Swapping any of them for a HashMap would make the
        // hash differ between runs on the same machine.
        h.tag("velocity").len(self.velocity.len());
        for (uid, v) in &self.velocity {
            h.node_uid(*uid).vec2(*v);
        }

        h.tag("vars").len(self.vars.len());
        for (uid, table) in &self.vars {
            h.node_uid(*uid);
            let mut keys: Vec<&String> = table.keys().collect();
            keys.sort();
            h.len(keys.len());
            for k in keys {
                h.str(k);
                table[k].hash_state(h);
            }
        }

        h.tag("anim").len(self.anim.len());
        for (uid, a) in &self.anim {
            h.node_uid(*uid);
            a.hash_state(h);
        }

        self.previous_input.hash_state(h);
        h.tag("spawns").u64(self.spawn_count);

        h.tag("tweens").len(self.tweens.len());
        for (uid, list) in &self.tweens {
            h.node_uid(*uid);
            h.len(list.len());
            for tween in list {
                tween.hash_state(h);
            }
        }

        self.input.hash_state(h);

        h.tag("signals").len(self.signals.len());
        for s in &self.signals {
            h.node_uid(s.from).str(&s.name);
        }
    }
}
