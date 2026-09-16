//! Honouring a script's scene-load request, between ticks.
//!
//! The simulation cannot do this itself and should not: it has no filesystem,
//! and a tick that swapped its own tree would stop being a pure function of
//! the state it started from (I8). So a script sets a request in state, the
//! tick finishes over the tree it started with, and this runs afterwards.
//!
//! It lives in the host because the host is what owns a `Project` — the
//! registry, the prefabs, the scripts and the disk. Every caller that drives a
//! simulation forward calls it: the player's session, `dim run`, and the
//! replay harness. One function rather than three, because three would be
//! three chances for a replay to load something the game did not.

use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_sim::Sim;

use crate::Project;

/// Apply a pending scene-load request, if there is one.
///
/// Returns the path that was loaded, so a caller can tell the difference
/// between "nothing was asked for" and "a floor changed" — the player's
/// session has to rebuild its atlas when it does.
///
/// A load that fails leaves the game running on the scene it already had. The
/// alternative is a session with no tree at all, which cannot draw, cannot
/// tick, and cannot tell the player what went wrong.
pub fn apply_pending_load(
    project: &mut Project,
    sim: &mut Sim,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    let request = sim.take_load_request()?;

    if let Err(d) = project.load_scene(&request.path) {
        diagnostics.extend(d);
        diagnostics.push(Diagnostic::new(
            Code::ASSET_MISSING,
            format!(
                "staying on the current scene: {:?} could not be loaded",
                request.path
            ),
        ));
        return None;
    }

    let (scene, diags) = match project.runtime_scene() {
        Ok(pair) => pair,
        Err(d) => {
            diagnostics.extend(d);
            return None;
        }
    };
    diagnostics.extend(diags);

    // Scripts are reloaded because the new scene may reference ones the old
    // one never mentioned, and because `require`'s module cache is dropped on
    // any reload — a module holding the previous floor's constants would be a
    // hot reload that appears to work and does nothing.
    diagnostics.extend(project.load_scripts());

    sim.swap_scene(scene, request.carry);
    Some(request.path)
}
