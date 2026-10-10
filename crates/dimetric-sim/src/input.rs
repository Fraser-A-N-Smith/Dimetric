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

    /// The first bit a project's own action can take.
    ///
    /// The five above are the engine's and never move. Everything from here up
    /// belongs to whatever the project declared, in the order it declared it —
    /// see [`super::InputLog::actions`] for why that order is written into a
    /// recording rather than trusted.
    pub const CUSTOM_FIRST: u32 = 5;

    /// How many actions of its own a project may declare.
    ///
    /// Sixteen, which takes bits 5 to 20 and leaves eleven spare. A cap at all
    /// because the bitfield is what a recording stores: a project that wanted a
    /// hundred verbs needs a different shape, and finding that out from a
    /// diagnostic beats finding it out from inputs that silently stop
    /// recording.
    pub const MAX_CUSTOM: usize = 16;

    /// The bit a project's `index`-th declared action takes.
    ///
    /// `None` past the cap, so a caller cannot produce a bit outside the
    /// field by arithmetic.
    pub const fn custom(index: usize) -> Option<u32> {
        match index < MAX_CUSTOM {
            true => Some(1 << (CUSTOM_FIRST as usize + index)),
            false => None,
        }
    }
}

/// The date a run is told it is when nobody tells it otherwise.
///
/// A headless run, a replay of a log written before dates existed, and every
/// fixture. Fixed rather than today's, because a run whose output moved with
/// the calendar would be a fixture nobody could check twice.
pub const FIXED_DATE: &str = "2000-01-01";

/// Which kind of device last produced input.
///
/// In the input frame rather than out of band, because what a game *does* with
/// it is scene state: a prompt reading "Space" or showing a pad's South button
/// is text in a `Label`, and text in a `Label` is hashed. A device read from
/// beside the frame would make that text depend on something a recording does
/// not carry, and a replay would diverge the first time somebody picked up a
/// controller.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// A key. The default, and what every log recorded before this existed
    /// means — see [`PlayerInput::hash_state`].
    #[default]
    Keyboard,
    /// The mouse: a move, a button or a wheel.
    Mouse,
    /// A gamepad: a button or a stick past its dead zone.
    Pad,
}

impl Device {
    /// The name a script reads and a log writes.
    pub fn name(self) -> &'static str {
        match self {
            Device::Keyboard => "keyboard",
            Device::Mouse => "mouse",
            Device::Pad => "pad",
        }
    }

    /// Parse the name a log wrote.
    pub fn parse(name: &str) -> Option<Device> {
        Some(match name {
            "keyboard" => Device::Keyboard,
            "mouse" => Device::Mouse,
            "pad" => Device::Pad,
            _ => return None,
        })
    }
}

/// The bit an action the engine owns sets, by name.
///
/// The five built-ins and nothing else. A project's own actions are resolved
/// against the list it declared, which the engine does not know here — see
/// [`action_button`].
pub fn builtin_button(name: &str) -> Option<u32> {
    Some(match name {
        "fire" => buttons::FIRE,
        "alt" => buttons::ALT,
        "dash" => buttons::DASH,
        "use" => buttons::USE,
        "pause" => buttons::PAUSE,
        _ => return None,
    })
}

/// The bit an action sets, built-in or declared by the project.
///
/// `declared` is the project's own actions in declaration order, and position
/// in that list is what picks the bit. **The one place that mapping is made**,
/// so the runtime building an input frame and the script reading one cannot
/// disagree about what bit 5 means.
///
/// A built-in wins, which is why declaring one of their names is refused
/// rather than allowed to shadow: a script asking for `fire` must get the
/// engine's bit whatever a project wrote.
pub fn action_button(name: &str, declared: &[String]) -> Option<u32> {
    if let Some(bit) = builtin_button(name) {
        return Some(bit);
    }
    let index = declared.iter().position(|a| a == name)?;
    buttons::custom(index)
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
    /// Where the pointer is, in canvas pixels.
    ///
    /// In the input frame rather than at the render boundary, because a UI
    /// click has to be replayable: the simulation lays the UI out against a
    /// fixed canvas and decides for itself what the pointer was over. Whole
    /// canvas pixels, so the value is exactly representable and writes into a
    /// log without rounding — a UI hit test has no use for a sixteenth of a
    /// pixel.
    pub pointer: Vec2Fx,
    /// Which kind of device last produced input.
    ///
    /// For prompts that match the hand on the device. See [`Device`] for why it
    /// travels in the frame.
    pub device: Device,
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
    ///
    /// The device contributes **nothing when it is the default**, which is what
    /// keeps every run recorded before it existed hashing exactly as it did. A
    /// log with no device column reads as [`Device::Keyboard`], that is what
    /// those runs were, and a value equal to the default adding nothing is the
    /// same rule an absent node property follows.
    ///
    /// It is hashed when it is anything else, and it has to be: a prompt that
    /// says "Space" under a keyboard and shows a pad glyph under a controller
    /// is different text in a `Label`, and that text is hashed. A device the
    /// hash could not see would let a replay diverge the moment somebody
    /// picked up a pad.
    ///
    /// Each device has one encoding and no two share one, so nothing is
    /// conflated: keyboard is the empty encoding, and the others name
    /// themselves.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.u64(self.buttons as u64)
            .vec2(self.move_dir)
            .angle(self.aim)
            .vec2(self.pointer);
        if self.device != Device::default() {
            h.tag("device").str(self.device.name());
        }
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
    /// The hash of the suspended run this session was resumed from.
    ///
    /// `None` for a session that started a run from its seed, which is every
    /// log recorded before suspending existed and every log recorded since by
    /// a session that did not resume.
    ///
    /// A resumed session's frames are not a run — they are the second half of
    /// one, and replaying them from a fresh scene would reproduce something
    /// nobody played. So the log names the save, by the state hash the save
    /// restores to, and a replay of such a log is refused unless it is given
    /// that save and the hashes agree.
    pub resumed: Option<StateHash>,
    /// The tick the first recorded frame belongs to.
    ///
    /// Zero for a run that started from its seed. For a resumed one it is the
    /// tick the save stopped at, so a tick number in this file means the same
    /// thing a tick number in a probe or a divergence report means.
    pub from_tick: u64,
    /// The date the recorded session began, as `YYYY-MM-DD` in UTC.
    ///
    /// `app.today()` is what a daily run is built on: one run a day, the same
    /// seed for every player, which an engine where two players on one seed
    /// really do get one dungeon is unusually well placed to offer. A title
    /// screen offering it writes the date into a label and picks a seed from
    /// it, and both of those are hashed — so the date has to be part of the
    /// recording rather than read from the clock at replay time, or every
    /// recording of a title screen would expire overnight.
    ///
    /// `None` for a log recorded before this existed, and for one recorded by
    /// a run that was never told. Such a log replays on [`FIXED_DATE`], which
    /// is what it was told at the time.
    ///
    /// The header line is omitted when it is absent, so those logs are
    /// byte-for-byte what they always were.
    pub date: Option<String>,
    /// The actions the project declared of its own, in the order it declared
    /// them.
    ///
    /// A project may name verbs the engine does not have — an undo, a screen
    /// key — and each takes a bit in [`PlayerInput::buttons`] from
    /// [`buttons::CUSTOM_FIRST`] upward, by position in this list. A recording
    /// stores raw bits, so that position *is* part of what the recording
    /// means: append an action and every older log still reads correctly;
    /// reorder or remove one and bit 5 now names something else.
    ///
    /// So the list travels with the log and a replay checks it. A disagreement
    /// is `DIM0703`, the same refusal a log recorded against another scene
    /// gets, rather than a run that replays wrongly and says nothing. A log
    /// recorded before the project declared anything has none, and its custom
    /// actions read as not held — which is what they were.
    ///
    /// Empty for every log this engine has ever written until now, and the
    /// header line is omitted when it is empty, so those logs are byte-for-byte
    /// what they always were.
    pub actions: Vec<String>,
    /// One frame per tick, from [`InputLog::from_tick`].
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
            resumed: None,
            from_tick: 0,
            player_count,
            date: None,
            actions: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// The same, recording a project's own declared actions.
    pub fn with_actions(mut self, actions: Vec<String>) -> InputLog {
        self.actions = actions;
        self
    }

    /// The same, recording the day the session began.
    pub fn with_date(mut self, date: impl Into<String>) -> InputLog {
        self.date = Some(date.into());
        self
    }

    /// The day this log should be replayed as, whatever today is.
    ///
    /// [`FIXED_DATE`] for a log that carries none, because that is what a run
    /// recorded before dates existed was told.
    pub fn date_or_fixed(&self) -> &str {
        self.date.as_deref().unwrap_or(FIXED_DATE)
    }

    /// Whether this log can be read against the actions a project now has.
    ///
    /// A **prefix**, not equality, because that is the rule stated exactly:
    /// every action keeps the bit its position gave it, so appending one leaves
    /// every older recording readable — the new bit is simply zero in it, which
    /// is what not-held means. Reordering or removing one moves a bit to
    /// another verb, and a recording stores raw bits.
    ///
    /// A log with no list is the empty prefix and so is accepted by any
    /// project: it was recorded before anything was declared, and all its
    /// custom bits are zero.
    pub fn actions_agree(&self, declared: &[String]) -> bool {
        declared.starts_with(&self.actions)
    }

    /// Append a frame.
    pub fn push(&mut self, frame: InputFrame) {
        self.frames.push(frame);
    }

    /// The frame for a tick, or idle input outside the log.
    ///
    /// Running past the end is normal: a replay may be asked for more ticks
    /// than were recorded, and idle input is the honest answer. A tick before
    /// [`InputLog::from_tick`] gets the same answer, which only arises if a
    /// caller replays a resumed log from the wrong place — and idle input that
    /// diverges immediately is better than silently reusing the first frame.
    pub fn frame(&self, tick: u64) -> InputFrame {
        tick.checked_sub(self.from_tick)
            .and_then(|i| self.frames.get(i as usize))
            .cloned()
            .unwrap_or_else(|| InputFrame::idle(self.player_count))
    }

    /// The tick one past the last recorded frame.
    pub fn end_tick(&self) -> u64 {
        self.from_tick + self.frames.len() as u64
    }

    /// Record that this session continued a suspended run.
    ///
    /// The hash is the state the save restores to, which is what makes the log
    /// checkable against it: a save that has been replaced since gives a
    /// different hash, and the replay refuses rather than reproducing a run
    /// nobody played.
    pub fn resumed_from(&mut self, state: StateHash, tick: u64, seed: u64) {
        self.resumed = Some(state);
        self.from_tick = tick;
        self.seed = seed;
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
        // Written only when there is one, so a log from a session that started
        // its own run is byte-for-byte what it always was.
        if let Some(h) = self.resumed {
            let _ = writeln!(out, "resumed {h}");
            let _ = writeln!(out, "from_tick {}", self.from_tick);
        }
        let _ = writeln!(out, "players {}", self.player_count);
        // Omitted when absent, for the same reason as `resumed`: a log from a
        // session that was never told the date is what it always was.
        if let Some(date) = &self.date {
            let _ = writeln!(out, "date {date}");
        }
        // Omitted when empty, so every log written before a project declared
        // an action of its own is byte-for-byte what it was.
        if !self.actions.is_empty() {
            let _ = writeln!(out, "actions {}", self.actions.join(" "));
        }
        let _ = writeln!(
            out,
            "# tick  then buttons move_x move_y aim pointer_x pointer_y, per player"
        );
        for (offset, frame) in self.frames.iter().enumerate() {
            let tick = self.from_tick + offset as u64;
            let _ = write!(out, "{tick}");
            for i in 0..self.player_count {
                let p = frame.player(i);
                let _ = write!(
                    out,
                    " {:04x} {} {} {} {} {}",
                    p.buttons,
                    p.move_dir.x.to_exact_string(),
                    p.move_dir.y.to_exact_string(),
                    p.aim.to_degrees_string(),
                    p.pointer.x.to_exact_string(),
                    p.pointer.y.to_exact_string()
                );
                // The device, and only when it is not the default. Two
                // reasons, and the second is the load-bearing one.
                //
                // Omitting the default keeps every keyboard frame — which is
                // every frame in every log written until now — byte-for-byte
                // what it was.
                //
                // The colon is what makes an *optional* per-player column
                // safe. Columns are positional and interleaved per player, so
                // a bare trailing field would be read as the next player's
                // buttons in a two-player log. With a sigil the parser can
                // look at the next token and know whose it is.
                if p.device != Device::default() {
                    let _ = write!(out, " :{}", p.device.name());
                }
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
        let mut resumed = None;
        let mut from_tick = 0u64;
        let mut player_count = 1usize;
        let mut date: Option<String> = None;
        let mut actions: Vec<String> = Vec::new();
        let mut frames = Vec::new();
        let mut saw_tag = false;

        for (number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace().peekable();
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
                "resumed" => {
                    resumed = Some(
                        parts
                            .next()
                            .and_then(StateHash::from_hex)
                            .ok_or_else(|| LogError::Malformed(number + 1, raw.to_string()))?,
                    );
                }
                "from_tick" => from_tick = parse_field(parts.next(), number)?,
                "players" => player_count = parse_field(parts.next(), number)?,
                "date" => date = parts.next().map(str::to_string),
                "actions" => actions = parts.map(str::to_string).collect(),
                _ => {
                    // A tick line. The tick number is positional and checked,
                    // so a log with a missing line fails loudly instead of
                    // silently shifting every input by one tick.
                    let tick: usize = head
                        .parse()
                        .map_err(|_| LogError::Malformed(number + 1, raw.to_string()))?;
                    let expected = from_tick as usize + frames.len();
                    if tick != expected {
                        return Err(LogError::OutOfOrder {
                            line: number + 1,
                            expected,
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
                        // The pointer arrived after the first logs were
                        // written, so a line without it is a log from before
                        // there was a pointer — which is idle, not malformed.
                        // Refusing those would strand every recording anyone
                        // had already made.
                        let px = optional_fx(parts.next(), number)?;
                        let py = optional_fx(parts.next(), number)?;
                        // `:pad`, if this player's frame carries one. Peeked
                        // rather than taken, because a column with no sigil is
                        // the next player's buttons — see `to_text`.
                        let device = match parts.peek().and_then(|t| t.strip_prefix(':')) {
                            Some(name) => {
                                let device = Device::parse(name).ok_or_else(|| {
                                    LogError::Malformed(number + 1, raw.to_string())
                                })?;
                                parts.next();
                                device
                            }
                            None => Device::default(),
                        };
                        players.push(PlayerInput {
                            buttons,
                            move_dir: Vec2Fx::new(x, y),
                            aim,
                            pointer: Vec2Fx::new(px, py),
                            device,
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
            resumed,
            from_tick,
            player_count,
            date,
            actions,
            frames,
        })
    }
}

fn parse_field<T: std::str::FromStr>(v: Option<&str>, line: usize) -> Result<T, LogError> {
    v.and_then(|s| s.parse().ok())
        .ok_or_else(|| LogError::Malformed(line + 1, v.unwrap_or("").to_string()))
}

/// A field that older logs do not have. Absent reads as zero; present but
/// unreadable is still an error, because that is a corrupt log rather than an
/// old one.
fn optional_fx(v: Option<&str>, line: usize) -> Result<Fx, LogError> {
    match v {
        None => Ok(Fx::ZERO),
        Some(_) => parse_fx(v, line),
    }
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
