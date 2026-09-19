//! `dim script check` checks the program the game actually runs.
//!
//! Two defects, both of which made the check quieter than the truth:
//!
//! **`require` could not resolve.** The command built a bare host and loaded
//! one file into it, so the first `require` line raised "unknown module" and
//! everything after it went unexamined. A script that pulls in a module — which
//! is every script worth linting — could not be checked at all.
//!
//! **A load failure suppressed the lint.** The determinism scan is a text scan
//! with no parser behind it, so it never depended on the load; but it sat
//! behind a `?` and a syntax error in one function hid a `pairs()` in the next.
//!
//! And one absence: there was no way to ask about a project, only about a file.
//! A caller who has to write the loop is the caller who skips the four files
//! that matter.

use clap::Parser;
use dimetric_agent::cli::Cli;

/// A project with a scene and the given scripts.
fn project(scripts: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    for (path, source) in scripts {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("a parent")).expect("mkdir");
        std::fs::write(full, source).expect("script");
    }
    dir
}

/// Run `dim` over that project, as a shell would.
fn dim(root: &std::path::Path, args: &[&str]) -> Result<serde_json::Value, Vec<String>> {
    let mut argv = vec!["dim", "--project", root.to_str().expect("utf-8")];
    argv.extend_from_slice(args);
    match dimetric_agent::run(Cli::parse_from(argv)) {
        Ok(out) => Ok(out.body),
        Err(diags) => Err(diags.iter().map(|d| d.to_string()).collect()),
    }
}

const SPELLBOOK: &str = "local M = {}\nM.bolt = { damage = 3 }\nreturn M\n";

const ARENA: &str = "local spells = require(\"scripts/spellbook.lua\")\n\
                     function on_tick(self)\n\
                     \x20 for k, v in pairs(spells) do\n\
                     \x20   self.damage = v.damage\n\
                     \x20 end\n\
                     end\n";

const BROKEN: &str = "function on_tick(self)\n\
                      \x20 self.x = self.x + 0.1\n\
                      \x20 this is not lua\n\
                      end\n";

#[test]
fn a_script_that_requires_a_module_can_be_checked() {
    let dir = project(&[
        ("scripts/spellbook.lua", SPELLBOOK),
        ("scripts/arena.lua", ARENA),
    ]);
    let body = dim(dir.path(), &["script", "check", "scripts/arena.lua"])
        .expect("a script that requires a module checks");
    assert_eq!(body["checked"], serde_json::json!(["scripts/arena.lua"]));
    assert_eq!(body["files"][0]["parses"], true);
}

#[test]
fn the_module_does_not_have_to_sort_before_the_script_that_requires_it() {
    // `scripts/arena.lua` sorts before `scripts/zzz.lua`. Registering every
    // source before loading any is what keeps that from mattering.
    let dir = project(&[
        ("scripts/zzz.lua", SPELLBOOK),
        (
            "scripts/arena.lua",
            "local spells = require(\"scripts/zzz.lua\")\nreturn spells.bolt.damage\n",
        ),
    ]);
    dim(dir.path(), &["script", "check", "scripts/arena.lua"])
        .expect("load order is not the alphabet");
}

#[test]
fn a_hazard_behind_a_require_is_reported() {
    // The `pairs()` on line 3 was invisible before: the check died on line 1.
    let dir = project(&[
        ("scripts/spellbook.lua", SPELLBOOK),
        ("scripts/arena.lua", ARENA),
    ]);
    let body = dim(
        dir.path(),
        &["script", "check", "--determinism", "scripts/arena.lua"],
    )
    .expect("it checks");
    let hazards = body["hazards"].as_array().expect("an array");
    assert_eq!(hazards.len(), 1, "one `pairs()`: {hazards:?}");
    assert_eq!(hazards[0]["code"], "DIM0506");
    assert_eq!(hazards[0]["span"]["line"], 3);
}

#[test]
fn a_file_that_does_not_parse_still_gets_scanned() {
    let dir = project(&[("scripts/broken.lua", BROKEN)]);
    let reported = dim(
        dir.path(),
        &["script", "check", "--determinism", "scripts/broken.lua"],
    )
    .expect_err("a syntax error fails the command");
    // Both, not either: the load error and the hazard it used to hide.
    assert!(
        reported.iter().any(|d| d.contains("DIM0501")),
        "the syntax error is reported: {reported:?}"
    );
    assert!(
        reported.iter().any(|d| d.contains("DIM0506")),
        "and so is the float it used to hide: {reported:?}"
    );
}

#[test]
fn no_path_means_every_script_in_the_project() {
    let dir = project(&[
        ("scripts/spellbook.lua", SPELLBOOK),
        ("scripts/arena.lua", ARENA),
        ("scripts/nested/quiet.lua", "local x = 1\n"),
    ]);
    let body = dim(dir.path(), &["script", "check"]).expect("a whole project checks");
    assert_eq!(
        body["checked"],
        serde_json::json!([
            "scripts/arena.lua",
            "scripts/nested/quiet.lua",
            "scripts/spellbook.lua"
        ]),
        "every script, in a sorted order that does not depend on the machine"
    );
}

#[test]
fn a_project_wide_check_finds_the_file_a_caller_would_have_skipped() {
    let dir = project(&[
        ("scripts/spellbook.lua", SPELLBOOK),
        ("scripts/arena.lua", ARENA),
    ]);
    let body = dim(dir.path(), &["script", "check", "--determinism"]).expect("it checks");
    let hazards = body["hazards"].as_array().expect("an array");
    assert_eq!(hazards.len(), 1);
    assert_eq!(hazards[0]["span"]["file"], "scripts/arena.lua");
}

#[test]
fn one_broken_file_does_not_hide_another_files_hazards() {
    let dir = project(&[
        ("scripts/spellbook.lua", SPELLBOOK),
        ("scripts/arena.lua", ARENA),
        ("scripts/broken.lua", BROKEN),
    ]);
    let reported = dim(dir.path(), &["script", "check", "--determinism"])
        .expect_err("a project with a broken script fails");
    assert!(
        reported
            .iter()
            .any(|d| d.contains("DIM0506") && d.contains("scripts/arena.lua")),
        "the other file's hazard survives: {reported:?}"
    );
    assert!(
        reported
            .iter()
            .any(|d| d.contains("DIM0501") && d.contains("scripts/broken.lua")),
        "and the broken one is named: {reported:?}"
    );
}

#[test]
fn a_project_with_no_scripts_reports_an_empty_set() {
    // It succeeds — a project with no Lua in it yet is not a failure — but it
    // says how many files it looked at, so a caller can tell "clean" from
    // "looked in the wrong place".
    let dir = project(&[]);
    let body = dim(dir.path(), &["script", "check", "--determinism"]).expect("nothing to check");
    assert_eq!(body["checked"], serde_json::json!([]));
}

#[test]
fn writing_a_script_that_requires_a_module_works() {
    // `script write` checks before it writes, and had the same bare host.
    let dir = project(&[("scripts/spellbook.lua", SPELLBOOK)]);
    dim(
        dir.path(),
        &[
            "script",
            "write",
            "scripts/arena.lua",
            "--source",
            "local spells = require(\"scripts/spellbook.lua\")\nreturn spells\n",
        ],
    )
    .expect("a script that requires a module can be written");
    assert!(dir.path().join("scripts/arena.lua").exists());
}
