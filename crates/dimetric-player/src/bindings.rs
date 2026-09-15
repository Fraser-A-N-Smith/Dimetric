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
        PlayerInput {
            buttons,
            move_dir: if raw.is_zero() { raw } else { raw.normalized() },
            aim: self.aim,
        }
    }
}
