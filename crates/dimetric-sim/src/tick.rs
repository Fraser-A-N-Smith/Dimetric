//! The tick loop.
//!
//! One tick is `(State, Inputs) -> State` and nothing else (I8). No I/O, no
//! clock, no allocation the state does not own. The accumulator that decides
//! *when* to call this lives in `dimetric-host`; from in here, time is a
//! counter.

use std::cell::{Ref, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use dimetric_core::{Diagnostics, Fx, NodeUid, StateHash, Vec2Fx};
use dimetric_scene::{Scene, Transform, Value};
use indexmap::IndexMap;

use crate::input::InputFrame;
use crate::phase::{Phase, PHASE_ORDER};
use crate::state::{CollisionEvent, SignalEvent, SimState};
use crate::world::PhysicsWorld;

/// Which lifecycle hook is being dispatched.
#[derive(Clone, PartialEq, Debug)]
pub enum Hook {
    /// Once, the first tick a node exists for.
    Ready,
    /// Every tick, before physics.
    Tick,
    /// Every tick, after collisions have been resolved and reported.
    PostTick,
    /// A contact.
    Collide {
        /// What was touched.
        other: NodeUid,
        /// Surface normal, pointing back toward this node.
        normal: Vec2Fx,
        /// True when nothing was blocked.
        trigger: bool,
    },
    /// A connected signal.
    Signal {
        /// Signal name.
        name: String,
        /// Emitter.
        from: NodeUid,
        /// Method to call on this node.
        method: String,
        /// Payload.
        payload: IndexMap<String, Value>,
    },
    /// A per-frame animation event.
    AnimEvent {
        /// Event name from the clip.
        event: String,
    },
    /// The node is about to be removed.
    Destroy,
}

impl Hook {
    /// The Lua function this hook calls.
    pub fn function_name(&self) -> &str {
        match self {
            Hook::Ready => "on_ready",
            Hook::Tick => "on_tick",
            Hook::PostTick => "on_post_tick",
            Hook::Collide { .. } => "on_collide",
            Hook::Signal { method, .. } => method,
            Hook::AnimEvent { .. } => "on_anim_event",
            Hook::Destroy => "on_destroy",
        }
    }
}

/// Something that can run node scripts.
///
/// Behind a trait so the tick loop can be tested, and replayed, without a Lua
/// interpreter in the picture at all.
pub trait ScriptHost {
    /// Run one hook on one node.
    fn dispatch(
        &mut self,
        state: &Rc<RefCell<SimState>>,
        node: NodeUid,
        script: &str,
        hook: &Hook,
    ) -> Result<(), dimetric_core::Diagnostic>;

    /// Replace a script's source in place.
    ///
    /// Hot reload's half of the bargain. Script *state* lives in Rust rather
    /// than in Lua globals, so swapping the source keeps every variable a node
    /// had — which is the difference between reloading a script and restarting
    /// the game.
    ///
    /// The default does nothing, for hosts that run no scripts.
    fn reload(&mut self, _path: &str, _source: &str) -> Result<(), dimetric_core::Diagnostic> {
        Ok(())
    }
}

/// A host that runs nothing. Physics-only simulations use this.
#[derive(Default)]
pub struct NoScripts;

impl ScriptHost for NoScripts {
    fn dispatch(
        &mut self,
        _state: &Rc<RefCell<SimState>>,
        _node: NodeUid,
        _script: &str,
        _hook: &Hook,
    ) -> Result<(), dimetric_core::Diagnostic> {
        Ok(())
    }
}

/// Fixed simulation settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SimConfig {
    /// Ticks per second. Part of the replay contract: changing it changes
    /// every recorded run's meaning.
    pub tick_rate: u32,
}

impl Default for SimConfig {
    fn default() -> SimConfig {
        SimConfig { tick_rate: 60 }
    }
}

impl SimConfig {
    /// Seconds per tick, as a scalar. This is the only `dt` in the engine.
    pub fn dt(&self) -> Fx {
        Fx::ONE / self.tick_rate as i32
    }
}

/// A running simulation.
pub struct Sim {
    state: Rc<RefCell<SimState>>,
    scripts: Box<dyn ScriptHost>,
    config: SimConfig,
    diagnostics: Diagnostics,
    /// This tick's intended motion per body, produced by the integrate phase
    /// and consumed by the resolve phase.
    ///
    /// Transient: rebuilt from velocity every tick before it is read, so it is
    /// deliberately not part of the snapshot.
    motion: BTreeMap<NodeUid, Vec2Fx>,
    /// The broadphase for this tick, built from positions as they stand
    /// *before* any motion is applied.
    world: Option<PhysicsWorld>,
}

impl Sim {
    /// Start a simulation over an already-resolved scene.
    pub fn new(scene: Scene, seed: u64, scripts: Box<dyn ScriptHost>, config: SimConfig) -> Sim {
        let mut state = SimState::new(scene, seed);
        state.scene.update_world_transforms();
        Sim {
            state: Rc::new(RefCell::new(state)),
            scripts,
            config,
            diagnostics: Diagnostics::new(),
            motion: BTreeMap::new(),
            world: None,
        }
    }

    /// Borrow the current state.
    pub fn state(&self) -> Ref<'_, SimState> {
        self.state.borrow()
    }

    /// The shared handle, for script hosts.
    pub fn shared(&self) -> &Rc<RefCell<SimState>> {
        &self.state
    }

    /// Settings.
    pub fn config(&self) -> SimConfig {
        self.config
    }

    /// The script host, for a caller that needs to reload sources into it.
    ///
    /// Only reachable between ticks: `step` takes `&mut self`, so the borrow
    /// checker refuses a reload part-way through one.
    pub fn scripts_mut(&mut self) -> &mut dyn ScriptHost {
        self.scripts.as_mut()
    }

    /// Diagnostics raised so far.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Take the diagnostics raised so far.
    pub fn take_diagnostics(&mut self) -> Diagnostics {
        std::mem::replace(&mut self.diagnostics, Diagnostics::new())
    }

    /// The current state hash.
    pub fn hash(&self) -> StateHash {
        self.state.borrow().hash()
    }

    /// Capture the whole state.
    pub fn snapshot(&self) -> SimState {
        self.state.borrow().snapshot()
    }

    /// Restore a captured state.
    pub fn restore(&mut self, snapshot: SimState) {
        *self.state.borrow_mut() = snapshot;
    }

    /// Advance one tick.
    ///
    /// Runs [`PHASE_ORDER`] exactly, in order, every time.
    pub fn step(&mut self, input: InputFrame) {
        for phase in PHASE_ORDER {
            self.run_phase(*phase, &input);
        }
    }

    fn run_phase(&mut self, phase: Phase, input: &InputFrame) {
        match phase {
            Phase::Input => {
                self.state.borrow_mut().input = input.clone();
                self.dispatch_ready();
            }
            Phase::ScriptsTick => self.dispatch_all(Hook::Tick),
            Phase::PhysicsIntegrate => self.integrate(),
            Phase::CollisionBroadphase => self.build_broadphase(),
            Phase::CollisionResolve => self.resolve_collisions(),
            Phase::CollisionCallbacks => self.dispatch_collisions(),
            Phase::ScriptsPostTick => self.dispatch_all(Hook::PostTick),
            Phase::SignalFlush => {
                self.flush_signals();
                self.apply_destroys();
            }
            Phase::TickIncrement => {
                let mut state = self.state.borrow_mut();
                state.tick = state.tick.next();
                state.collisions.clear();
            }
        }
    }

    /// Scripted nodes, in depth-first order.
    fn scripted_nodes(&self) -> Vec<(NodeUid, String)> {
        let state = self.state.borrow();
        state
            .scene
            .walk()
            .into_iter()
            .filter_map(|id| {
                let node = state.scene.get(id)?;
                let script = node.script.as_ref()?;
                Some((node.uid, script.target().to_string()))
            })
            .collect()
    }

    fn dispatch_ready(&mut self) {
        let pending: Vec<(NodeUid, String)> = {
            let state = self.state.borrow();
            self.scripted_nodes()
                .into_iter()
                .filter(|(uid, _)| !state.readied.contains(uid))
                .collect()
        };
        for (uid, script) in pending {
            self.state.borrow_mut().readied.push(uid);
            self.call(uid, &script, &Hook::Ready);
        }
    }

    fn dispatch_all(&mut self, hook: Hook) {
        for (uid, script) in self.scripted_nodes() {
            self.call(uid, &script, &hook);
        }
    }

    fn call(&mut self, node: NodeUid, script: &str, hook: &Hook) {
        if let Err(d) = self.scripts.dispatch(&self.state, node, script, hook) {
            self.diagnostics.push(d);
        }
    }

    /// Turn velocity into the motion this tick intends.
    ///
    /// Deliberately does **not** write node positions. If it did, the
    /// broadphase would be built from the destination and every sweep would
    /// start where it was supposed to finish — which reads as working right up
    /// until something moves far enough in one tick to jump a wall.
    fn integrate(&mut self) {
        let dt = self.config.dt();
        let state = self.state.borrow();
        self.motion = state
            .velocity
            .iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(uid, v)| (*uid, *v * dt))
            .collect();
    }

    /// Build the spatial hash over positions as they currently stand.
    fn build_broadphase(&mut self) {
        let mut state = self.state.borrow_mut();
        state.scene.update_world_transforms();
        self.world = Some(PhysicsWorld::build(&state.scene));
    }

    /// Sweep every dynamic body along its intended motion.
    fn resolve_collisions(&mut self) {
        let Some(world) = self.world.take() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let mut events = Vec::new();
        let mut writes: Vec<(NodeUid, Vec2Fx)> = Vec::new();

        for (index, body) in world.bodies().iter().enumerate() {
            if body.is_static || body.is_area {
                continue;
            }
            let motion = self.motion.get(&body.uid).copied().unwrap_or(Vec2Fx::ZERO);
            let (resolved, contacts) = world.move_body(index, body.pos + motion);
            // Sweeping handles motion. Depenetration handles what sweeping
            // cannot: a body spawned inside a wall, or one a script teleported
            // by writing `self.pos` directly.
            let resolved = world.depenetrate(index, resolved);
            writes.push((body.uid, resolved));
            for c in contacts {
                events.push(CollisionEvent {
                    node: c.a,
                    other: c.b,
                    normal: c.normal,
                    trigger: c.trigger,
                });
            }
        }

        for (uid, world_pos) in writes {
            let Some(id) = state.scene.by_uid(uid) else {
                continue;
            };
            // Positions come back in world space; the node stores a local one.
            let parent_world = state
                .scene
                .get(id)
                .and_then(|n| n.parent())
                .and_then(|p| state.scene.world_of(p))
                .unwrap_or(Transform::IDENTITY);
            let local = parent_world.inverse_apply(world_pos);
            state.scene.set_position(id, local);
        }
        state.scene.update_world_transforms();

        // Sorted by the pair's permanent ids, so callback order never depends
        // on iteration order or memory layout (I4).
        events.sort_by_key(|e| (e.node.body().to_string(), e.other.body().to_string()));
        state.collisions = events;
    }

    fn dispatch_collisions(&mut self) {
        let events = self.state.borrow().collisions.clone();
        let scripts: Vec<(NodeUid, String)> = self.scripted_nodes();
        for event in events {
            let Some((_, script)) = scripts.iter().find(|(uid, _)| *uid == event.node) else {
                continue;
            };
            let script = script.clone();
            self.call(
                event.node,
                &script,
                &Hook::Collide {
                    other: event.other,
                    normal: event.normal,
                    trigger: event.trigger,
                },
            );
        }
    }

    /// Deliver this tick's signals along their declared connections.
    fn flush_signals(&mut self) {
        let (pending, connections) = {
            let mut state = self.state.borrow_mut();
            let pending = std::mem::take(&mut state.signals);
            (pending, state.scene.connections.clone())
        };
        let scripts = self.scripted_nodes();

        for signal in pending {
            for connection in &connections {
                if connection.from != signal.from || connection.signal != signal.name {
                    continue;
                }
                let Some((_, script)) = scripts.iter().find(|(uid, _)| *uid == connection.to)
                else {
                    continue;
                };
                let script = script.clone();
                self.call(
                    connection.to,
                    &script,
                    &Hook::Signal {
                        name: signal.name.clone(),
                        from: signal.from,
                        method: connection.method.clone(),
                        payload: signal.payload.clone(),
                    },
                );
            }
        }
    }

    /// Remove nodes scripts asked to destroy.
    ///
    /// Deferred to a phase boundary so that a script cannot delete a node
    /// another script is part-way through working with.
    fn apply_destroys(&mut self) {
        let pending: Vec<NodeUid> = std::mem::take(&mut self.state.borrow_mut().destroy_queue);
        let scripts = self.scripted_nodes();
        for uid in &pending {
            if let Some((_, script)) = scripts.iter().find(|(u, _)| u == uid) {
                let script = script.clone();
                self.call(*uid, &script, &Hook::Destroy);
            }
        }
        let mut state = self.state.borrow_mut();
        for uid in pending {
            let Some(id) = state.scene.by_uid(uid) else {
                continue;
            };
            for removed in state.scene.remove_subtree(id) {
                state.velocity.remove(&removed.uid);
                state.vars.remove(&removed.uid);
                state.anim.remove(&removed.uid);
                state.readied.retain(|u| *u != removed.uid);
            }
        }
    }

    /// Queue a signal for this tick's flush.
    pub fn emit(
        &mut self,
        from: NodeUid,
        name: impl Into<String>,
        payload: IndexMap<String, Value>,
    ) {
        self.state.borrow_mut().signals.push(SignalEvent {
            from,
            name: name.into(),
            payload,
        });
    }

    /// Set a body's velocity, in world units per second.
    pub fn set_velocity(&mut self, node: NodeUid, velocity: Vec2Fx) {
        self.state.borrow_mut().velocity.insert(node, velocity);
    }
}
