//! Run modes and the fixed-timestep accumulator.

use serde::{Deserialize, Serialize};

/// What the host is doing with the project.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Editing. Commands apply, nothing ticks.
    #[default]
    Edit,
    /// Playing. Input comes from devices and is recorded.
    Play,
    /// Replaying a recorded run from a seed and an input log.
    Replay,
    /// Running without a window, for CI and for agents.
    Headless,
}

impl RunMode {
    /// Whether the simulation advances in this mode.
    pub fn ticks(self) -> bool {
        !matches!(self, RunMode::Edit)
    }

    /// Whether scene-editing commands are accepted.
    pub fn accepts_edits(self) -> bool {
        matches!(self, RunMode::Edit)
    }

    /// The mode's name in JSON and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            RunMode::Edit => "edit",
            RunMode::Play => "play",
            RunMode::Replay => "replay",
            RunMode::Headless => "headless",
        }
    }
}

/// Decides how many fixed ticks to run for a given amount of elapsed real time.
///
/// This is the only place in the engine that touches a wall clock, and it lives
/// here rather than in `dimetric-sim` precisely so that the simulation never
/// sees one (I5). What crosses the boundary is a tick count and nothing else.
#[derive(Clone, Copy, Debug)]
pub struct Accumulator {
    /// Ticks per second.
    pub tick_rate: u32,
    /// Unspent time, in seconds.
    pending: f64,
    /// Ticks to run in one update before giving up and dropping time.
    pub max_catch_up: u32,
}

impl Accumulator {
    /// A 60 Hz accumulator.
    pub fn new(tick_rate: u32) -> Accumulator {
        Accumulator {
            tick_rate,
            pending: 0.0,
            max_catch_up: 8,
        }
    }

    /// Feed in elapsed real time and get back how many ticks to run.
    ///
    /// Caps at `max_catch_up`. Without a cap, one long stall — a breakpoint, a
    /// laptop lid — produces a burst of ticks that takes longer to simulate
    /// than it did to accumulate, and the game never catches up.
    pub fn advance(&mut self, elapsed_seconds: f64) -> u32 {
        // I3-exempt: the accumulator is host-side and never touches sim state.
        self.pending += elapsed_seconds.clamp(0.0, 1.0);
        let step = 1.0 / self.tick_rate as f64;
        let mut ticks = 0;
        while self.pending >= step && ticks < self.max_catch_up {
            self.pending -= step;
            ticks += 1;
        }
        if ticks == self.max_catch_up {
            self.pending = 0.0;
        }
        ticks
    }

    /// How far between ticks the renderer should interpolate, in `0.0 ..< 1.0`.
    ///
    /// Render-side only: this is read to smooth a 60 Hz simulation onto a
    /// 144 Hz display and never written back (I7).
    pub fn interpolation(&self) -> f32 {
        // I3-exempt: presentation value, never part of simulation state.
        (self.pending * self.tick_rate as f64) as f32
    }
}
