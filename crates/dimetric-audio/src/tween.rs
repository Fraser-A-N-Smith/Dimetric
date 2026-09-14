//! Tweened values, for fades and room transitions.
//!
//! Presentation-side, so seconds and floats are fine here. Nothing in this
//! module may be reached from a tick: a fade that the simulation could see
//! would make gameplay depend on how long a frame took.

/// How a tween gets from one value to the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Curve {
    /// Constant rate.
    #[default]
    Linear,
    /// Slow at the start.
    EaseIn,
    /// Slow at the end. The one to reach for on a fade-out: it holds the sound
    /// up and then drops it, rather than the reverse.
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
}

impl Curve {
    /// Shape a `0.0..=1.0` progress value.
    pub fn shape(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::EaseIn => t * t,
            Curve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Curve::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - 2.0 * (1.0 - t) * (1.0 - t)
                }
            }
        }
    }
}

/// A value moving from one number to another over a span of seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    elapsed: f32,
    duration: f32,
    curve: Curve,
}

impl Tween {
    /// A tween from `from` to `to` over `seconds`.
    ///
    /// A duration of zero lands on `to` immediately, which is what makes a
    /// "fade" of zero seconds a cut rather than a division by zero.
    pub fn new(from: f32, to: f32, seconds: f32, curve: Curve) -> Tween {
        Tween {
            from,
            to,
            elapsed: 0.0,
            duration: seconds.max(0.0),
            curve,
        }
    }

    /// A value that is not going anywhere.
    pub fn held(value: f32) -> Tween {
        Tween::new(value, value, 0.0, Curve::Linear)
    }

    /// Move time forward.
    pub fn advance(&mut self, seconds: f32) {
        self.elapsed = (self.elapsed + seconds.max(0.0)).min(self.duration);
    }

    /// The value now.
    pub fn value(&self) -> f32 {
        if self.duration <= 0.0 {
            return self.to;
        }
        let t = self.curve.shape(self.elapsed / self.duration);
        self.from + (self.to - self.from) * t
    }

    /// Where it is heading.
    pub fn target(&self) -> f32 {
        self.to
    }

    /// Whether it has arrived.
    pub fn is_done(&self) -> bool {
        self.elapsed >= self.duration
    }

    /// Redirect to a new target from wherever it is now.
    ///
    /// Starting the new tween from the current value rather than the old
    /// target is what stops a fade-out interrupted by a fade-in from jumping.
    pub fn retarget(&mut self, to: f32, seconds: f32, curve: Curve) {
        *self = Tween::new(self.value(), to, seconds, curve);
    }
}
