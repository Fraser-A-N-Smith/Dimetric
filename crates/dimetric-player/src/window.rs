//! What the window calls itself, and where that comes from.
//!
//! Host presentation, and kept on this side of the line deliberately: a title
//! and an icon reach a window manager and nothing else. No part of the engine
//! below the runtime reads either of them, the simulation never sees them, and
//! renaming a game cannot change what a recorded run replays to — the same
//! line audio is on.
//!
//! The window is created before any of the project has been read and no script
//! can reach it, so neither of these can come from the game at runtime. They
//! come from `project.toml` while a project is being developed, and from the
//! manifest `dim build` writes once it has shipped.

use dimetric_host::settings::Game;

/// The title bar's text when a project does not name itself.
pub const DEFAULT_TITLE: &str = "Dimetric";

/// What this window should be called.
///
/// The manifest wins over `project.toml` for a packaged game, because
/// `dim build --name` can ship a project under a different name from the one
/// it is developed under and the project file is staged beside the manifest
/// either way. A project run with `--project` has no manifest and uses its own
/// settings, so a developer sees the real title too.
pub fn title(manifest: Option<&str>, game: &Game) -> String {
    manifest
        .and_then(dimetric_host::package::game_name)
        .or_else(|| game.name.clone())
        .unwrap_or_else(|| DEFAULT_TITLE.to_string())
}

/// Where this window's icon is, relative to whatever the project is read
/// through.
///
/// `None` means the window manager's default, which is what every game got
/// before there was a way to say otherwise.
pub fn icon_path(manifest: Option<&str>, game: &Game) -> Option<String> {
    manifest
        .and_then(dimetric_host::package::game_icon)
        .or_else(|| game.icon.clone())
}
