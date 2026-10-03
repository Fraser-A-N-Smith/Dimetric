//! Turning what the simulation asked for into sound.
//!
//! The simulation says what it would play and never touches a device (see
//! [`dimetric_sim::sound`]). This is the other side of that line: it reads the
//! per-tick list, decides which voices survive the pool's caps and stealing,
//! and hands the survivors to a backend — a device in the player, a mock that
//! writes down what it was asked to do everywhere else.
//!
//! Everything decided here is presentation. The pitch jitter comes from a
//! stream of its own, deliberately not the simulation's: sharing one would mean
//! that triggering one fewer sound effect shifted every gameplay roll after it.

use std::collections::BTreeMap;

use dimetric_assets::Artifact;
use dimetric_audio::backend::{Backend, VoiceParams};
use dimetric_audio::{db_to_linear, pitch_variation, Bus, Device, Rejected, Voice, VoicePool};
use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid, Rng};
use dimetric_sim::sound::SoundEvent;
use dimetric_sim::SimState;

use crate::project::Project;

/// Voices that can sound at once.
///
/// The slice peaks around thirty-five live projectiles, each of which could
/// want an impact sound in the same tick. Thirty-two is enough for that to
/// sound like a battle and few enough that a runaway effect cannot drown
/// everything else.
const VOICES: usize = 32;

/// How many copies of one clip may play at once.
const PER_CLIP: usize = 4;

/// Fade applied when a sound is stopped, in seconds.
const STOP_FADE: f32 = 0.05;

/// The mixer, a backend, and what is currently playing.
pub struct Speaker {
    pool: VoicePool,
    backend: Box<dyn Backend>,
    /// The presentation RNG, which the simulation never sees.
    rng: Rng,
    /// Which voice each node started, and what clip it is, so stopping one is
    /// possible and a reused node id is noticeable.
    playing: BTreeMap<NodeUid, (u64, String)>,
    /// Voices that outlive the node that asked for them, by stream and bus.
    ///
    /// A scene load swaps the whole tree, and the sweep below stops voices whose
    /// node has gone — correctly, because a projectile's loop must not outlive
    /// it. So a region theme restarted from bar one on every floor, and "music
    /// continues across a level transition" is the default expectation of every
    /// game with levels.
    ///
    /// Keyed by `(stream, bus)` rather than by node, which is the whole trick:
    /// the next scene's own `Sound` node asks for the same track on the same bus,
    /// finds it already sounding, and continues it.
    themes: BTreeMap<(String, String), u64>,
    /// Anything that went wrong and did not stop the sound.
    pub diagnostics: Diagnostics,
}

impl Speaker {
    /// Open a speaker for a project, loading every audio clip it imported.
    ///
    /// A clip that will not load is a warning: a game missing one sound should
    /// still run, and the diagnostic says which.
    pub fn open(project: &Project, device: Device) -> Speaker {
        let (backend, unavailable) = device.open();
        let mut speaker = Speaker::with_backend(project, backend);
        // A runtime that cannot make a noise says so, here, beside the other
        // startup diagnostics. The session drains these and the player prints
        // them, which is the whole point: a silent game that reports nothing is
        // indistinguishable from a scene with no sounds in it.
        if let Some(d) = unavailable {
            speaker.diagnostics.push(d);
        }
        speaker
    }

    /// The same, over a backend the caller built.
    ///
    /// How a test listens: a mock writing to a log the test holds answers
    /// "was the footstep actually stopped" by being read.
    pub fn with_backend(project: &Project, backend: Box<dyn Backend>) -> Speaker {
        let mut speaker = Speaker {
            pool: VoicePool::new(VOICES, PER_CLIP),
            backend,
            // A stream of its own. Nothing downstream of this reaches the
            // simulation, so the seed only has to be stable enough that a
            // recorded session sounds the same when it is played back.
            rng: Rng::new(0x5000_0000, 0xa0d1),
            playing: BTreeMap::new(),
            themes: BTreeMap::new(),
            diagnostics: Diagnostics::new(),
        };
        speaker.load_clips(project);
        speaker
    }

    /// Hand the backend every audio clip the project has imported.
    pub fn load_clips(&mut self, project: &Project) {
        let Some(imported) = project.imported() else {
            return;
        };
        for (name, artifact) in &imported.artifacts {
            let Artifact::Audio { bytes } = artifact else {
                continue;
            };
            if self.backend.is_loaded(name) {
                continue;
            }
            if let Err(e) = self.backend.load(name, bytes) {
                self.diagnostics.push(
                    Diagnostic::new(Code::ASSET_MISSING, e.to_string())
                        .with_field("asset", name.clone())
                        .with_severity(dimetric_core::Severity::Warning),
                );
            }
        }
    }

    /// The backend, for tests and for reporting what is playing.
    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    /// How many voices are sounding.
    pub fn playing(&self) -> usize {
        self.pool.len()
    }

    /// How many voices of one clip are sounding.
    pub fn playing_clip(&self, clip: &str) -> usize {
        self.pool.voices().iter().filter(|v| v.clip == clip).count()
    }

    /// Whether a continuous voice for this stream and bus is sounding.
    pub fn theme_playing(&self, stream: &str, bus: &str) -> bool {
        self.themes
            .contains_key(&(stream.to_string(), bus.to_string()))
    }

    /// Set a bus's gain, over `seconds`.
    pub fn set_bus_gain(&mut self, bus: Bus, gain: f32, seconds: f32) {
        self.pool.set_gain(bus, gain);
        self.backend.set_bus_gain(bus, gain, seconds);
    }

    /// Act on one tick's worth of sound events.
    ///
    /// Call it after [`dimetric_sim::Sim::step`], with the state that step
    /// produced. Reading rather than draining: the simulation owns the list and
    /// clears it itself at the start of the next tick, so a second listener —
    /// a recorder, a test — sees the same thing this one did.
    pub fn tick(&mut self, state: &SimState) {
        self.pool.age();

        // A looping sound on a node that has been destroyed would otherwise
        // play forever, and a destroy is the normal way a projectile ends.
        //
        // A node id that is still in the tree but now belongs to a *different*
        // node is the trap underneath this: two scene files that reuse an id —
        // easy when ids come from a generator — would leave a voice attached to
        // whatever landed on that id after the swap, and two tracks would play
        // at once. Saying so is cheap and the alternative is an afternoon.
        let mut gone: Vec<NodeUid> = Vec::new();
        for (uid, (_, clip)) in &self.playing {
            match state.scene.by_uid(*uid).and_then(|id| state.scene.get(id)) {
                None => gone.push(*uid),
                Some(node) => {
                    let still_ours = dimetric_sim::sound::SoundCue::of(node)
                        .is_some_and(|cue| &cue.stream == clip);
                    if !still_ours {
                        self.diagnostics.push(
                            Diagnostic::new(
                                Code::SOUND_NODE_REUSED,
                                format!(
                                    "a voice playing {clip:?} belongs to node {}, which is \
                                     now a different node — two scenes reusing one id will \
                                     leave both sounding",
                                    uid.to_text()
                                ),
                            )
                            .with_severity(dimetric_core::Severity::Warning),
                        );
                        gone.push(*uid);
                    }
                }
            }
        }
        for uid in gone {
            self.stop_node(uid);
        }

        // A theme nobody in the new scene asks for stops. The question is asked
        // of the *scene* rather than of a timer: a floor that still has the
        // region's music node keeps it, and one that does not loses it on the
        // tick the swap happens rather than N ticks later. No magic interval,
        // and nothing to tune.
        let wanted: std::collections::BTreeSet<(String, String)> = state
            .scene
            .walk()
            .into_iter()
            .filter_map(|id| state.scene.get(id))
            .filter_map(dimetric_sim::sound::SoundCue::of)
            .filter(|cue| cue.continuous)
            .map(|cue| (cue.stream, cue.bus))
            .collect();
        let orphaned: Vec<(String, String)> = self
            .themes
            .keys()
            .filter(|key| !wanted.contains(*key))
            .cloned()
            .collect();
        for key in orphaned {
            self.stop_theme(&key);
        }

        for event in &state.sounds {
            match event {
                SoundEvent::Play(cue) => self.play(cue),
                SoundEvent::Stop { node } => {
                    // A continuous voice is not the node's, so stopping it means
                    // stopping the track the node names rather than the handle
                    // the node started — which may have been a node in a scene
                    // that is no longer loaded.
                    let cue = state
                        .scene
                        .by_uid(*node)
                        .and_then(|id| state.scene.get(id))
                        .and_then(dimetric_sim::sound::SoundCue::of);
                    match cue {
                        Some(cue) if cue.continuous => {
                            self.stop_theme(&(cue.stream, cue.bus));
                        }
                        _ => self.stop_node(*node),
                    }
                }
            }
        }
    }

    fn play(&mut self, cue: &dimetric_sim::sound::SoundCue) {
        if !self.backend.is_loaded(&cue.stream) {
            self.diagnostics.push(
                Diagnostic::new(
                    Code::ASSET_MISSING,
                    format!("no audio clip named {}", cue.stream),
                )
                .with_field("asset", cue.stream.clone())
                .with_severity(dimetric_core::Severity::Warning),
            );
            return;
        }
        let key = (cue.stream.clone(), cue.bus.clone());
        if cue.continuous {
            // Already sounding: continue it rather than starting it again. This
            // is the whole point — the next floor's own music node asks for the
            // same track and the bar it is on does not move.
            if self.themes.contains_key(&key) {
                return;
            }
            // A different track on the same bus replaces what is there. Stopping
            // fades out while the new one starts at full gain, which is a
            // cross-fade with the mechanism the mixer already has.
            let others: Vec<(String, String)> = self
                .themes
                .keys()
                .filter(|(_, bus)| bus == &cue.bus)
                .cloned()
                .collect();
            for other in others {
                self.stop_theme(&other);
            }
        } else {
            // A node that is already sounding restarts rather than doubling: two
            // copies of one node's own sound is a bug every time.
            self.stop_node(cue.node);
        }

        // I3-exempt: presentation-side audio maths, below the line the
        // simulation can see.
        let volume_db = cue.volume_db.to_f32();
        let pitch = pitch_variation(&mut self.rng, cue.pitch_variation.to_f32());
        let voice = Voice {
            clip: cue.stream.clone(),
            bus: Bus::parse(&cue.bus).unwrap_or(Bus::Sfx),
            volume_db,
            pitch,
            pan: 0.0,
            priority: if cue.looping { 1 } else { 0 },
            handle: 0,
            age: 0,
        };
        match self.pool.play(voice.clone()) {
            Ok(handle) => {
                let params = VoiceParams {
                    clip: voice.clip.clone(),
                    bus: voice.bus,
                    gain: db_to_linear(volume_db) * self.pool.gain(voice.bus),
                    pitch,
                    pan: 0.0,
                    looping: cue.looping,
                };
                if let Err(e) = self.backend.start(handle, &params) {
                    self.diagnostics.push(
                        Diagnostic::new(Code::COMMAND_REJECTED, e.to_string())
                            .with_severity(dimetric_core::Severity::Warning),
                    );
                    self.pool.stop(handle);
                    return;
                }
                match cue.continuous {
                    // Deliberately *not* in `playing`: that map is what the
                    // destroyed-node sweep walks, and a theme outliving its node
                    // is the feature.
                    true => {
                        self.themes.insert(key, handle);
                    }
                    false => {
                        self.playing.insert(cue.node, (handle, cue.stream.clone()));
                    }
                }
            }
            // Not a diagnostic: a full pool is the pool working. A hundred
            // copies of one impact sound is both inaudible and expensive, which
            // is what the caps are for.
            Err(Rejected::ClipCapReached) | Err(Rejected::NoVoiceAvailable) => {}
        }
    }

    fn stop_node(&mut self, node: NodeUid) {
        let Some((handle, _)) = self.playing.remove(&node) else {
            return;
        };
        self.pool.stop(handle);
        self.backend.stop(handle, STOP_FADE);
    }

    fn stop_theme(&mut self, key: &(String, String)) {
        let Some(handle) = self.themes.remove(key) else {
            return;
        };
        self.pool.stop(handle);
        self.backend.stop(handle, STOP_FADE);
    }
}
