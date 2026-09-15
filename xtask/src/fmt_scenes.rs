//! Checks or rewrites every `.dim` in the repository in canonical form.
//!
//! Keeping scenes canonical means a diff only ever shows a real change. Without
//! it, the first person whose tooling writes keys in a different order produces
//! a thousand-line diff containing one edit.

use std::path::{Path, PathBuf};

pub fn run(check: bool) -> Result<(), String> {
    let root = crate::workspace_root();
    let mut scenes = Vec::new();
    for dir in ["examples", "tests"] {
        collect(&root.join(dir), &mut scenes);
    }
    scenes.sort();
    if scenes.is_empty() {
        eprintln!("xtask: no scenes found");
        return Ok(());
    }

    let mut offenders = Vec::new();
    for scene in &scenes {
        // A prefab sits in a subdirectory of the project whose kinds it uses,
        // so the project is the nearest directory up that declares any, not
        // the one the file happens to be in.
        let dir = scene.parent().ok_or("a scene needs a directory")?;
        let project = project_root(dir, &root);
        let relative = scene
            .strip_prefix(project)
            .map_err(|_| "a scene outside its project")?
            .with_extension("");
        let name = relative.to_str().ok_or("non-UTF-8 path")?.to_string();
        let name = name.as_str();
        let mut args = vec![
            "run",
            "--quiet",
            "-p",
            "dimetric-agent",
            "--",
            "--project",
            project.to_str().ok_or("non-UTF-8 path")?,
            "--scene",
            name,
            "scene",
            "fmt",
        ];
        if check {
            args.push("--check");
        }
        let status = std::process::Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(&args)
            .status()
            .map_err(|e| format!("running dim: {e}"))?;
        if !status.success() {
            offenders.push(scene.display().to_string());
        }
    }

    if offenders.is_empty() {
        eprintln!("xtask: {} scenes canonical", scenes.len());
        return Ok(());
    }
    Err(format!(
        "{} scenes are not canonical:\n  {}\n\nRun `cargo xtask fmt-scenes` to fix them.",
        offenders.len(),
        offenders.join("\n  ")
    ))
}

/// The nearest ancestor of `dir` that declares node kinds, or `dir` itself.
fn project_root<'a>(dir: &'a Path, stop: &Path) -> &'a Path {
    let mut cursor = dir;
    loop {
        if cursor.join("kinds.toml").is_file() {
            return cursor;
        }
        if cursor == stop {
            return dir;
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => return dir,
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("dim") {
            out.push(path);
        }
    }
}
