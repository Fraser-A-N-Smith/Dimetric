//! The reference cannot claim a different sandbox than the one that exists.
//!
//! `docs/API.md` said "scripts see exactly these globals and nothing else" and
//! then listed ten of the eleven, for a whole milestone. The cause was that
//! the Lua section was a string literal in `xtask` while the command, kind and
//! diagnostic tables around it were generated — so I10's "documented" held
//! everywhere except the one place a script author actually reads.
//!
//! These tests are the fix. The manifest is data, the sandbox is introspected,
//! and the two have to agree in both directions.

use dimetric_sim::api_doc::{BORROWED, GLOBALS};
use dimetric_sim::script::sandbox_globals;

#[test]
fn every_global_the_sandbox_installs_is_documented() {
    // The direction that actually failed: `ui` existed and was not written
    // down.
    let actual = sandbox_globals().expect("sandbox builds");
    for name in &actual {
        if BORROWED.contains(&name.as_str()) {
            continue;
        }
        assert!(
            GLOBALS.iter().any(|g| g.name == name),
            "the sandbox installs `{name}` and docs/API.md does not mention it. \
             Add it to dimetric_sim::api_doc::GLOBALS."
        );
    }
}

#[test]
fn every_documented_global_is_actually_installed() {
    // The other direction, which is how a reference ends up promising an API
    // that was removed three releases ago.
    let actual = sandbox_globals().expect("sandbox builds");
    for global in GLOBALS {
        assert!(
            actual.iter().any(|n| n == global.name),
            "docs/API.md documents `{}` and the sandbox does not install it",
            global.name
        );
    }
}

#[test]
fn the_borrowed_names_are_all_really_borrowed() {
    // Stops the subtraction list rotting into a way to hide a global: a name
    // listed as borrowed that the sandbox does not have would silently excuse
    // a real global of the same name later.
    let actual = sandbox_globals().expect("sandbox builds");
    for name in BORROWED {
        assert!(
            actual.iter().any(|n| n == name),
            "`{name}` is listed as a borrowed Lua name and the sandbox has no such key"
        );
    }
}

#[test]
fn the_counts_agree() {
    // A blunt cross-check, so the number in the prose can be trusted.
    let actual = sandbox_globals().expect("sandbox builds");
    assert_eq!(
        actual.len(),
        GLOBALS.len() + BORROWED.len(),
        "sandbox has {} names; the manifest accounts for {} engine globals \
         plus {} borrowed",
        actual.len(),
        GLOBALS.len(),
        BORROWED.len()
    );
}
