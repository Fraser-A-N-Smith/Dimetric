//! The one suspended run, on disk beside the profile.
//!
//! [`crate::savefile`] is the *format*: a canonical `scene.dim` plus a
//! `state.toml`, versioned, and tested to load back to an identical state hash.
//! This is the *policy* on top of it, and the policy is what a player sees:
//!
//! - **One slot.** A game with several save files is a game where a player can
//!   keep the good run and reroll the bad one, which is a different design from
//!   the one this was asked for. One directory, one run.
//! - **Consumed on resume.** Suspending writes it, resuming deletes it. So a
//!   run cannot be suspended, played on, lost, and then resumed from the
//!   version that was still alive.
//! - **Beside the profile, never inside the game.** `profile.toml` already
//!   lives in a directory the player owns and the runtime writes to. A
//!   single-file game's `Project::writable()` is false — there is nowhere
//!   inside an executable to put a save, and nothing should try — so this is
//!   keyed on the same root the profile is, which for a packaged game is the
//!   directory the executable sits in.
//! - **Deliberately not a function that also touches the profile.** The
//!   discipline [`crate::savefile`] and [`crate::profile_store`] keep is that
//!   nothing reaches both a run and a profile, because the one thing that must
//!   never happen is a profile finding its way into the hash. That holds here:
//!   this module knows about runs, and the session saves the profile itself.
//!
//! # Why the write is in two renames rather than two files
//!
//! A save is two files, so writing them in place means a crash between them
//! leaves half a save — a `scene.dim` from the new run and a `state.toml` from
//! the old, which is a state that never existed and would load without
//! complaint.
//!
//! So a new save is built in a scratch directory and moved into place by
//! rename, which is atomic. The previous save is renamed aside first rather
//! than deleted, so the window in which neither exists is one syscall wide and
//! survivable: [`read`] falls back to the set-aside copy, which is the previous
//! run rather than nothing. Strays from an interrupted write are cleaned up by
//! the next one.

use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics, StateHash};
use dimetric_scene::KindRegistry;
use dimetric_sim::SimState;

use crate::savefile::{self, SaveFile};

/// The directory a suspended run lives in, under the save root.
pub const SUSPENDED_DIR: &str = "suspended";
/// Where a new save is assembled before it is moved into place.
const PENDING_DIR: &str = "suspended.pending";
/// Where the previous save waits while a new one is moved into place.
const PREVIOUS_DIR: &str = "suspended.previous";

/// Where the suspended run lives under `root`.
pub fn suspended_dir(root: &Path) -> PathBuf {
    root.join(SUSPENDED_DIR)
}

/// What is in the slot.
#[derive(Clone, Debug)]
pub enum Slot {
    /// Nothing has been suspended.
    Empty,
    /// Something is there and this build cannot use it.
    ///
    /// The game is told there is no suspended run, because a Continue row that
    /// fails when pressed is worse than one that was never offered. The
    /// diagnostic says why, so the player is not left guessing either.
    Stale(Diagnostic),
    /// A run this build can continue.
    Ready {
        /// The project-relative scene it was on.
        scene: String,
        /// The tick it stopped at.
        tick: u64,
        /// The run's seed.
        seed: u64,
    },
}

impl Slot {
    /// Whether a run is waiting that this build can actually continue.
    ///
    /// This is what `app.suspended()` answers. A stale save is *not* one.
    pub fn ready(&self) -> bool {
        matches!(self, Slot::Ready { .. })
    }
}

/// Look at the slot without loading the run.
///
/// Reads the header only: the scene is not parsed, so an unusable save is
/// reported as unusable rather than as a scene that would not load.
pub fn probe(root: &Path) -> Slot {
    let dir = match existing(root) {
        Some(dir) => dir,
        None => return Slot::Empty,
    };
    let header = match savefile::read_header(&dir) {
        Ok(header) => header,
        Err(d) => return Slot::Stale(d),
    };
    match header.usable() {
        Err(d) => Slot::Stale(d),
        Ok(()) => Slot::Ready {
            scene: header.scene.clone(),
            tick: header.tick,
            seed: header.seed,
        },
    }
}

/// Write `state` as the suspended run, replacing whatever was there.
///
/// Returns the hash of the state that was written, which is what a recording
/// of the resumed session names so the log can be replayed against this save.
pub fn write(
    root: &Path,
    state: &SimState,
    scene_path: &str,
    registry: &KindRegistry,
) -> Result<StateHash, Diagnostic> {
    let pending = root.join(PENDING_DIR);
    let previous = root.join(PREVIOUS_DIR);
    let final_dir = suspended_dir(root);

    // Strays from an interrupted write. Removed before anything is built, so a
    // half-written pending directory is never mistaken for a new save.
    remove(&pending)?;
    remove(&previous)?;

    savefile::save(&pending, state, scene_path, registry)?;

    if final_dir.exists() {
        rename(&final_dir, &previous)?;
    }
    rename(&pending, &final_dir)?;
    remove(&previous)?;
    Ok(state.hash())
}

/// Read the suspended run back, leaving it in place.
///
/// Deleting it is [`discard`], and the session does that only once the state
/// has actually been installed — a resume that failed half way should leave the
/// save where it was rather than consume it.
pub fn read(
    root: &Path,
    registry: &KindRegistry,
) -> Result<(SimState, SaveFile, Diagnostics), Diagnostic> {
    let dir = existing(root).ok_or_else(|| {
        Diagnostic::new(
            Code::SUSPEND_REFUSED,
            format!("there is no suspended run under {}", root.display()),
        )
    })?;
    let header = savefile::read_header(&dir)?;
    let (state, diagnostics) = savefile::load(&dir, registry)?;
    Ok((state, header, diagnostics))
}

/// Throw the suspended run away.
///
/// Succeeds when there was nothing there: a player starting a new run does not
/// care whether one was waiting, and a game that had to check first would check
/// wrong one day.
pub fn discard(root: &Path) -> Result<(), Diagnostic> {
    remove(&suspended_dir(root))?;
    remove(&root.join(PREVIOUS_DIR))?;
    remove(&root.join(PENDING_DIR))
}

/// The directory holding the current save, allowing for an interrupted write.
///
/// A crash in the one-syscall window between moving the old save aside and
/// moving the new one in leaves only `suspended.previous`. That is the previous
/// run, intact, which is exactly what should be continued — so it is used
/// rather than reported as nothing.
fn existing(root: &Path) -> Option<PathBuf> {
    let dir = suspended_dir(root);
    if dir.join(savefile::STATE_FILE).is_file() {
        return Some(dir);
    }
    let previous = root.join(PREVIOUS_DIR);
    if previous.join(savefile::STATE_FILE).is_file() {
        return Some(previous);
    }
    None
}

fn remove(dir: &Path) -> Result<(), Diagnostic> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!("removing {}: {e}", dir.display()),
        )),
    }
}

fn rename(from: &Path, to: &Path) -> Result<(), Diagnostic> {
    std::fs::rename(from, to).map_err(|e| {
        Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!("moving {} to {}: {e}", from.display(), to.display()),
        )
    })
}

/// Where a recorded session keeps the save it resumed from: beside the log.
///
/// A recording of a resumed run is only replayable with the run it continued,
/// and the slot that run came out of is emptied by the resume — so the two
/// travel together, as `run.input` and `run.input.save/`. Here rather than in
/// the runtime or the CLI because both of them need it and neither depends on
/// the other.
pub fn sidecar_save(log: &Path) -> PathBuf {
    let mut name = log.as_os_str().to_os_string();
    name.push(".save");
    PathBuf::from(name)
}
