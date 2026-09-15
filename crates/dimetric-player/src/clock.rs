//! Turning wall-clock time into ticks.
//!
//! The simulation advances in fixed steps and nothing else will do: a tick
//! whose length depended on how long the last frame took would make the run
//! unreproducible, which is the one thing this engine is for.
//!
//! So the clock accumulates real time and spends it in whole ticks. What is
//! left over is the interpolation alpha the renderer draws with, and it never
//! reaches the simulation.

use std::time::Duration;

/// How many ticks one frame may run before the rest are dropped.
///
/// Without a cap, a frame that took a second — a breakpoint, a laptop lid, a
/// stalled driver — owes sixty ticks, and simulating them takes longer than a
/// frame, which owes more still. The game never catches up and never draws
/// again. Dropping time is visible; a spiral is terminal.
const MAX_CATCH_UP: u32 = 8;

/// A fixed-timestep accumulator.
#[derive(Clone, Debug)]
pub struct Clock {
    rate: u32,
    /// Time owed but not yet spent, in nanoseconds.
    owed: u64,
    /// Ticks dropped to stay out of a spiral, over the clock's life.
    dropped: u64,
}

impl Clock {
    /// A clock for a tick rate in hertz.
    pub fn new(rate: u32) -> Clock {
        Clock {
            rate: rate.max(1),
            owed: 0,
            dropped: 0,
        }
    }

    /// The length of one tick.
    pub fn step(&self) -> Duration {
        Duration::from_nanos(self.step_nanos())
    }

    fn step_nanos(&self) -> u64 {
        1_000_000_000 / u64::from(self.rate)
    }

    /// Spend `elapsed` and report how many ticks it bought.
    ///
    /// Whatever is left is kept, so a frame that lands between two ticks does
    /// not lose the remainder — sixty 16.7ms frames buy sixty ticks, not
    /// fifty-eight.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.owed = self
            .owed
            .saturating_add(elapsed.as_nanos().min(u128::from(u64::MAX)) as u64);
        let step = self.step_nanos();
        let owed_ticks = self.owed / step;
        let ticks = owed_ticks.min(u64::from(MAX_CATCH_UP)) as u32;
        if owed_ticks > u64::from(ticks) {
            self.dropped += owed_ticks - u64::from(ticks);
            // Drop the excess rather than carrying it: carrying it is the
            // spiral.
            self.owed = 0;
        } else {
            self.owed -= u64::from(ticks) * step;
        }
        ticks
    }

    /// How far the next tick is from having arrived, in `0.0..1.0`.
    ///
    /// The renderer's interpolation alpha. Below the render boundary, so a
    /// float is fine here (I3).
    pub fn alpha(&self) -> f32 {
        self.owed as f32 / self.step_nanos() as f32
    }

    /// Ticks this clock has dropped rather than spiral.
    ///
    /// Worth reporting: a game that drops ticks is a game that is too slow, and
    /// silently running in slow motion is how that goes unnoticed.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
