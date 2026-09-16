//! Reading and writing a player's profile.
//!
//! Deliberately a separate file from [`crate::savefile`], and a separate file
//! on disk. A run save and a profile are different kinds of thing — one is
//! simulation state and one must never be — and the cheapest way to keep them
//! from mixing is to never have a function that touches both.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic};
use dimetric_scene::Value;
use dimetric_sim::profile::Profile;

/// Where a project's profile lives.
pub const PROFILE_FILE: &str = "profile.toml";

/// The file's shape.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Stored {
    /// Entries, by key.
    #[serde(default)]
    entries: BTreeMap<String, Value>,
}

/// The profile path under a project root.
pub fn profile_path(root: &Path) -> PathBuf {
    root.join(PROFILE_FILE)
}

/// Read a profile, or an empty one when there is none yet.
///
/// A missing file is a new player, not an error. A *corrupt* file is an error:
/// silently starting someone from nothing because their unlocks failed to
/// parse is the worst possible handling of the one file they cannot rebuild.
pub fn load(root: &Path) -> Result<Profile, Diagnostic> {
    let path = profile_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Profile::new()),
        Err(e) => {
            return Err(Diagnostic::new(
                Code::SAVE_UNREADABLE,
                format!("reading {}: {e}", path.display()),
            ))
        }
    };
    let stored: Stored = toml::from_str(&text).map_err(|e| {
        Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!("{} is not a readable profile: {e}", path.display()),
        )
    })?;
    Ok(Profile::from_entries(stored.entries))
}

/// Write a profile out.
pub fn save(root: &Path, profile: &Profile) -> Result<(), Diagnostic> {
    let path = profile_path(root);
    let stored = Stored {
        entries: profile.entries().clone(),
    };
    let text = toml::to_string_pretty(&stored).map_err(|e| {
        Diagnostic::new(Code::SAVE_UNREADABLE, format!("encoding the profile: {e}"))
    })?;
    std::fs::write(&path, text).map_err(|e| {
        Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!("writing {}: {e}", path.display()),
        )
    })
}
