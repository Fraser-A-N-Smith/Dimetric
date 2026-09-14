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
use dimetric_scene::Value;

use crate::state::AnimState;

/// Clips by asset name, as the importer produced them.
pub type Clips = BTreeMap<String, Vec<Clip>>;

/// Find one clip by asset and name.
pub fn find<'a>(clips: &'a Clips, asset: &str, name: &str) -> Option<&'a Clip> {
    clips.get(asset)?.iter().find(|c| c.name == name)
}

/// Advance every playing animation by one tick.
///
/// Reads each `AnimatedSprite2D` node's `animation` and `playing` properties,
/// advances the clip its own sheet declares, and writes the resulting frame
/// index back onto the node. The write is what the renderer reads: the
/// simulation never hands the renderer anything but the scene (I7).
///
/// Returns the animation events that fired, in node order, for the caller to
/// dispatch. Dispatching here would mean calling a script while holding the
/// state borrow this function needs.
pub fn advance(
    scene: &mut dimetric_scene::Scene,
    anim: &mut BTreeMap<NodeUid, AnimState>,
    clips: &Clips,
) -> Vec<(NodeUid, String)> {
    let mut events = Vec::new();
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        if node.kind != "AnimatedSprite2D" {
            continue;
        }
        let uid = node.uid;
        // The sheet the node draws from decides which clips it can play. Two
        // sheets are allowed a clip called "walk" each.
        let Some(sheet) = node
            .get("frames")
            .and_then(Value::as_ref_value)
            .map(|r| r.target().to_string())
        else {
            continue;
        };
        let wanted = node
            .get("animation")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let wanted = if wanted.is_empty() {
            match clips.get(&sheet).and_then(|list| list.first()) {
                Some(clip) => clip.name.clone(),
                None => continue,
            }
        } else {
            wanted
        };
        let playing = node.get("playing").and_then(Value::as_bool).unwrap_or(true);
        // An integer ratio rather than a float, because frame advance is
        // gameplay and has to land on the same tick everywhere.
        let speed = Speed {
            numerator: node
                .get("speed_numerator")
                .and_then(Value::as_int)
                .unwrap_or(1)
                .max(1) as u32,
            denominator: node
                .get("speed_denominator")
                .and_then(Value::as_int)
                .unwrap_or(1)
                .max(1) as u32,
        };

        let state = anim.entry(uid).or_insert_with(|| AnimState {
            clip: wanted.clone(),
            frame: 0,
            ticks_in_frame: 0,
            playing,
            finished: false,
        });
        // Switching clips restarts; asking for the clip already running does
        // not, so setting `animation` every tick is harmless.
        if state.clip != wanted {
            state.clip = wanted;
            state.frame = 0;
            state.ticks_in_frame = 0;
            state.finished = false;
        }
        state.playing = playing;

        if let Some(event) = step(
            state,
            clips.get(&sheet).map(Vec::as_slice).unwrap_or(&[]),
            speed,
        ) {
            events.push((uid, event));
        }

        let frame = state.frame as i64;
        if let Some(node) = scene.get_mut(id) {
            node.props.insert("frame".to_string(), Value::Int(frame));
        }
    }

    // Nodes a script drives directly through `anim.play`, which need no sheet
    // property and no node kind.
    let scripted: Vec<NodeUid> = anim
        .keys()
        .copied()
        .filter(|uid| {
            scene
                .by_uid(*uid)
                .and_then(|id| scene.get(id))
                .is_none_or(|n| n.kind != "AnimatedSprite2D")
        })
        .collect();
    for uid in scripted {
        let Some(state) = anim.get_mut(&uid) else {
            continue;
        };
        let all: Vec<Clip> = clips.values().flatten().cloned().collect();
        if let Some(event) = step(state, &all, Speed::NORMAL) {
            events.push((uid, event));
        }
    }

    events
}

/// How fast a node plays its clips, as a ratio.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Speed {
    /// Multiply the rate by this.
    pub numerator: u32,
    /// Divide the rate by this.
    pub denominator: u32,
}

impl Speed {
    /// One to one.
    pub const NORMAL: Speed = Speed {
        numerator: 1,
        denominator: 1,
    };

    /// How long a frame authored for `ticks` is held at this speed.
    ///
    /// Integer arithmetic, and never zero: a frame no ticks long advances
    /// infinitely fast and hangs the walker. Doubling the speed halves the
    /// duration, which is why the numerator divides here.
    pub fn hold(self, ticks: u32) -> u32 {
        let numerator = self.numerator.max(1) as u64;
        let denominator = self.denominator.max(1) as u64;
        (((ticks as u64) * denominator).div_ceil(numerator)).max(1) as u32
    }
}

/// Move one animation on by a tick. Returns an event if the new frame carries
/// one.
fn step(state: &mut AnimState, clips: &[Clip], speed: Speed) -> Option<String> {
    if !state.playing || state.finished {
        return None;
    }
    // A clip that is not loaded holds its frame rather than resetting: a
    // missing asset should look wrong, not stop the tick.
    let clip = clips.iter().find(|c| c.name == state.clip)?;
    if clip.frames.is_empty() {
        return None;
    }

    state.ticks_in_frame = state.ticks_in_frame.saturating_add(1);
    let current = state.frame.min(clip.frames.len() as u32 - 1) as usize;
    if state.ticks_in_frame < speed.hold(clip.frames[current].ticks) {
        return None;
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
            return None;
        }
    } else {
        state.frame = next as u32;
    }

    clip.frames[state.frame as usize].event.clone()
}
