//! Aseprite documents, imported as a sprite sheet plus named clips.
//!
//! Tags become clips directly, which is the point: frame animation needs no
//! editor UI at all, because the artist already authored it in the tool they
//! were going to use anyway. Rename a tag, reimport, and the clip is renamed.
//!
//! Frame durations are milliseconds in the source. They are converted to whole
//! ticks **here**, at import, by [`crate::ms_to_ticks`] — converting on load
//! would make frame advance depend on whatever tick rate happened to be
//! configured that session, and an activation frame is gameplay.
//!
//! # What is tested and what is trusted
//!
//! Decoding the binary format is `asefile`'s job. What this module adds —
//! expanding a playback direction into a plain frame list and converting
//! durations — is [`clip_from_range`], which takes no Aseprite types and is
//! tested directly. The glue between them is a field-for-field mapping.

use asefile::AsepriteFile;

use crate::clip::{ms_to_ticks, Clip, Frame};
use crate::image::{Image, ImageError};

/// What an Aseprite document imports to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Aseprite {
    /// Every frame, laid out in one horizontal strip.
    ///
    /// A strip rather than a grid so a frame index is a multiplication rather
    /// than a division and a remainder, and so the sheet's own layout carries
    /// no information a reader has to be told separately.
    pub sheet: Image,
    /// Width of one frame, in pixels.
    pub frame_width: u32,
    /// Height of one frame, in pixels.
    pub frame_height: u32,
    /// How many frames the strip holds.
    pub frame_count: u32,
    /// Clips from the document's tags.
    pub clips: Vec<Clip>,
}

impl Aseprite {
    /// The clip with a given name.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }
}

/// Read an Aseprite document.
///
/// `tick_rate` is the project's, and it is baked into the clips that come back.
pub fn import(path: &std::path::Path, tick_rate: u32) -> Result<Aseprite, ImageError> {
    let file = AsepriteFile::read_file(path).map_err(|e| ImageError::decode(path, e))?;
    let (frame_width, frame_height) = (file.width() as u32, file.height() as u32);
    let frame_count = file.num_frames();

    // Every layer flattened, because a layer stack is an authoring convenience
    // and the renderer draws one quad.
    let mut sheet = Image::blank(
        "",
        frame_width.max(1) * frame_count.max(1),
        frame_height.max(1),
    );
    for index in 0..frame_count {
        let rendered = file.frame(index).image();
        let frame = Image {
            name: String::new(),
            width: rendered.width(),
            height: rendered.height(),
            pixels: rendered.into_raw(),
        };
        sheet.blit(&frame, index * frame_width, 0);
    }

    let durations: Vec<u32> = (0..frame_count).map(|i| file.frame(i).duration()).collect();
    let mut clips: Vec<Clip> = (0..file.num_tags())
        .map(|i| clip_of(file.tag(i), &durations, tick_rate))
        .collect();

    // An untagged document still animates: one clip covering every frame, so a
    // simple spinning coin does not force the artist to tag it first.
    if clips.is_empty() && frame_count > 0 {
        clips.push(Clip {
            name: "default".to_string(),
            frames: (0..frame_count)
                .map(|i| Frame {
                    index: i,
                    ticks: ms_to_ticks(durations[i as usize], tick_rate),
                    event: None,
                })
                .collect(),
            looping: true,
        });
    }

    Ok(Aseprite {
        sheet,
        frame_width,
        frame_height,
        frame_count,
        clips,
    })
}

/// Which way a tag plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Playback {
    /// First frame to last.
    Forward,
    /// Last frame to first.
    Reverse,
    /// Forward, then back without repeating either end.
    PingPong,
}

/// Build a clip from a frame range, a playback direction and the durations.
///
/// Ping-pong and reverse are playback *directions* rather than frame orders,
/// and the runtime frame walker only knows how to go forwards. Expanding them
/// here means the clip a script sees is always a plain list, which is also what
/// makes a clip's tick count something you can read off it.
pub fn clip_from_range(
    name: &str,
    from: u32,
    to: u32,
    playback: Playback,
    durations: &[u32],
    tick_rate: u32,
) -> Clip {
    let (from, to) = (from.min(to), from.max(to));
    let order: Vec<u32> = match playback {
        Playback::Forward => (from..=to).collect(),
        Playback::Reverse => (from..=to).rev().collect(),
        // Skipping both ends on the way back is what stops the turnaround
        // frames being held for twice as long as every other frame.
        Playback::PingPong => (from..=to)
            .chain(
                (from..=to)
                    .rev()
                    .skip(1)
                    .take((to - from).saturating_sub(1) as usize),
            )
            .collect(),
    };
    Clip {
        name: name.to_string(),
        frames: order
            .into_iter()
            .map(|i| Frame {
                index: i,
                ticks: ms_to_ticks(durations.get(i as usize).copied().unwrap_or(100), tick_rate),
                event: None,
            })
            .collect(),
        // Aseprite has no "plays once" flag on a tag, so every clip loops and a
        // script that wants one pass stops it itself.
        looping: true,
    }
}

/// Map one Aseprite tag onto [`clip_from_range`].
fn clip_of(tag: &asefile::Tag, durations: &[u32], tick_rate: u32) -> Clip {
    let playback = match tag.animation_direction() {
        asefile::AnimationDirection::Reverse => Playback::Reverse,
        asefile::AnimationDirection::PingPong => Playback::PingPong,
        _ => Playback::Forward,
    };
    clip_from_range(
        tag.name(),
        tag.from_frame(),
        tag.to_frame(),
        playback,
        durations,
        tick_rate,
    )
}
