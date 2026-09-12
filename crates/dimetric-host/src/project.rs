//! The project: paths, the open scene, and the bus that edits it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid, Rng};
use dimetric_scene::{KindRegistry, Reference, Scene, SceneDoc, SceneSource};

use crate::bus::CommandBus;
use crate::command::Command;

/// Extension for scene files.
pub const SCENE_EXTENSION: &str = "dim";
/// Extension for the editor's view-state sidecar.
pub const SIDECAR_EXTENSION: &str = "dim.editor";

/// An open project.
pub struct Project {
    /// Project root on disk.
    pub root: PathBuf,
    /// Registered node kinds.
    pub registry: KindRegistry,
    /// The open scene, if any.
    pub open: Option<SceneDoc>,
    /// Undo and redo.
    pub bus: CommandBus,
    /// Loaded scripts, by project-relative path.
    pub scripts: BTreeMap<String, String>,
    /// Id generator.
    ///
    /// Seeded rather than drawn from the operating system, so that replaying a
    /// recorded command log produces the same ids and the same file.
    ids: Rng,
}

impl Project {
    /// Open a project rooted at `root`.
    pub fn open(root: impl Into<PathBuf>, id_seed: u64) -> Project {
        Project {
            root: root.into(),
            registry: KindRegistry::with_builtins(),
            open: None,
            bus: CommandBus::new(),
            scripts: BTreeMap::new(),
            ids: Rng::new(id_seed, 0x1d1e),
        }
    }

    /// Draw a fresh node id that is not already used by the open scene.
    pub fn new_node_id(&mut self) -> NodeUid {
        loop {
            let uid = NodeUid::generate(&mut self.ids);
            let taken = self
                .open
                .as_ref()
                .is_some_and(|d| d.scene.contains_uid(uid));
            if !taken {
                return uid;
            }
        }
    }

    /// Resolve a project-relative path.
    pub fn path_of(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// The path of a scene reference such as `prefabs/skeleton`.
    pub fn scene_path(&self, name: &str) -> PathBuf {
        let trimmed = name.trim_start_matches("scene:");
        if trimmed.ends_with(&format!(".{SCENE_EXTENSION}")) {
            self.root.join(trimmed)
        } else {
            self.root.join(format!("{trimmed}.{SCENE_EXTENSION}"))
        }
    }

    /// Load a scene into the project.
    pub fn load_scene(&mut self, relative: &str) -> Result<Diagnostics, Diagnostics> {
        let path = self.scene_path(relative);
        let source = std::fs::read_to_string(&path).map_err(|e| {
            Diagnostics(vec![Diagnostic::new(
                Code::ASSET_MISSING,
                format!("cannot read {}: {e}", path.display()),
            )
            .with_span(dimetric_core::Span::file(path.display().to_string()))])
        })?;
        let display = path.display().to_string();
        let out = dimetric_scene::parse(&source, &display, &self.registry);
        if out.diagnostics.has_errors() {
            return Err(out.diagnostics);
        }
        self.open = out.doc;
        self.bus.clear();
        Ok(out.diagnostics)
    }

    /// Write the open scene back to disk.
    ///
    /// Writes the `toml_edit` document, so an unedited scene is written back
    /// exactly as it was read (I2).
    pub fn save_scene(&self, relative: Option<&str>) -> Result<PathBuf, Diagnostic> {
        let doc = self
            .open
            .as_ref()
            .ok_or_else(|| Diagnostic::new(Code::COMMAND_REJECTED, "no scene is open"))?;
        let path = match relative {
            Some(r) => self.scene_path(r),
            None => PathBuf::from(&doc.source_path),
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, doc.to_text()).map_err(|e| {
            Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("cannot write {}: {e}", path.display()),
            )
        })?;
        Ok(path)
    }

    /// Apply a command, routing project-level ones away from the scene bus.
    pub fn apply(&mut self, command: Command) -> Result<(), Diagnostics> {
        match &command {
            Command::LoadScene { path } => {
                self.load_scene(path)?;
                Ok(())
            }
            Command::SaveScene { path } => {
                self.save_scene(path.as_deref()).map_err(one)?;
                Ok(())
            }
            Command::WriteScript { path, source } => {
                let full = self.path_of(path);
                if let Some(parent) = full.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&full, source).map_err(|e| {
                    Diagnostics(vec![Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        format!("cannot write {}: {e}", full.display()),
                    )])
                })?;
                self.scripts.insert(path.clone(), source.clone());
                Ok(())
            }
            Command::ImportAsset { path } => {
                let full = self.path_of(path);
                if !full.exists() {
                    return Err(Diagnostics(vec![Diagnostic::new(
                        Code::ASSET_MISSING,
                        format!("{} is not in the project", full.display()),
                    )]));
                }
                Ok(())
            }
            _ => {
                let registry = self.registry.clone();
                let doc = self.open.as_mut().ok_or_else(|| {
                    Diagnostics(vec![Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        "no scene is open",
                    )])
                })?;
                self.bus.apply(doc, &registry, command).map_err(one)
            }
        }
    }

    /// Undo the last scene edit.
    pub fn undo(&mut self) -> Result<Command, Diagnostic> {
        let registry = self.registry.clone();
        let doc = self
            .open
            .as_mut()
            .ok_or_else(|| Diagnostic::new(Code::COMMAND_REJECTED, "no scene is open"))?;
        self.bus.undo(doc, &registry)
    }

    /// Redo the last undone edit.
    pub fn redo(&mut self) -> Result<Command, Diagnostic> {
        let registry = self.registry.clone();
        let doc = self
            .open
            .as_mut()
            .ok_or_else(|| Diagnostic::new(Code::COMMAND_REJECTED, "no scene is open"))?;
        self.bus.redo(doc, &registry)
    }

    /// The open scene with every prefab instance resolved, ready to simulate.
    pub fn runtime_scene(&self) -> Result<(Scene, Diagnostics), Diagnostics> {
        let doc = self.open.as_ref().ok_or_else(|| {
            Diagnostics(vec![Diagnostic::new(
                Code::COMMAND_REJECTED,
                "no scene is open",
            )])
        })?;
        let sources = DiskScenes {
            root: self.root.clone(),
            registry: self.registry.clone(),
        };
        Ok(dimetric_scene::resolve(
            &doc.scene,
            &sources,
            &self.registry,
        ))
    }

    /// Load every `.lua` file under `scripts/`.
    pub fn load_scripts(&mut self) -> Diagnostics {
        let mut diags = Diagnostics::new();
        let dir = self.root.join("scripts");
        collect_scripts(&dir, &self.root, &mut self.scripts, &mut diags);
        diags
    }
}

fn one(d: Diagnostic) -> Diagnostics {
    Diagnostics(vec![d])
}

fn collect_scripts(
    dir: &Path,
    root: &Path,
    out: &mut BTreeMap<String, String>,
    diags: &mut Diagnostics,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Sorted, so scripts load in the same order everywhere. Directory order is
    // filesystem-dependent and would make load order a property of the machine.
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_scripts(&path, root, out, diags);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                let key = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(key, source);
            }
            Err(e) => diags.push(Diagnostic::new(
                Code::ASSET_MISSING,
                format!("cannot read {}: {e}", path.display()),
            )),
        }
    }
}

/// Loads prefabs from the project directory.
pub struct DiskScenes {
    /// Project root.
    pub root: PathBuf,
    /// Kinds the prefabs may use.
    pub registry: KindRegistry,
}

impl SceneSource for DiskScenes {
    fn load(&self, reference: &Reference) -> Result<Scene, Diagnostic> {
        let name = reference.target();
        let path = if name.ends_with(&format!(".{SCENE_EXTENSION}")) {
            self.root.join(name)
        } else {
            self.root.join(format!("{name}.{SCENE_EXTENSION}"))
        };
        let source = std::fs::read_to_string(&path).map_err(|e| {
            Diagnostic::new(
                Code::ASSET_MISSING,
                format!("cannot read prefab {}: {e}", path.display()),
            )
            .with_field("reference", reference.to_text())
        })?;
        let display = path.display().to_string();
        let out = dimetric_scene::parse(&source, &display, &self.registry);
        if out.diagnostics.has_errors() {
            return Err(Diagnostic::new(
                Code::ASSET_MISSING,
                format!("prefab {} has errors:\n{}", display, out.diagnostics),
            )
            .with_field("reference", reference.to_text()));
        }
        out.doc
            .map(|d| d.scene)
            .ok_or_else(|| Diagnostic::new(Code::ASSET_MISSING, format!("{display} is empty")))
    }
}
