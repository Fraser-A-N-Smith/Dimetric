//! Player input, and the log format replays are driven from.
//!
//! Input is a plain struct the simulation reads. Where it came from — a
//! gamepad, a log file, or one day a socket — is invisible from inside a tick.
//! That indistinguishability is invariant I8's practical form, and it is most
//! of what makes replay and, later, rollback possible.

use std::fmt::Write as _;

use dimetric_core::{Angle, Fx, StateHash, StateHasher, Vec2Fx};
use serde::{Deserialize, Serialize};

/// Button bits. Gameplay names them; the engine only moves the bits around.
pub mod buttons {
    /// Primary action.
    pub const FIRE: u32 = 1 << 0;
    /// Secondary action.
    pub const ALT: u32 = 1 << 1;
    /// Dash or dodge.
    pub const DASH: u32 = 1 << 2;
    /// Interact.
    pub const USE: u32 = 1 << 3;
    /// Pause. Read outside the simulation.
    pub const PAUSE: u32 = 1 << 4;
}

/// One player's input for one tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Held buttons.
    pub buttons: u32,
    /// Movement stick, expected to be within the unit circle.
    pub move_dir: Vec2Fx,
    /// Aim direction.
    pub aim: Angle,
}

impl PlayerInput {
    /// True when a button is held.
    #[inline]
    pub fn held(&self, button: u32) -> bool {
        self.buttons & button != 0
    }

    /// True when a button is held now and was not last tick.
    #[inline]
    pub fn pressed(&self, previous: &PlayerInput, button: u32) -> bool {
        self.held(button) && !previous.held(button)
    }

    /// Feed into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.u64(self.buttons as u64).vec2(self.move_dir).angle(self.aim);
    }
}

/// Every player's input for one tick.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct InputFrame {
    /// Per-player, in player-number order.
    pub players: Vec<PlayerInput>,
}

impl InputFrame {
    /// A frame with `n` idle players.
    pub fn idle(n: usize) -> InputFrame {
        InputFrame {
            players: vec![PlayerInput::default(); n],
        }
    }

    /// One player's input, or idle when that player is absent.
    pub fn player(&self, index: usize) -> PlayerInput {
        self.players.get(index).copied().unwrap_or_default()
    }

    /// Feed into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.tag("input").len(self.players.len());
        for p in &self.players {
            p.hash_state(h);
        }
    }
}

/// A recorded run: a seed, what it was recorded against, and one frame per
/// tick.
///
/// Text and diffable on purpose. An input log is evidence — when a replay
/// diverges, being able to read and bisect the log by hand is worth more than
/// the bytes a binary format would save.
#[derive(Clone, PartialEq, Debug)]
pub struct InputLog {
    /// Run seed.
    pub seed: u64,
    /// Engine version that recorded it.
    pub engine: String,
    /// Hash of the scene it was recorded against.
    pub scene_hash: Option<StateHash>,
    /// Players in the run.
    pub player_count: usize,
    /// One frame per tick, from tick zero.
    pub frames: Vec<InputFrame>,
}

/// The header tag on every input log.
pub const LOG_TAG: &str = "dimetric-input";
/// Input log format version.
pub const LOG_VERSION: u32 = 1;

impl InputLog {
    /// An empty log.
    pub fn new(seed: u64, engine: impl Into<String>, player_count: usize) -> InputLog {
        InputLog {
            seed,
            engine: engine.into(),
            scene_hash: None,
            player_count,
            frames: Vec::new(),
        }
    }

    /// Append a frame.
    pub fn push(&mut self, frame: InputFrame) {
        self.frames.push(frame);
    }

    /// The frame for a tick, or idle input past the end of the log.
    ///
    /// Running past the end is normal: a replay may be asked for more ticks
    /// than were recorded, and idle input is the honest answer.
    pub fn frame(&self, tick: u64) -> InputFrame {
        self.frames
            .get(tick as usize)
            .cloned()
            .unwrap_or_else(|| InputFrame::idle(self.player_count))
    }

    /// Render as text.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{LOG_TAG} {LOG_VERSION}");
        let _ = writeln!(out, "seed {}", self.seed);
        let _ = writeln!(out, "engine {}", self.engine);
        if let Some(h) = self.scene_hash {
            let _ = writeln!(out, "scene {h}");
        }
        let _ = writeln!(out, "players {}", self.player_count);
        let _ = writeln!(out, "# tick  then buttons move_x move_y aim, per player");
        for (tick, frame) in self.frames.iter().enumerate() {
            let _ = write!(out, "{tick}");
            for i in 0..self.player_count {
                let p = frame.player(i);
                let _ = write!(
                    out,
                    " {:04x} {} {} {}",
                    p.buttons,
                    p.move_dir.x.to_exact_string(),
                    p.move_dir.y.to_exact_string(),
                    p.aim.to_degrees_string()
                );
            }
            out.push('\n');
        }
        out
    }

    /// Parse from text.
    pub fn parse(text: &str) -> Result<InputLog, LogError> {
        let mut seed = None;
        let mut engine = String::new();
        let mut scene_hash = None;
        let mut player_count = 1usize;
        let mut frames = Vec::new();
        let mut saw_tag = false;

        for (number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let head = parts.next().unwrap_or_default();
            match head {
                LOG_TAG => {
                    let v: u32 = parse_field(parts.next(), number)?;
                    if v != LOG_VERSION {
                        return Err(LogError::Version(v));
                    }
                    saw_tag = true;
                }
                "seed" => seed = Some(parse_field(parts.next(), number)?),
                "engine" => engine = parts.next().unwrap_or_default().to_string(),
                "scene" => {
                    scene_hash = parts.next().and_then(StateHash::from_hex);
                }
                "players" => player_count = parse_field(parts.next(), number)?,
                _ => {
                    // A tick line. The tick number is positional and checked,
                    // so a log with a missing line fails loudly instead of
                    // silently shifting every input by one tick.
                    let tick: usize = head
                        .parse()
                        .map_err(|_| LogError::Malformed(number + 1, raw.to_string()))?;
                    if tick != frames.len() {
                        return Err(LogError::OutOfOrder {
                            line: number + 1,
                            expected: frames.len(),
                            found: tick,
                        });
                    }
                    let mut players = Vec::with_capacity(player_count);
                    for _ in 0..player_count {
                        let buttons = u32::from_str_radix(
                            parts
                                .next()
                                .ok_or_else(|| LogError::Malformed(number + 1, raw.to_string()))?,
                            16,
                        )
                        .map_err(|_| LogError::Malformed(number + 1, raw.to_string()))?;
                        let x = parse_fx(parts.next(), number)?;
                        let y = parse_fx(parts.next(), number)?;
                        let aim = Angle::from_degrees_str(
                            parts
                                .next()
                                .ok_or_else(|| LogError::Malformed(number + 1, raw.to_string()))?,
                        )
                        .map_err(|_| LogError::Malformed(number + 1, raw.to_string()))?;
                        players.push(PlayerInput {
                            buttons,
                            move_dir: Vec2Fx::new(x, y),
                            aim,
                        });
                    }
                    frames.push(InputFrame { players });
                }
            }
        }

        if !saw_tag {
            return Err(LogError::NotAnInputLog);
        }
        Ok(InputLog {
            seed: seed.ok_or(LogError::MissingSeed)?,
            engine,
            scene_hash,
            player_count,
            frames,
        })
    }
}

fn parse_field<T: std::str::FromStr>(v: Option<&str>, line: usize) -> Result<T, LogError> {
    v.and_then(|s| s.parse().ok())
        .ok_or_else(|| LogError::Malformed(line + 1, v.unwrap_or("").to_string()))
}

fn parse_fx(v: Option<&str>, line: usize) -> Result<Fx, LogError> {
    v.and_then(|s| Fx::parse_exact(s).ok())
        .ok_or_else(|| LogError::Malformed(line + 1, v.unwrap_or("").to_string()))
}

/// Why an input log could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogError {
    /// No `dimetric-input` header.
    #[error("this is not an input log; it should start with `{LOG_TAG} {LOG_VERSION}`")]
    NotAnInputLog,
    /// Wrong format version.
    #[error("input log format version {0} is not supported")]
    Version(u32),
    /// No seed line.
    #[error("input log has no seed, so the run it recorded cannot be reproduced")]
    MissingSeed,
    /// A line did not parse.
    #[error("line {0} is malformed: {1:?}")]
    Malformed(usize, String),
    /// Tick numbers are not consecutive from zero.
    #[error("line {line}: expected tick {expected}, found {found}; input logs have one line per tick with no gaps")]
    OutOfOrder {
        /// Line number.
        line: usize,
        /// Tick the line should have carried.
        expected: usize,
        /// Tick it actually carried.
        found: usize,
    },
}
