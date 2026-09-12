//! Enforces the crate dependency direction.
//!
//! Dependencies run strictly downward:
//!
//! ```text
//! core <- scene <- {sim, render, audio, assets} <- host <- {editor, agent}
//! ```
//!
//! with `platform` beside them. An upward edge is how a layered engine becomes
//! one crate with ten directories, and it is far cheaper to refuse the first
//! one than to untangle the fortieth.

use std::collections::BTreeMap;

/// What each crate is allowed to depend on, within the workspace.
fn allowed() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("dimetric-core", vec![]),
        ("dimetric-scene", vec!["dimetric-core"]),
        ("dimetric-sim", vec!["dimetric-core", "dimetric-scene"]),
        ("dimetric-render", vec!["dimetric-core", "dimetric-scene"]),
        ("dimetric-audio", vec!["dimetric-core"]),
        ("dimetric-assets", vec!["dimetric-core"]),
        // Input is a simulation type, so the platform layer that sources it
        // depends on sim. Nothing above platform depends on it except host.
        ("dimetric-platform", vec!["dimetric-core", "dimetric-sim"]),
        (
            "dimetric-host",
            vec!["dimetric-core", "dimetric-scene", "dimetric-sim"],
        ),
        ("dimetric-editor", vec!["dimetric-core", "dimetric-host"]),
        (
            "dimetric-agent",
            vec![
                "dimetric-core",
                "dimetric-scene",
                "dimetric-sim",
                "dimetric-host",
            ],
        ),
        // The umbrella exists to re-export, so it may reach anything.
        (
            "dimetric",
            vec![
                "dimetric-core",
                "dimetric-scene",
                "dimetric-sim",
                "dimetric-render",
                "dimetric-audio",
                "dimetric-assets",
                "dimetric-host",
                "dimetric-platform",
                "dimetric-editor",
            ],
        ),
    ])
}

pub fn run() -> Result<(), String> {
    let root = crate::workspace_root();
    let rules = allowed();
    let mut findings = Vec::new();

    for (crate_name, permitted) in &rules {
        let manifest = root.join("crates").join(crate_name).join("Cargo.toml");
        let text = match std::fs::read_to_string(&manifest) {
            Ok(t) => t,
            Err(e) => return Err(format!("reading {}: {e}", manifest.display())),
        };
        for dependency in engine_dependencies(&text) {
            if !permitted.contains(&dependency.as_str()) {
                findings.push(format!(
                    "{crate_name} depends on {dependency}, which is not below it"
                ));
            }
        }
    }

    if findings.is_empty() {
        eprintln!(
            "xtask: dependency direction clean across {} crates",
            rules.len()
        );
        return Ok(());
    }
    let mut message = String::from("dependency direction violated:\n");
    for f in &findings {
        message.push_str(&format!("  {f}\n"));
    }
    message.push_str(
        "\nDependencies run core <- scene <- {sim, render, audio, assets} <- host <- {editor, agent}.\n\
         If the new edge is genuinely right, change the rule in xtask/src/check_deps.rs and say why.",
    );
    Err(message)
}

/// Engine crates named in a manifest's dependency sections.
fn engine_dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed.contains("dependencies");
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let Some(name) = trimmed.split(['=', '.', ' ']).next() else {
            continue;
        };
        if name.starts_with("dimetric-") {
            out.push(name.to_string());
        }
    }
    out
}
