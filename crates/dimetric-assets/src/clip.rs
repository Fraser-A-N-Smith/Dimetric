//! Animation clips, and the conversion that has to happen at import time.

use serde::{Deserialize, Serialize};

/// Convert a frame duration in milliseconds to whole ticks.
///
/// **This runs at import time, never at runtime.** The artist's timings are in
/// milliseconds and the simulation counts ticks; converting on load would make
/// frame advance depend on whatever tick rate happened to be configured that
/// session, and a hitbox activation frame is gameplay, not decoration.
///
/// Rounds half away from zero and never returns zero, because a clip with a
/// zero-length frame advances infinitely fast and hangs the frame walker.
pub fn ms_to_ticks(milliseconds: u32, tick_rate: u32) -> u32 {
    debug_assert!(tick_rate > 0, "a tick rate of zero has no meaning");
    let numerator = milliseconds as u64 * tick_rate as u64;
    let ticks = (numerator * 2 + 1000) / 2000;
    ticks.max(1) as u32
}

/// One frame of an imported clip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Index into the sheet.
    pub index: u32,
    /// How long the frame is held, in ticks.
    pub ticks: u32,
    /// Event dispatched to scripts when this frame begins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// A named animation clip, imported from an Aseprite tag.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Clip {
    /// Tag name from the source document.
    pub name: String,
    /// Frames, in order.
    pub frames: Vec<Frame>,
    /// Whether the clip repeats.
    pub looping: bool,
}

impl Clip {
    /// How many ticks one pass through the clip takes.
    pub fn duration_ticks(&self) -> u32 {
        self.frames.iter().map(|f| f.ticks).sum()
    }

    /// Which frame is showing at `tick` into the clip.
    pub fn frame_at(&self, tick: u32) -> Option<&Frame> {
        if self.frames.is_empty() {
            return None;
        }
        let total = self.duration_ticks();
        let t = if self.looping && total > 0 {
            tick % total
        } else {
            tick.min(total.saturating_sub(1))
        };
        let mut elapsed = 0;
        for frame in &self.frames {
            elapsed += frame.ticks;
            if t < elapsed {
                return Some(frame);
            }
        }
        self.frames.last()
    }
}
