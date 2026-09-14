//! The `kira` backend: the one that makes noise.
//!
//! Behind the `kira` feature, and off by default. It pulls in a device stack —
//! ALSA and D-Bus development headers on Linux — and a headless run, an agent
//! session and CI all want the mock backend anyway. A build that needs no
//! system audio libraries to run the test suite is worth the feature flag.
//!
//! # The rule worth repeating
//!
//! `kira`'s clock scheduling is for music only. Wiring gameplay to the audio
//! clock is an I5 violation wearing a feature's clothing: it looks like
//! rhythm-game support and it is a replay that depends on how far behind the
//! sound card is. Nothing in this file reads a clock.

use std::collections::BTreeMap;
use std::time::Duration;

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, PlaybackRate};

use crate::backend::{AudioError, Backend, VoiceParams};
use crate::Bus;

/// Plays sound through the system's audio device.
pub struct Kira {
    /// Held, not read: dropping the manager tears down the audio thread and
    /// silences every track that came from it.
    #[allow(dead_code)]
    manager: AudioManager<DefaultBackend>,
    tracks: BTreeMap<Bus, TrackHandle>,
    clips: BTreeMap<String, StaticSoundData>,
    voices: BTreeMap<u64, StaticSoundHandle>,
}

impl Kira {
    /// Open the default output device and build one track per bus.
    pub fn new() -> Result<Kira, AudioError> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let mut tracks = BTreeMap::new();
        for bus in Bus::ALL {
            let track = manager
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| AudioError::Device(e.to_string()))?;
            tracks.insert(bus, track);
        }
        Ok(Kira {
            manager,
            tracks,
            clips: BTreeMap::new(),
            voices: BTreeMap::new(),
        })
    }
}

fn tween(seconds: f32) -> kira::Tween {
    kira::Tween {
        duration: Duration::from_secs_f32(seconds.max(0.0)),
        ..Default::default()
    }
}

/// Linear gain as decibels, with zero mapped to silence rather than `-inf`.
fn decibels(gain: f32) -> Decibels {
    if gain <= 0.000_1 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * gain.log10())
    }
}

impl Backend for Kira {
    fn load(&mut self, clip: &str, bytes: &[u8]) -> Result<(), AudioError> {
        let data =
            StaticSoundData::from_cursor(std::io::Cursor::new(bytes.to_vec())).map_err(|e| {
                AudioError::Decode {
                    clip: clip.to_string(),
                    detail: e.to_string(),
                }
            })?;
        self.clips.insert(clip.to_string(), data);
        Ok(())
    }

    fn is_loaded(&self, clip: &str) -> bool {
        self.clips.contains_key(clip)
    }

    fn start(&mut self, handle: u64, params: &VoiceParams) -> Result<(), AudioError> {
        let data = self
            .clips
            .get(&params.clip)
            .ok_or_else(|| AudioError::UnknownClip(params.clip.clone()))?
            .clone()
            .volume(decibels(params.gain))
            .playback_rate(PlaybackRate(params.pitch as f64))
            .panning(Panning(params.pan.clamp(-1.0, 1.0)));
        let data = if params.looping {
            data.loop_region(0.0..)
        } else {
            data
        };
        let track = self
            .tracks
            .get_mut(&params.bus)
            .ok_or_else(|| AudioError::Device(format!("no track for {:?}", params.bus)))?;
        let sound = track
            .play(data)
            .map_err(|e| AudioError::Device(e.to_string()))?;
        self.voices.insert(handle, sound);
        Ok(())
    }

    fn update(&mut self, handle: u64, params: &VoiceParams, seconds: f32) {
        let Some(sound) = self.voices.get_mut(&handle) else {
            return;
        };
        sound.set_volume(decibels(params.gain), tween(seconds));
        sound.set_playback_rate(PlaybackRate(params.pitch as f64), tween(seconds));
        sound.set_panning(Panning(params.pan.clamp(-1.0, 1.0)), tween(seconds));
    }

    fn stop(&mut self, handle: u64, seconds: f32) {
        if let Some(mut sound) = self.voices.remove(&handle) {
            sound.stop(tween(seconds));
        }
    }

    fn set_bus_gain(&mut self, bus: Bus, gain: f32, seconds: f32) {
        if let Some(track) = self.tracks.get_mut(&bus) {
            track.set_volume(decibels(gain), tween(seconds));
        }
    }

    fn name(&self) -> &'static str {
        "kira"
    }
}
