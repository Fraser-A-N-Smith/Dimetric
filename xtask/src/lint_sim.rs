//! Enforces the simulation invariants that can be checked mechanically.
//!
//! Invariants I3, I4 and I5 all fail the same way: silently, and weeks later,
//! as a replay that diverges on someone else's machine. None of them produces a
//! compile error on its own, so they are checked here and in CI.
//!
//! A line may opt out with an `I3-exempt:` comment naming the reason, either on
//! the line itself or in the doc comment of the function it sits in.
//! Conversions at the render, import and scripting boundaries are legitimate;
//! the marker makes each one a deliberate, greppable decision documented where
//! a reader will see it, rather than an oversight.

use std::path::{Path, PathBuf};

/// Crates whose `src/` is simulation code.
const SIM_CRATES: &[&str] = &["dimetric-core", "dimetric-scene", "dimetric-sim"];

/// Patterns that break an invariant, with the invariant they break.
const BANNED: &[(&str, &str, &str)] = &[
    ("f32", "I3", "no floats in simulation"),
    ("f64", "I3", "no floats in simulation"),
    (
        "HashMap",
        "I4",
        "iteration order must not depend on hashing",
    ),
    (
        "HashSet",
        "I4",
        "iteration order must not depend on hashing",
    ),
    (
        "Instant::now",
        "I5",
        "simulation sees tick counts, not a clock",
    ),
    (
        "SystemTime",
        "I5",
        "simulation sees tick counts, not a clock",
    ),
    (
        "std::time",
        "I5",
        "simulation sees tick counts, not a clock",
    ),
];

/// The marker that opts a line out.
const EXEMPT: &str = "-exempt:";

pub fn run() -> Result<(), String> {
    let root = crate::workspace_root();
    let mut findings = Vec::new();

    for crate_name in SIM_CRATES {
        let dir = root.join("crates").join(crate_name).join("src");
        let mut files = Vec::new();
        collect_rust(&dir, &mut files)?;
        files.sort();
        for file in files {
            check_file(&file, &root, &mut findings)?;
        }
    }

    if findings.is_empty() {
        eprintln!("xtask: lint-sim clean across {} crates", SIM_CRATES.len());
        return Ok(());
    }
    let mut message = format!("{} simulation invariant violations:\n", findings.len());
    for f in &findings {
        message.push_str(&format!("  {f}\n"));
    }
    message.push_str(
        "\nIf a conversion is genuinely at the render or import boundary, mark the line\n\
         with an `I3-exempt:` comment saying why.",
    );
    Err(message)
}

fn collect_rust(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_rust(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn check_file(path: &Path, root: &Path, findings: &mut Vec<String>) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();

    for (index, line) in lines.iter().enumerate() {
        // Only the code counts. A doc comment explaining why floats are banned
        // should not itself trip the check.
        let code = strip_comment(line);
        if code.trim().is_empty() {
            continue;
        }
        if line.contains(EXEMPT)
            || comment_block_above_is_exempt(&lines, index)
            || enclosing_item_is_exempt(&lines, index)
        {
            continue;
        }
        for (pattern, invariant, why) in BANNED {
            if code.contains(pattern) {
                findings.push(format!(
                    "{relative}:{}: {invariant} — {why} (found `{pattern}`)\n      {}",
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    Ok(())
}

/// Whether the comment block directly above line `index` marks it exempt.
///
/// The whole block, not just the line before, so a reason long enough to wrap
/// still counts.
fn comment_block_above_is_exempt(lines: &[&str], index: usize) -> bool {
    for i in (0..index).rev() {
        let t = lines[i].trim_start();
        if !t.starts_with("//") {
            return false;
        }
        if t.contains(EXEMPT) {
            return true;
        }
    }
    false
}

/// Whether the function containing line `index` is marked exempt.
///
/// Walks back to the enclosing `fn`, then reads the comment block directly
/// above it. A boundary conversion is a property of the function, so it is
/// stated once in the documentation rather than repeated on every line of the
/// body.
fn enclosing_item_is_exempt(lines: &[&str], index: usize) -> bool {
    let Some(signature) = (0..=index).rev().find(|i| {
        let t = lines[*i].trim_start();
        t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub const fn ")
    }) else {
        return false;
    };
    // Read upward through the attribute and doc-comment block above it.
    for i in (0..signature).rev() {
        let t = lines[i].trim_start();
        if t.starts_with("///") || t.starts_with("//") || t.starts_with("#[") {
            if t.contains(EXEMPT) {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

/// Drop the trailing `//` comment, leaving string literals alone.
fn strip_comment(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'\\' if in_string => escaped = !escaped,
            b'"' if !escaped => in_string = !in_string,
            b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => {
                return line[..i].to_string();
            }
            _ => escaped = false,
        }
    }
    line.to_string()
}
