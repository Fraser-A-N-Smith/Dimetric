//! The editor: view state, panel models, and the actions a client dispatches.
//!
//! # What is here
//!
//! Everything about an editor that can be wrong without a window being open.
//! The scene tree, the inspector, the asset browser, the console, play-in-editor
//! and the replay scrubber are all models over engine state, and a client draws
//! them. None of it depends on a GUI toolkit.
//!
//! That split is not tidiness for its own sake. §16 names editor scope as the
//! highest risk in the project, and the mitigation it gives is thin-client
//! discipline. A client that holds no logic cannot grow any.
//!
//! # The contract the client has to keep
//!
//! The editor holds no authoritative state. Every widget reads engine state and
//! emits an [`Action`], which [`Editor::dispatch`] turns into
//! [`Command`](dimetric_host::Command)s; none of them touches the scene
//! directly (invariant I1). That is checkable rather than aspirational: the
//! acceptance test drives a scripted editing session with no GUI at all and
//! compares the commands it produced against the diff it made.
//!
//! The other rule is that opening and closing a scene produces **zero** diff in
//! the `.dim`. Everything that changes when you merely look at a scene — where
//! the camera is, what is selected, which branches are folded — belongs in the
//! [`Sidecar`], which is committed so a team shares it.

#![warn(missing_docs)]

pub mod action;
pub mod console;
pub mod editor;
pub mod panels;
pub mod playback;
pub mod sidecar;

pub use action::Action;
pub use console::Console;
pub use editor::{world_position, Editor, Outcome};
pub use playback::{Mode, Playback};
pub use sidecar::{Sidecar, SIDECAR_EXTENSION};
