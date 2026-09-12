//! Mixer buses and voice management.
//!
//! Audio is presentation. It may use floats and wall-clock time freely, and —
//! subject to invariant I7 — none of its state is ever snapshotted or fed into
//! a state hash. A headless run with sound triggered must produce exactly the
//! same hash as a windowed one, or replay would depend on audio device latency.
//!
//! # What is here and what is not
//!
//! Voice allocation, stealing, per-clip caps and bus gain are implemented and
//! tested, because they are pure bookkeeping and they are where the bugs are.
//! The `kira` backend that actually makes noise is **not in this build**.
//!
//! # The rule worth repeating
//!
//! `kira`'s clock scheduling is for music only. Wiring gameplay to the audio
//! clock is an I5 violation wearing a feature's clothing: it looks like
//! rhythm-game support and it is a replay that depends on how far behind the
//! sound card is.

#![warn(missing_docs)]

use std::collections::BTreeMap;

use dimetric_core::Rng;
use serde::{Deserialize, Serialize};

/// Where a sound is routed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Bus {
    /// Music.
    Music,
    /// Sound effects.
    #[default]
    Sfx,
    /// Interface sounds, which usually keep playing when the game is paused.
    Ui,
}

impl Bus {
    /// Every bus, in a fixed order.
    pub const ALL: [Bus; 3] = [Bus::Music, Bus::Sfx, Bus::Ui];

    /// Parse the scene-file spelling.
    pub fn parse(name: &str) -> Option<Bus> {
        Some(match name {
            "Music" => Bus::Music,
            "Sfx" => Bus::Sfx,
            "Ui" => Bus::Ui,
            _ => return None,
        })
    }

    /// The bus's name.
    pub fn name(self) -> &'static str {
        match self {
            Bus::Music => "Music",
            Bus::Sfx => "Sfx",
            Bus::Ui => "Ui",
        }
    }
}

/// A playing sound.
#[derive(Clone, PartialEq, Debug)]
pub struct Voice {
    /// Clip being played.
    pub clip: String,
    /// Bus it routes through.
    pub bus: Bus,
    /// Gain in decibels, before bus gain.
    pub volume_db: f32,
    /// Playback rate multiplier.
    pub pitch: f32,
    /// Stereo position, `-1.0` left to `1.0` right.
    pub pan: f32,
    /// Higher priority voices are stolen last.
    pub priority: u8,
    /// Which trigger this voice belongs to, for stopping it later.
    pub handle: u64,
    /// Ticks since it started, for the stealing heuristic.
    pub age: u32,
}

/// Why a trigger did not produce a voice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejected {
    /// The clip is already at its own cap.
    ClipCapReached,
    /// Every voice is in use and none could be stolen.
    NoVoiceAvailable,
}

/// A fixed pool of voices with stealing and per-clip caps.
///
/// A pool rather than unbounded playback because the slice game triggers
/// hundreds of overlapping spell sounds, and a hundred copies of one impact
/// sound is both inaudible and expensive. The per-clip cap is what stops one
/// noisy effect from evicting everything else.
#[derive(Clone, Debug)]
pub struct VoicePool {
    voices: Vec<Voice>,
    capacity: usize,
    per_clip_cap: usize,
    next_handle: u64,
    gains: BTreeMap<Bus, f32>,
}

impl VoicePool {
    /// A pool with the given total capacity and per-clip limit.
    pub fn new(capacity: usize, per_clip_cap: usize) -> VoicePool {
        VoicePool {
            voices: Vec::with_capacity(capacity),
            capacity,
            per_clip_cap,
            next_handle: 1,
            gains: Bus::ALL.iter().map(|b| (*b, 1.0)).collect(),
        }
    }

    /// Currently playing voices.
    pub fn voices(&self) -> &[Voice] {
        &self.voices
    }

    /// How many voices are playing.
    pub fn len(&self) -> usize {
        self.voices.len()
    }

    /// True when nothing is playing.
    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// Set a bus's gain, `0.0` silent to `1.0` unity.
    pub fn set_gain(&mut self, bus: Bus, gain: f32) {
        self.gains.insert(bus, gain.clamp(0.0, 4.0));
    }

    /// A bus's gain.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.gains.get(&bus).copied().unwrap_or(1.0)
    }

    /// Start a sound, stealing a voice if the pool is full.
    ///
    /// Stealing picks the oldest voice of the lowest priority. Oldest because a
    /// sound that has been playing longest has already been heard; lowest
    /// priority first because a footstep should lose to a boss roar.
    pub fn play(&mut self, mut voice: Voice) -> Result<u64, Rejected> {
        let same_clip = self.voices.iter().filter(|v| v.clip == voice.clip).count();
        if same_clip >= self.per_clip_cap {
            return Err(Rejected::ClipCapReached);
        }
        if self.voices.len() >= self.capacity {
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.priority <= voice.priority)
                .max_by_key(|(_, v)| (std::cmp::Reverse(v.priority), v.age))
                .map(|(i, _)| i);
            match victim {
                Some(index) => {
                    self.voices.remove(index);
                }
                None => return Err(Rejected::NoVoiceAvailable),
            }
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        voice.handle = handle;
        voice.age = 0;
        self.voices.push(voice);
        Ok(handle)
    }

    /// Stop one voice.
    pub fn stop(&mut self, handle: u64) -> bool {
        let before = self.voices.len();
        self.voices.retain(|v| v.handle != handle);
        self.voices.len() != before
    }

    /// Stop every voice on a bus.
    pub fn stop_bus(&mut self, bus: Bus) {
        self.voices.retain(|v| v.bus != bus);
    }

    /// Age every voice by one frame.
    pub fn age(&mut self) {
        for voice in &mut self.voices {
            voice.age = voice.age.saturating_add(1);
        }
    }

    /// The effective gain of a voice, including its bus.
    pub fn effective_gain(&self, voice: &Voice) -> f32 {
        db_to_linear(voice.volume_db) * self.gain(voice.bus)
    }
}

/// Convert decibels to a linear gain.
pub fn db_to_linear(db: f32) -> f32 {
    // I3-exempt: presentation-side audio maths.
    10f32.powf(db / 20.0)
}

/// Pick a pitch multiplier within `±spread` of unity.
///
/// Drawn from a **presentation** RNG stream, deliberately separate from the
/// simulation's. Sharing a stream would mean that turning the sound off, or
/// triggering one fewer sound effect, shifted every gameplay roll after it.
pub fn pitch_variation(rng: &mut Rng, spread: f32) -> f32 {
    // I3-exempt: presentation-side audio maths.
    if spread <= 0.0 {
        return 1.0;
    }
    let unit = rng.unit_fx().to_f32() * 2.0 - 1.0;
    1.0 + unit * spread.clamp(0.0, 1.0)
}

/// The audio backend in use.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Backend {
    /// No device. Headless runs and CI select this, so an agent session and a
    /// test suite produce no device I/O at all.
    #[default]
    Mock,
    /// The real `kira` backend. Not in this build.
    Kira,
}
