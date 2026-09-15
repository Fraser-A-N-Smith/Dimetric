//! What the simulation asks to be heard.
//!
//! A sound is presentation, and the line matters more here than anywhere else
//! in the engine. If triggering a sound touched the simulation — consumed a
//! random number, wrote a field that gets hashed — then muting a game would
//! change how it plays, and a replay recorded with audio on would diverge from
//! one played with it off. M6's acceptance criterion is exactly that: a
//! headless run with audio triggered produces the same state hash as a windowed
//! one.
//!
//! So the simulation does not play sounds. It *says what it would play*, into a
//! list that is cleared at the start of every tick, never hashed, and read by
//! whoever is listening — a device in the player, nothing at all in a headless
//! run. Which voice gets stolen, what pitch it plays at and whether there is a
//! sound card involved are all decided on the other side of that list.

use dimetric_core::{Fx, NodeUid};
use dimetric_scene::{Node, Value};

/// Something the simulation asked for this tick.
#[derive(Clone, PartialEq, Debug)]
pub enum SoundEvent {
    /// Start a sound.
    Play(SoundCue),
    /// Stop whatever a node started.
    Stop {
        /// The node whose sound should stop.
        node: NodeUid,
    },
}

/// A sound to start, as the scene authored it.
///
/// Every field comes from the `Sound` node's own properties, so what plays is
/// something a designer can see and an override can change, rather than an
/// argument buried in a script.
#[derive(Clone, PartialEq, Debug)]
pub struct SoundCue {
    /// The node that asked.
    pub node: NodeUid,
    /// Clip to play, as the scene names it.
    pub stream: String,
    /// Mixer bus, by name.
    pub bus: String,
    /// Gain in decibels.
    pub volume_db: Fx,
    /// Random pitch spread, applied on the presentation side.
    pub pitch_variation: Fx,
    /// Whether it repeats.
    pub looping: bool,
}

impl SoundCue {
    /// Read a cue off a `Sound` node, or nothing when the node is not one.
    pub fn of(node: &Node) -> Option<SoundCue> {
        if node.base != "Sound" {
            return None;
        }
        let stream = match node.get("stream") {
            Some(Value::Ref(reference)) => reference.to_text(),
            Some(Value::Str(text)) => text.clone(),
            _ => return None,
        };
        Some(SoundCue {
            node: node.uid,
            stream: stream.strip_prefix("asset:").unwrap_or(&stream).to_string(),
            bus: node
                .get("bus")
                .and_then(Value::as_str)
                .unwrap_or("Sfx")
                .to_string(),
            volume_db: node
                .get("volume_db")
                .and_then(Value::as_scalar)
                .unwrap_or(Fx::ZERO),
            pitch_variation: node
                .get("pitch_variation")
                .and_then(Value::as_scalar)
                .unwrap_or(Fx::ZERO),
            looping: node
                .get("looping")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// True when the node wants to start on its own.
    pub fn autoplays(node: &Node) -> bool {
        node.base == "Sound"
            && node
                .get("autoplay")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }
}
