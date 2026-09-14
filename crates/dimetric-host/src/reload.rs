//! Hot reload, applied at a tick boundary.
//!
//! # Why the boundary matters
//!
//! A tick is a pure function of the state it starts from and the input it is
//! given (I8). Swapping a script or a texture part-way through one would make
//! it a function of when the file happened to be saved, and the state hash for
//! that tick would describe nothing anybody could reproduce.
//!
//! The boundary is not a convention here, it is the borrow checker: applying a
//! reload needs `&mut Sim`, and so does `Sim::step`. There is no arrangement of
//! calls that reloads inside a tick.
//!
//! # What reloading does and does not keep
//!
//! Reloading a script keeps every variable the nodes running it had, because
//! script state lives in `SimState` rather than in Lua globals. What changes is
//! the code. A function the new source deleted is gone, since the script gets a
//! fresh environment.
//!
//! Scene files are reported and not applied. Reloading the scene mid-run would
//! throw away the simulation, which is a restart rather than a reload, and the
//! caller is better placed to decide that.
//!
//! # Replay
//!
//! A replay that picked up an edited script would not be a replay. In
//! [`RunMode::Replay`] nothing is queued and nothing is applied.

use std::collections::BTreeSet;

use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_sim::Sim;

use crate::project::Project;
use crate::run::RunMode;

/// Something on disk that moved.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Change {
    /// An asset, by the name a scene refers to it by.
    Asset(String),
    /// A script, by project-relative path.
    Script(String),
    /// A scene, by project-relative path.
    Scene(String),
}

impl Change {
    /// What moved.
    pub fn target(&self) -> &str {
        match self {
            Change::Asset(s) | Change::Script(s) | Change::Scene(s) => s,
        }
    }
}

/// What a reload did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Applied {
    /// Assets reimported.
    pub assets: Vec<String>,
    /// Scripts swapped into the running simulation.
    pub scripts: Vec<String>,
    /// Scenes that changed and were left alone.
    pub scenes: Vec<String>,
}

impl Applied {
    /// Whether anything happened.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty() && self.scripts.is_empty() && self.scenes.is_empty()
    }

    /// How many files were involved.
    pub fn len(&self) -> usize {
        self.assets.len() + self.scripts.len() + self.scenes.len()
    }
}

/// Watches a project for changes and applies them between ticks.
pub struct Reloader {
    mode: RunMode,
    pending: BTreeSet<Change>,
    scripts: std::collections::BTreeMap<String, String>,
    scenes: std::collections::BTreeMap<String, String>,
    reloads: u64,
}

impl Reloader {
    /// Watch a project.
    ///
    /// The first [`Reloader::poll`] after this reports nothing: what is on disk
    /// when watching starts is the baseline, not a change.
    pub fn new(mode: RunMode, project: &mut Project) -> Reloader {
        let mut reloader = Reloader {
            mode,
            pending: BTreeSet::new(),
            scripts: Default::default(),
            scenes: Default::default(),
            reloads: 0,
        };
        project.scan_assets();
        reloader.scripts = script_hashes(project);
        reloader.scenes = scene_hashes(project);
        reloader
    }

    /// How many reloads have been applied.
    pub fn reloads(&self) -> u64 {
        self.reloads
    }

    /// Changes seen but not yet applied, in a stable order.
    pub fn pending(&self) -> impl Iterator<Item = &Change> {
        self.pending.iter()
    }

    /// Look for changes. Returns how many are now waiting.
    ///
    /// Safe to call at any point, including mid-frame: it only reads the
    /// filesystem and adds to a queue.
    pub fn poll(&mut self, project: &mut Project) -> usize {
        if self.mode == RunMode::Replay {
            return 0;
        }
        for name in project.scan_assets() {
            self.pending.insert(Change::Asset(name));
        }
        let scripts = script_hashes(project);
        for path in changed(&self.scripts, &scripts) {
            self.pending.insert(Change::Script(path));
        }
        self.scripts = scripts;

        let scenes = scene_hashes(project);
        for path in changed(&self.scenes, &scenes) {
            self.pending.insert(Change::Scene(path));
        }
        self.scenes = scenes;

        self.pending.len()
    }

    /// Apply everything waiting.
    ///
    /// Taking `&mut Sim` is what pins this to a tick boundary: `Sim::step` also
    /// takes `&mut self`, so a reload cannot overlap one.
    pub fn apply(&mut self, project: &mut Project, sim: &mut Sim) -> (Applied, Diagnostics) {
        let mut applied = Applied::default();
        let mut diagnostics = Diagnostics::new();
        if self.pending.is_empty() {
            return (applied, diagnostics);
        }
        if self.mode == RunMode::Replay {
            self.pending.clear();
            diagnostics.push(Diagnostic::new(
                Code::COMMAND_REJECTED,
                "a replay does not hot reload; a run that picked up an edited script \
                 would not reproduce the recording",
            ));
            return (applied, diagnostics);
        }

        let pending = std::mem::take(&mut self.pending);
        let reimport = pending.iter().any(|c| matches!(c, Change::Asset(_)));

        for change in pending {
            match change {
                Change::Asset(name) => applied.assets.push(name),
                Change::Scene(path) => applied.scenes.push(path),
                Change::Script(path) => {
                    let source = match std::fs::read_to_string(project.path_of(&path)) {
                        Ok(source) => source,
                        // Deleted, or being written as we looked. Leave the old
                        // source running rather than tearing a script out from
                        // under the nodes using it.
                        Err(_) => continue,
                    };
                    match sim.scripts_mut().reload(&path, &source) {
                        Ok(()) => {
                            project.scripts.insert(path.clone(), source);
                            applied.scripts.push(path);
                        }
                        // A script that will not compile keeps the version that
                        // did. The alternative is a typo stopping the game.
                        Err(d) => diagnostics.push(d),
                    }
                }
            }
        }

        if reimport {
            let imported = project.import_assets();
            for (asset, why) in &imported.failures {
                diagnostics.push(
                    Diagnostic::new(Code::ASSET_MISSING, why.clone())
                        .with_field("asset", asset.clone())
                        .with_severity(dimetric_core::Severity::Warning),
                );
            }
        }

        if !applied.is_empty() {
            self.reloads += 1;
        }
        (applied, diagnostics)
    }
}

fn changed(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut out: Vec<String> = after
        .iter()
        .filter(|(path, hash)| before.get(*path) != Some(*hash))
        .map(|(path, _)| path.clone())
        .collect();
    out.extend(
        before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .cloned(),
    );
    out.sort();
    out.dedup();
    out
}

fn script_hashes(project: &mut Project) -> std::collections::BTreeMap<String, String> {
    let mut scripts = std::collections::BTreeMap::new();
    std::mem::swap(&mut scripts, &mut project.scripts);
    project.load_scripts();
    let hashes: std::collections::BTreeMap<String, String> = project
        .scripts
        .iter()
        .map(|(path, source)| {
            (
                path.clone(),
                dimetric_assets::content_hash(source.as_bytes()),
            )
        })
        .collect();
    // `load_scripts` merges into whatever is already there, so a script deleted
    // on disk would otherwise never look deleted.
    project.scripts.retain(|path, _| hashes.contains_key(path));
    hashes
}

fn scene_hashes(project: &Project) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    walk(&project.root, &project.root, &mut out);
    out
}

fn walk(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> =
        entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            // The cache is derived, so a change in it is an effect rather than
            // a cause.
            if name != dimetric_assets::IMPORT_DIR && !name.starts_with('.') {
                walk(&path, root, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(crate::project::SCENE_EXTENSION)
        {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, dimetric_assets::content_hash(text.as_bytes()));
            }
        }
    }
}
