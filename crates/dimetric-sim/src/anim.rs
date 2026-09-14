//! Frame animation playback.
//!
//! The clips come from the importer, already in ticks. Nothing here converts
//! milliseconds, and that is the point: the timing an artist authored was
//! resolved once, at import, against the project's tick rate, rather than
//! against whatever rate a session happened to be running at.
//!
//! Frames can carry an event name. When playback reaches such a frame the node
//! gets `on_anim_event`, which is how a hitbox opens on the swing frame rather
//! than on a timer a designer had to keep in sync by hand.

use std::collections::BTreeMap;

use dimetric_assets::Clip;
use dimetric_core::NodeUid;

use crate::state::AnimState;

/// Clips by asset name, as the importer produced them.
pub type Clips = BTreeMap<String, Vec<Clip>>;

/// Find one clip by asset and name.
pub fn find<'a>(clips: &'a Clips, asset: &str, name: &str) -> Option<&'a Clip> {
    clips.get(asset)?.iter().find(|c| c.name == name)
}

/// Advance every playing animation by one tick.
///
/// Returns the animation events that fired, in node order, for the caller to
/// dispatch. Dispatching here would mean calling a script while holding the
/// state borrow that this function needs.
pub fn advance(anim: &mut BTreeMap<NodeUid, AnimState>, clips: &Clips) -> Vec<(NodeUid, String)> {
    let mut events = Vec::new();
    for (uid, state) in anim.iter_mut() {
        if !state.playing || state.finished {
            continue;
        }
        let Some(clip) = clips.values().flatten().find(|c| c.name == state.clip) else {
            // A clip that is not loaded holds its frame rather than resetting.
            // A missing asset should look wrong, not crash the tick.
            continue;
        };
        if clip.frames.is_empty() {
            continue;
        }

        state.ticks_in_frame = state.ticks_in_frame.saturating_add(1);
        let current = state.frame.min(clip.frames.len() as u32 - 1) as usize;
        if state.ticks_in_frame < clip.frames[current].ticks {
            continue;
        }

        state.ticks_in_frame = 0;
        let next = current + 1;
        if next >= clip.frames.len() {
            if clip.looping {
                state.frame = 0;
            } else {
                state.frame = (clip.frames.len() - 1) as u32;
                state.finished = true;
                state.playing = false;
                continue;
            }
        } else {
            state.frame = next as u32;
        }

        if let Some(event) = clip.frames[state.frame as usize].event.as_ref() {
            events.push((*uid, event.clone()));
        }
    }
    events
}
