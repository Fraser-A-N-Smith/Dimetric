//! Where the window's title and icon come from, and in what order.
//!
//! Three sources, and the order between them is the whole decision: a packaged
//! game's manifest, then the project's own `project.toml`, then the engine's
//! name. The manifest wins because `dim build --name` can ship a project under
//! a different name from the one it is developed under, and `project.toml` is
//! staged beside the manifest either way — so if the project file won, the
//! flag would do nothing in the only build it was for.

use dimetric_host::settings::Settings;
use dimetric_player::{icon_path, title, DEFAULT_TITLE};

/// A manifest as `dim build` writes one.
fn manifest(extra: &str) -> String {
    format!(
        "format = \"dimetric-game\"\nversion = 1\nname = \"Shipped\"\n\
         engine = \"0.1.0\"\ntarget = \"x86_64-unknown-linux-gnu\"\n\
         scene = \"main.dim\"\nseed = 0\n{extra}"
    )
}

fn declared(toml: &str) -> dimetric_host::settings::Game {
    Settings::parse(toml, "project.toml").0.game
}

#[test]
fn a_project_that_names_itself_is_called_that() {
    let game = declared("[game]\nname = \"Confluence\"\n");
    assert_eq!(title(None, &game), "Confluence");
}

#[test]
fn a_project_that_says_nothing_is_called_dimetric() {
    // What every game got before there was a way to say otherwise, and what
    // one still gets: a missing value falls back rather than failing.
    assert_eq!(title(None, &declared("")), DEFAULT_TITLE);
    assert_eq!(title(None, &declared("")), "Dimetric");
    assert_eq!(icon_path(None, &declared("")), None);
}

#[test]
fn a_packaged_games_manifest_wins_over_its_project_file() {
    let game = declared("[game]\nname = \"Developed\"\n");
    assert_eq!(title(Some(&manifest("")), &game), "Shipped");
}

#[test]
fn a_manifest_that_names_nothing_falls_through_to_the_project() {
    // A game staged before `[game]` existed has a manifest with a `name` from
    // the directory, so this is really the `icon` case — but the fallback is
    // the same one and worth pinning on both.
    let game = declared("[game]\nicon = \"icon.png\"\n");
    assert_eq!(
        icon_path(Some(&manifest("")), &game).as_deref(),
        Some("icon.png")
    );
    assert_eq!(
        icon_path(Some(&manifest("icon = \"shipped.png\"\n")), &game).as_deref(),
        Some("shipped.png")
    );
}

#[test]
fn a_malformed_manifest_is_not_a_crash() {
    // Read with the same leniency as the boot scene: a manifest that does not
    // parse means no manifest, and the project decides.
    let game = declared("[game]\nname = \"Confluence\"\n");
    assert_eq!(title(Some("this is not toml ]["), &game), "Confluence");
    assert_eq!(title(Some(""), &declared("")), DEFAULT_TITLE);
}
