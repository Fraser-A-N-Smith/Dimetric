//! The mixer: what plays, how loud, and on which bus.
//!
//! Everything interesting about audio that can go wrong lives here rather than
//! in a backend — voice stealing, per-clip caps, fades, bus gain — so it is all
//! testable against the mock backend, without a sound card.
//!
//! # The rule that matters
//!
//! Nothing here is ever snapshotted or hashed (I7). A headless run with sound
//! triggered must produce exactly the same state hash as a windowed one, or a
//! replay would depend on audio device latency. Pitch variation comes from a
//! presentation RNG stream for the same reason: sharing the simulation's stream
//! would mean that turning the sound off shifted every gameplay roll after it.

use std::collections::BTreeMap;

use dimetric_core::Rng;

use crate::backend::{Backend, VoiceParams};
use crate::tween::{Curve, Tween};
use crate::{db_to_linear, pitch_variation, Bus, Rejected, Voice, VoicePool};

/// A request to play a sound.
#[derive(Clone, PartialEq, Debug)]
pub struct Play {
    /// Clip name, as a scene refers to it.
    pub clip: String,
    /// Bus it routes through.
    pub bus: Bus,
    /// Gain in decibels, before bus gain.
    pub volume_db: f32,
    /// Stereo position, `-1.0` left to `1.0` right.
    pub pan: f32,
    /// Higher priority voices are stolen last.
    pub priority: u8,
    /// Random pitch spread either side of unity, `0.0` for none.
    ///
    /// A spell cast three hundred times at exactly the same pitch stops
    /// sounding like a spell and starts sounding like a bug.
    pub pitch_spread: f32,
    /// Fade in over this many seconds.
    pub fade_in: f32,
    /// Whether it repeats.
    pub looping: bool,
}

impl Play {
    /// A one-shot on the effects bus.
    pub fn clip(name: impl Into<String>) -> Play {
        Play {
            clip: name.into(),
            bus: Bus::Sfx,
            volume_db: 0.0,
            pan: 0.0,
            priority: 128,
            pitch_spread: 0.0,
            fade_in: 0.0,
            looping: false,
        }
    }

    /// Route it through a different bus.
    pub fn on(mut self, bus: Bus) -> Play {
        self.bus = bus;
        self
    }

    /// Set the gain in decibels.
    pub fn volume_db(mut self, db: f32) -> Play {
        self.volume_db = db;
        self
    }

    /// Vary the pitch by up to `spread` either side of unity.
    pub fn pitch_spread(mut self, spread: f32) -> Play {
        self.pitch_spread = spread;
        self
    }

    /// Fade in over `seconds`.
    pub fn fade_in(mut self, seconds: f32) -> Play {
        self.fade_in = seconds;
        self
    }

    /// Loop until stopped.
    pub fn looping(mut self, looping: bool) -> Play {
        self.looping = looping;
        self
    }

    /// Set the stealing priority.
    pub fn priority(mut self, priority: u8) -> Play {
        self.priority = priority;
        self
    }
}

/// A voice's own fade, on top of its static gain.
struct Fade {
    tween: Tween,
    /// Stop the voice once the fade lands on silence.
    stop_when_done: bool,
}

/// Voices, buses, fades, and a backend to send them to.
pub struct Mixer {
    pool: VoicePool,
    backend: Box<dyn Backend>,
    bus_gain: BTreeMap<Bus, Tween>,
    fades: BTreeMap<u64, Fade>,
    rng: Rng,
}

impl Mixer {
    /// A mixer with the given voice budget, feeding `backend`.
    ///
    /// `seed` drives pitch variation only. It is a presentation stream and must
    /// not be the simulation's.
    pub fn new(
        backend: Box<dyn Backend>,
        capacity: usize,
        per_clip_cap: usize,
        seed: u64,
    ) -> Mixer {
        Mixer {
            pool: VoicePool::new(capacity, per_clip_cap),
            backend,
            bus_gain: Bus::ALL.iter().map(|b| (*b, Tween::held(1.0))).collect(),
            fades: BTreeMap::new(),
            // A stream of its own, named so it is obvious in a stack trace that
            // this is not the simulation's.
            rng: Rng::new(seed, 0xa0d10),
        }
    }

    /// The backend in use.
    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    /// The backend in use, for a caller that needs to inspect it.
    pub fn backend_mut(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// The voice pool.
    pub fn pool(&self) -> &VoicePool {
        &self.pool
    }

    /// Hand a clip's encoded bytes to the backend.
    pub fn load(&mut self, clip: &str, bytes: &[u8]) -> Result<(), crate::backend::AudioError> {
        self.backend.load(clip, bytes)
    }

    /// Trigger a sound.
    pub fn play(&mut self, request: Play) -> Result<u64, Rejected> {
        let pitch = pitch_variation(&mut self.rng, request.pitch_spread);
        let voice = Voice {
            clip: request.clip.clone(),
            bus: request.bus,
            volume_db: request.volume_db,
            pitch,
            pan: request.pan,
            priority: request.priority,
            handle: 0,
            age: 0,
        };
        // The pool decides whether there is room, and who loses if not. Ask it
        // before the backend, so a stolen voice is stopped rather than left
        // ringing on a device nobody is tracking any more.
        let playing: Vec<u64> = self.pool.voices().iter().map(|v| v.handle).collect();
        let handle = self.pool.play(voice)?;
        for stolen in playing {
            if !self.pool.voices().iter().any(|v| v.handle == stolen) {
                self.backend.stop(stolen, 0.0);
                self.fades.remove(&stolen);
            }
        }

        let fade = Fade {
            tween: Tween::new(
                if request.fade_in > 0.0 { 0.0 } else { 1.0 },
                1.0,
                request.fade_in,
                Curve::EaseOut,
            ),
            stop_when_done: false,
        };
        let params = self.params(handle, fade.tween.value(), request.looping);
        self.fades.insert(handle, fade);
        if let Some(params) = params {
            // A backend that cannot start the sound costs the voice: keeping it
            // in the pool would reserve a slot for something inaudible.
            if self.backend.start(handle, &params).is_err() {
                self.pool.stop(handle);
                self.fades.remove(&handle);
                return Err(Rejected::NoVoiceAvailable);
            }
        }
        Ok(handle)
    }

    /// Stop a voice, fading out over `seconds`.
    pub fn stop(&mut self, handle: u64, seconds: f32) {
        if seconds <= 0.0 {
            self.pool.stop(handle);
            self.fades.remove(&handle);
            self.backend.stop(handle, 0.0);
            return;
        }
        if let Some(fade) = self.fades.get_mut(&handle) {
            fade.tween.retarget(0.0, seconds, Curve::EaseOut);
            fade.stop_when_done = true;
            self.backend.stop(handle, seconds);
        }
    }

    /// Stop every voice on a bus.
    pub fn stop_bus(&mut self, bus: Bus, seconds: f32) {
        let handles: Vec<u64> = self
            .pool
            .voices()
            .iter()
            .filter(|v| v.bus == bus)
            .map(|v| v.handle)
            .collect();
        for handle in handles {
            self.stop(handle, seconds);
        }
    }

    /// Fade a bus to a new gain.
    ///
    /// This is what a room transition and a pause menu are made of.
    pub fn fade_bus(&mut self, bus: Bus, gain: f32, seconds: f32, curve: Curve) {
        let gain = gain.clamp(0.0, 4.0);
        self.bus_gain
            .entry(bus)
            .or_insert_with(|| Tween::held(1.0))
            .retarget(gain, seconds, curve);
        self.backend.set_bus_gain(bus, gain, seconds);
    }

    /// A bus's gain right now.
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        self.bus_gain.get(&bus).map(|t| t.value()).unwrap_or(1.0)
    }

    /// Move audio time forward.
    ///
    /// Seconds of wall clock, not ticks. Audio is presentation and runs on the
    /// frame rate; a fade that ran on the tick rate would stutter whenever the
    /// simulation caught up on several ticks at once.
    pub fn advance(&mut self, seconds: f32) {
        for tween in self.bus_gain.values_mut() {
            tween.advance(seconds);
        }
        self.pool.set_gain(Bus::Music, self.bus_gain(Bus::Music));
        self.pool.set_gain(Bus::Sfx, self.bus_gain(Bus::Sfx));
        self.pool.set_gain(Bus::Ui, self.bus_gain(Bus::Ui));

        let handles: Vec<u64> = self.fades.keys().copied().collect();
        let mut finished = Vec::new();
        for handle in handles {
            let (value, done, stop) = {
                let Some(fade) = self.fades.get_mut(&handle) else {
                    continue;
                };
                fade.tween.advance(seconds);
                (
                    fade.tween.value(),
                    fade.tween.is_done(),
                    fade.stop_when_done,
                )
            };
            if done && stop {
                finished.push(handle);
                continue;
            }
            if !done {
                if let Some(params) = self.params(handle, value, false) {
                    self.backend.update(handle, &params, 0.0);
                }
            }
        }
        for handle in finished {
            self.pool.stop(handle);
            self.fades.remove(&handle);
        }
        self.pool.age();
    }

    /// The parameters a voice should be playing with right now.
    fn params(&self, handle: u64, fade: f32, looping: bool) -> Option<VoiceParams> {
        let voice = self.pool.voices().iter().find(|v| v.handle == handle)?;
        Some(VoiceParams {
            clip: voice.clip.clone(),
            bus: voice.bus,
            gain: db_to_linear(voice.volume_db) * self.bus_gain(voice.bus) * fade,
            pitch: voice.pitch,
            pan: voice.pan,
            looping,
        })
    }
}
