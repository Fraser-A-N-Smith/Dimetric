//! Gamepads, and the boundary they have to cross.
//!
//! A pad is the hardest input device to let into a deterministic engine, and
//! not for the reason people expect. The problem is not that it reports
//! floats — a keyboard's `true` becomes a fixed-point 1 without difficulty.
//! It is that a stick *never stops moving*. Resting a thumb on one produces a
//! slightly different reading every poll, so a naive mapping writes a new line
//! into the input log sixty times a second while the player sits still, and
//! two machines with slightly different drivers disagree from the first tick.
//!
//! So a stick is quantised — see [`crate::bindings::quantize_stick`] — into
//! one of a small number of magnitudes on an angle from the engine's own
//! tables. What crosses into a tick is exactly representable, identical
//! everywhere, and stable while the player holds still.
//!
//! Hotplug is handled the same way as everything else here: connections and
//! disconnections are events read at the boundary, between ticks, and a pad
//! that vanishes mid-game releases what it was holding rather than leaving a
//! button down for ever.

use dimetric_core::Vec2Fx;

use crate::bindings::Held;
#[cfg(feature = "pad")]
use crate::bindings::{quantize_stick, Action};

/// Every gamepad, polled as one player.
///
/// One player for now: local multiplayer wants a pad-to-player mapping and a
/// way to say which is which, and inventing that before there is a game that
/// needs it is how an input system ends up with an assignment scheme nobody
/// asked for.
pub struct Pads {
    #[cfg(feature = "pad")]
    gilrs: Option<gilrs::Gilrs>,
    /// Names of connected pads, for diagnostics.
    connected: Vec<String>,
}

impl Default for Pads {
    fn default() -> Pads {
        Pads::new()
    }
}

impl Pads {
    /// Open the gamepad subsystem, or carry on without one.
    ///
    /// A machine with no input backend at all — a container, a CI runner — is
    /// not an error. It is a machine with no gamepads.
    pub fn new() -> Pads {
        #[cfg(feature = "pad")]
        {
            let gilrs = gilrs::Gilrs::new().ok();
            let connected = gilrs
                .as_ref()
                .map(|g| g.gamepads().map(|(_, p)| p.name().to_string()).collect())
                .unwrap_or_default();
            Pads { gilrs, connected }
        }
        #[cfg(not(feature = "pad"))]
        Pads {
            connected: Vec::new(),
        }
    }

    /// Names of the pads currently connected.
    pub fn connected(&self) -> &[String] {
        &self.connected
    }

    /// Drain pending events into `held`.
    ///
    /// Called between ticks, never inside one: a pad read half way through a
    /// tick would make the tick depend on when the poll happened.
    #[allow(unused_variables)]
    pub fn poll(&mut self, held: &mut Held) {
        #[cfg(feature = "pad")]
        {
            let Some(gilrs) = self.gilrs.as_mut() else {
                return;
            };
            while let Some(event) = gilrs.next_event() {
                match event.event {
                    gilrs::EventType::Disconnected => {
                        // Everything this pad was holding, released. A button
                        // left down by a yanked cable is held for ever.
                        held.push_stick(None);
                        for action in BUTTONS.iter().map(|(_, a)| *a) {
                            held.set(action, false);
                        }
                    }
                    gilrs::EventType::Connected => {}
                    _ => {}
                }
            }
            self.connected = gilrs
                .gamepads()
                .map(|(_, p)| p.name().to_string())
                .collect();

            // State rather than events for the steady-state reads: what
            // matters at a tick boundary is what is held *now*, and replaying
            // a queue of presses and releases to find out is a longer way to
            // the same answer.
            let Some((_, pad)) = gilrs.gamepads().next() else {
                held.push_stick(None);
                return;
            };
            for (button, action) in BUTTONS {
                held.set(*action, pad.is_pressed(*button));
            }
            let x = pad.value(gilrs::Axis::LeftStickX);
            // A pad's Y axis points up and the engine's points down.
            let y = -pad.value(gilrs::Axis::LeftStickY);
            let stick = quantize_stick(x, y);
            held.push_stick(if stick.is_zero() { None } else { Some(stick) });

            // The right stick aims, when it is pushed far enough to mean it.
            let ax = pad.value(gilrs::Axis::RightStickX);
            let ay = -pad.value(gilrs::Axis::RightStickY);
            let aim = quantize_stick(ax, ay);
            if !aim.is_zero() {
                held.aim_at(dimetric_core::Angle::from_vector(aim.x, aim.y));
            }
        }
    }
}

#[cfg(feature = "pad")]
const BUTTONS: &[(gilrs::Button, Action)] = &[
    (gilrs::Button::South, Action::Fire),
    (gilrs::Button::West, Action::Alt),
    (gilrs::Button::RightTrigger, Action::Fire),
    (gilrs::Button::East, Action::Dash),
    (gilrs::Button::North, Action::Use),
    (gilrs::Button::Start, Action::Pause),
    (gilrs::Button::DPadUp, Action::Up),
    (gilrs::Button::DPadDown, Action::Down),
    (gilrs::Button::DPadLeft, Action::Left),
    (gilrs::Button::DPadRight, Action::Right),
];

/// Where a window pixel falls on the UI canvas.
///
/// The one place the window's size is allowed to matter, and it is divided out
/// here: what crosses into the tick is a canvas pixel, so a click lands on the
/// same button whatever size the window is. Whole pixels, because a UI hit
/// test has no use for a fraction of one and a fraction would not write into a
/// log exactly.
pub fn window_to_canvas(
    window: (f64, f64),
    window_size: (u32, u32),
    canvas: dimetric_scene::ui::Canvas,
) -> Vec2Fx {
    // I3-exempt: the device boundary, which is exactly where a float is
    // supposed to become a fixed-point value.
    let (w, h) = (window_size.0.max(1) as f64, window_size.1.max(1) as f64);
    let x = (window.0 / w * canvas.width as f64).floor() as i32;
    let y = (window.1 / h * canvas.height as f64).floor() as i32;
    Vec2Fx::from_ints(
        x.clamp(0, canvas.width.saturating_sub(1)),
        y.clamp(0, canvas.height.saturating_sub(1)),
    )
}
