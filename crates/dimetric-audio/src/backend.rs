//! Where sound actually goes.
//!
//! The mixer decides what plays, at what gain, on which bus. A backend takes
//! those decisions and makes noise, or — in the headless case — writes them
//! down. Keeping the two apart is what lets every interesting rule about voice
//! stealing and fades be tested without a sound card anywhere near it.

use crate::Bus;

/// What a backend needs to know to play a voice.
#[derive(Clone, PartialEq, Debug)]
pub struct VoiceParams {
    /// Clip name, as a scene refers to it.
    pub clip: String,
    /// Bus it routes through.
    pub bus: Bus,
    /// Linear gain, bus gain already applied.
    pub gain: f32,
    /// Playback rate multiplier.
    pub pitch: f32,
    /// Stereo position, `-1.0` left to `1.0` right.
    pub pan: f32,
    /// Whether it repeats.
    pub looping: bool,
}

/// Why a backend could not do something.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The clip has not been handed to the backend.
    #[error("no clip named {0} has been loaded")]
    UnknownClip(String),
    /// The bytes were not audio this build can decode.
    #[error("cannot decode {clip}: {detail}")]
    Decode {
        /// Clip that failed.
        clip: String,
        /// What the decoder said.
        detail: String,
    },
    /// The device refused.
    #[error("audio device: {0}")]
    Device(String),
}

/// Something that can play sounds.
pub trait Backend {
    /// Hand the backend a clip's encoded bytes.
    fn load(&mut self, clip: &str, bytes: &[u8]) -> Result<(), AudioError>;

    /// Whether a clip has been loaded.
    fn is_loaded(&self, clip: &str) -> bool;

    /// Start a voice.
    fn start(&mut self, handle: u64, params: &VoiceParams) -> Result<(), AudioError>;

    /// Change a playing voice's gain, pitch or panning, over `seconds`.
    fn update(&mut self, handle: u64, params: &VoiceParams, seconds: f32);

    /// Stop a voice, fading out over `seconds`.
    fn stop(&mut self, handle: u64, seconds: f32);

    /// Set a bus's gain, over `seconds`.
    fn set_bus_gain(&mut self, bus: Bus, gain: f32, seconds: f32);

    /// What this backend is called, for diagnostics.
    fn name(&self) -> &'static str;
}

/// One thing a backend was asked to do.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    /// A clip's bytes arrived.
    Loaded {
        /// Clip name.
        clip: String,
        /// How many bytes.
        bytes: usize,
    },
    /// A voice started.
    Started {
        /// Voice handle.
        handle: u64,
        /// What it was asked to play.
        params: VoiceParams,
    },
    /// A playing voice changed.
    Updated {
        /// Voice handle.
        handle: u64,
        /// What it now is.
        params: VoiceParams,
        /// Over how long.
        seconds: f32,
    },
    /// A voice stopped.
    Stopped {
        /// Voice handle.
        handle: u64,
        /// Fade-out length.
        seconds: f32,
    },
    /// A bus's gain changed.
    BusGain {
        /// Which bus.
        bus: Bus,
        /// New gain.
        gain: f32,
        /// Over how long.
        seconds: f32,
    },
}

/// A backend that makes no sound and remembers everything it was asked to do.
///
/// This is what a headless run and CI select, so an agent session produces no
/// device I/O at all. It is also how the mixer's rules get tested: the question
/// "did the footstep get stolen" is answerable by reading a list.
#[derive(Debug, Default)]
pub struct Mock {
    events: Log,
    loaded: std::collections::BTreeSet<String>,
}

/// A shared list of what a mock backend was asked to do.
///
/// Shared because a mixer owns its backend, and a test that wants to know
/// whether a stolen voice was actually stopped has to be able to look.
pub type Log = std::rc::Rc<std::cell::RefCell<Vec<Event>>>;

impl Mock {
    /// A fresh mock backend.
    pub fn new() -> Mock {
        Mock::default()
    }

    /// A mock backend writing to a list the caller holds.
    pub fn with_log(events: Log) -> Mock {
        Mock {
            events,
            loaded: Default::default(),
        }
    }

    /// The list it writes to.
    pub fn log(&self) -> Log {
        self.events.clone()
    }
}

/// The clips that started, in order.
pub fn started(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Started { params, .. } => Some(params.clip.as_str()),
            _ => None,
        })
        .collect()
}

/// The voices that were stopped, in order.
pub fn stopped(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Stopped { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect()
}

impl Backend for Mock {
    fn load(&mut self, clip: &str, bytes: &[u8]) -> Result<(), AudioError> {
        self.loaded.insert(clip.to_string());
        self.events.borrow_mut().push(Event::Loaded {
            clip: clip.to_string(),
            bytes: bytes.len(),
        });
        Ok(())
    }

    fn is_loaded(&self, clip: &str) -> bool {
        self.loaded.contains(clip)
    }

    fn start(&mut self, handle: u64, params: &VoiceParams) -> Result<(), AudioError> {
        if !self.loaded.contains(&params.clip) {
            return Err(AudioError::UnknownClip(params.clip.clone()));
        }
        self.events.borrow_mut().push(Event::Started {
            handle,
            params: params.clone(),
        });
        Ok(())
    }

    fn update(&mut self, handle: u64, params: &VoiceParams, seconds: f32) {
        self.events.borrow_mut().push(Event::Updated {
            handle,
            params: params.clone(),
            seconds,
        });
    }

    fn stop(&mut self, handle: u64, seconds: f32) {
        self.events
            .borrow_mut()
            .push(Event::Stopped { handle, seconds });
    }

    fn set_bus_gain(&mut self, bus: Bus, gain: f32, seconds: f32) {
        self.events
            .borrow_mut()
            .push(Event::BusGain { bus, gain, seconds });
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}
