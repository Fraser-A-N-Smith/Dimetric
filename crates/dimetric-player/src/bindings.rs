//! Keys to input.
//!
//! The simulation reads a [`PlayerInput`]: a bitfield, a movement vector and an
//! aim angle. Everything a keyboard or a gamepad does has to become one of
//! those before it crosses into a tick, because inside a tick there is no such
//! thing as a key (I8).
//!
//! Bindings are names rather than key codes, so the mapping is a table a
//! project can print, diff and eventually configure, rather than a match arm.

use std::collections::BTreeSet;

use dimetric_core::{Angle, Fx, Vec2Fx};
use dimetric_sim::input::buttons;
use dimetric_sim::PlayerInput;

/// Something a player can be doing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Action {
    /// Move north.
    Up,
    /// Move south.
    Down,
    /// Move west.
    Left,
    /// Move east.
    Right,
    /// Primary action.
    Fire,
    /// Secondary action.
    Alt,
    /// Dash or dodge.
    Dash,
    /// Interact.
    Use,
    /// Pause. Read outside the simulation.
    Pause,
}

impl Action {
    /// The button bit this action sets, if it is a button rather than a
    /// direction.
    pub fn button(self) -> Option<u32> {
        Some(match self {
            Action::Fire => buttons::FIRE,
            Action::Alt => buttons::ALT,
            Action::Dash => buttons::DASH,
            Action::Use => buttons::USE,
            Action::Pause => buttons::PAUSE,
            _ => return None,
        })
    }

    /// The name used in bindings and in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Action::Up => "up",
            Action::Down => "down",
            Action::Left => "left",
            Action::Right => "right",
            Action::Fire => "fire",
            Action::Alt => "alt",
            Action::Dash => "dash",
            Action::Use => "use",
            Action::Pause => "pause",
        }
    }
}

/// Snap an analogue stick to something a log can hold exactly.
///
/// A stick reports floats, and a float written into simulation state is the
/// easiest way to break a replay (I3). Worse, the raw value jitters: a stick
/// at rest sends slightly different numbers every poll, and every one of them
/// would be a different line in the log.
///
/// So the magnitude is rounded to one of [`STICK_STEPS`] steps and the value
/// rebuilt from the engine's own trig tables. What comes out is exactly
/// representable, identical on every machine, and stable while the player
/// holds still. Below the dead zone it is exactly zero, which is what stops a
/// resting stick writing input for ever.
pub fn quantize_stick(x: f32, y: f32) -> Vec2Fx {
    // I3-exempt: this is the device boundary, and quantising is the whole
    // point of the function. Nothing downstream of it sees a float.
    let magnitude = (x * x + y * y).sqrt();
    if magnitude < STICK_DEAD_ZONE {
        return Vec2Fx::ZERO;
    }
    let clamped = magnitude.min(1.0);
    let step = ((clamped * STICK_STEPS as f32).round() as i32).clamp(1, STICK_STEPS);
    let scale = Fx::from_int(step) / Fx::from_int(STICK_STEPS);

    // The direction goes through the engine's own fixed-point atan2 rather
    // than the platform's, for the same reason everything else does: libm
    // implementations do not agree with each other.
    let angle = Angle::from_vector(Fx::from_f64_lossy(x as f64), Fx::from_f64_lossy(y as f64));
    let (sin, cos) = angle.sin_cos();
    Vec2Fx::new(cos * scale, sin * scale)
}

/// How many magnitude steps a stick is rounded to.
///
/// Sixteen is finer than a player can feel and coarse enough that holding a
/// stick still produces one repeated value rather than a stream of them.
pub const STICK_STEPS: i32 = 16;

/// Below this, a stick reads as centred.
pub const STICK_DEAD_ZONE: f32 = 0.2;

/// Which key does what.
#[derive(Clone, Debug)]
pub struct Bindings {
    pairs: Vec<(String, Action)>,
}

impl Default for Bindings {
    fn default() -> Bindings {
        Bindings::wasd()
    }
}

impl Bindings {
    /// WASD and the arrow keys to move, mouse and space to act.
    ///
    /// Both movement sets at once, because the first thing anyone does with a
    /// new game is try whichever they prefer, and a game that ignores one of
    /// them reads as broken rather than opinionated.
    pub fn wasd() -> Bindings {
        Bindings {
            pairs: vec![
                ("KeyW".into(), Action::Up),
                ("KeyS".into(), Action::Down),
                ("KeyA".into(), Action::Left),
                ("KeyD".into(), Action::Right),
                ("ArrowUp".into(), Action::Up),
                ("ArrowDown".into(), Action::Down),
                ("ArrowLeft".into(), Action::Left),
                ("ArrowRight".into(), Action::Right),
                ("Space".into(), Action::Fire),
                ("Mouse0".into(), Action::Fire),
                ("Mouse1".into(), Action::Alt),
                ("ShiftLeft".into(), Action::Dash),
                ("KeyE".into(), Action::Use),
                ("Escape".into(), Action::Pause),
            ],
        }
    }

    /// What a key does, if anything.
    pub fn action(&self, key: &str) -> Option<Action> {
        self.pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, action)| *action)
    }

    /// Every binding, in declaration order.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, Action)> {
        self.pairs.iter().map(|(k, a)| (k.as_str(), *a))
    }
}

/// The actions currently held.
#[derive(Clone, Debug, Default)]
pub struct Held {
    actions: BTreeSet<Action>,
    aim: Angle,
    pointer: Vec2Fx,
    /// A gamepad stick, when one is being pushed.
    ///
    /// Separate from the key-derived direction rather than folded into it: a
    /// stick is analogue and keys are not, and adding them would let a player
    /// holding both walk at twice the speed.
    stick: Option<Vec2Fx>,
}

impl Held {
    /// Nothing held, aiming east.
    pub fn new() -> Held {
        Held::default()
    }

    /// Record a press or a release.
    pub fn set(&mut self, action: Action, down: bool) {
        if down {
            self.actions.insert(action);
        } else {
            self.actions.remove(&action);
        }
    }

    /// True when an action is held.
    pub fn holds(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }

    /// Point the aim somewhere.
    pub fn aim_at(&mut self, aim: Angle) {
        self.aim = aim;
    }

    /// Put the pointer at a canvas pixel.
    ///
    /// Whole pixels, and the caller does the conversion from window
    /// coordinates: the window is the one thing that must not reach a tick,
    /// so the boundary is where its size gets divided out.
    pub fn point_at(&mut self, canvas_pixel: Vec2Fx) {
        self.pointer = canvas_pixel;
    }

    /// Where the pointer is.
    pub fn pointer(&self) -> Vec2Fx {
        self.pointer
    }

    /// Push the movement stick, or let it go with `None`.
    ///
    /// The value is quantised by [`quantize_stick`] before it gets here.
    pub fn push_stick(&mut self, stick: Option<Vec2Fx>) {
        self.stick = stick;
    }

    /// Release everything. Used when the window loses focus, so a key held at
    /// the moment someone alt-tabs does not stay held forever.
    pub fn release_all(&mut self) {
        self.actions.clear();
    }

    /// The input a tick reads.
    ///
    /// Diagonals are normalised, so walking north-east is not faster than
    /// walking north. `Vec2Fx::normalized` is fixed point, so the value is the
    /// same on every machine and writes exactly into an input log.
    pub fn player_input(&self) -> PlayerInput {
        let mut buttons = 0;
        for action in &self.actions {
            if let Some(bit) = action.button() {
                buttons |= bit;
            }
        }
        let x = i32::from(self.holds(Action::Right)) - i32::from(self.holds(Action::Left));
        // North is negative y, as it is everywhere else in the engine.
        let y = i32::from(self.holds(Action::Down)) - i32::from(self.holds(Action::Up));
        let raw = Vec2Fx::new(Fx::from_int(x), Fx::from_int(y));
        let keys = if raw.is_zero() { raw } else { raw.normalized() };

        // A pushed stick wins over the keys rather than adding to them: a
        // player resting a hand on both should not move at twice the speed,
        // and whichever device they are actually using is the one that is
        // moving.
        let move_dir = match self.stick {
            Some(stick) if !stick.is_zero() => stick,
            _ => keys,
        };

        PlayerInput {
            buttons,
            move_dir,
            aim: self.aim,
            pointer: self.pointer,
        }
    }
}
