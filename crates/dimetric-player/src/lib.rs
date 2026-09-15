//! The game runtime.
//!
//! What a player does is run the same simulation the CLI runs, at the speed a
//! clock says, over input a person is producing live, and draw the result. Only
//! the last of those needs a window.
//!
//! So the window is not here. [`Clock`] decides how many ticks a frame owes and
//! how far between two of them the drawn frame sits; [`Bindings`] turns held
//! keys into a [`PlayerInput`]; [`Session`] owns the project, the simulation and
//! the atlas, and hands back a frame to draw. All three are testable on a
//! machine with no display, which is where their behaviour is pinned. `dim-play`
//! is the winit and wgpu shell over them, behind the `gui` feature.
//!
//! A session records what it was given. A run someone played is an input log
//! like any other, so a bug found by playing reproduces under `dim replay`
//! rather than as a description of what happened.

#![warn(missing_docs)]

pub mod bindings;
pub mod clock;
pub mod session;

pub use bindings::{Action, Bindings, Held};
pub use clock::Clock;
pub use session::{Session, SessionConfig};
