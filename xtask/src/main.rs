//! Repository automation. `cargo xtask <task>`.
//!
//! These are the checks CI runs, in a form you can run locally.

mod check_deps;
mod fmt_scenes;
mod gen_docs;
mod gen_trig;
mod lint_sim;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];

    let result = match task {
        "gen-trig" => gen_trig::run(),
        "lint-sim" => lint_sim::run(),
        "check-deps" => check_deps::run(),
        "gen-docs" => gen_docs::run(),
        "fmt-scenes" => fmt_scenes::run(args.iter().any(|a| a == "--check")),
        "ci" => run_ci(),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown task {other:?}\n\n{USAGE}")),
    };
    let _ = rest;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
tasks:
  ci          everything below, in the order CI runs it
  lint-sim    refuse floats, HashMap iteration and wall-clock reads in sim crates (I3, I4, I5)
  check-deps  refuse upward dependencies between crates
  fmt-scenes  rewrite every .dim in canonical form (--check to only report)
  gen-docs    regenerate docs/API.md and docs/schemas/
  gen-trig    regenerate the committed trig tables
";

fn usage() {
    print!("{USAGE}");
}

fn run_ci() -> Result<(), String> {
    lint_sim::run()?;
    check_deps::run()?;
    fmt_scenes::run(true)?;
    // docs/ is a committed build artifact. The workflow checks it too, but
    // finding out here costs a minute and finding out there costs a round trip.
    check_generated_docs()?;
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    Ok(())
}

/// Regenerate `docs/` and fail if that changed anything.
///
/// The question is whether the committed artifacts match what the engine
/// generates now, which is a before-and-after comparison rather than a `git
/// status`: docs legitimately change in the working tree alongside the schema
/// change that caused them.
fn check_generated_docs() -> Result<(), String> {
    let docs = workspace_root().join("docs");
    let before = snapshot(&docs);
    gen_docs::run()?;
    let after = snapshot(&docs);

    let stale: Vec<String> = after
        .iter()
        .filter(|(path, content)| before.get(*path) != Some(*content))
        .map(|(path, _)| path.clone())
        .collect();
    if stale.is_empty() {
        eprintln!("xtask: generated documentation is current");
        return Ok(());
    }
    Err(format!(
        "docs/ did not match what the engine generates, and has been rewritten:\n  {}\n\n\
         Commit the result — an agent reading a stale contract writes confident code \
         against an API that no longer exists.",
        stale.join("\n  ")
    ))
}

/// Every file under a directory, by relative path.
fn snapshot(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(
        dir: &std::path::Path,
        root: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(name, bytes);
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Run a cargo subcommand, inheriting stdio.
pub fn cargo(args: &[&str]) -> Result<(), String> {
    eprintln!("xtask: cargo {}", args.join(" "));
    let status = std::process::Command::new(env!("CARGO"))
        .args(args)
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

/// The workspace root, derived from this crate's manifest location.
pub fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}
