//! The project: paths, the open scene, and the bus that edits it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid, Rng};
use dimetric_scene::{KindRegistry, Reference, Scene, SceneDoc, SceneSource};

use dimetric_assets::{Catalog, Imported};

use crate::bus::CommandBus;
use crate::command::Command;

/// Extension for scene files.
pub const SCENE_EXTENSION: &str = "dim";
/// Extension for the editor's view-state sidecar.
pub const SIDECAR_EXTENSION: &str = "dim.editor";
/// Ticks a second a project runs at unless it says otherwise.
pub const DEFAULT_TICK_RATE: u32 = 60;

/// An open project.
pub struct Project {
    /// Project root on disk.
    pub root: PathBuf,
    /// Registered node kinds, the built-ins plus anything `kinds.toml` adds.
    pub registry: KindRegistry,
    /// Anything wrong with the project's own `kinds.toml`.
    ///
    /// Held rather than returned, because opening a project is infallible and
    /// a broken kinds file should surface where a scene fails to load rather
    /// than as a panic on startup.
    pub kind_diagnostics: Diagnostics,
    /// The open scene, if any.
    pub open: Option<SceneDoc>,
    /// Undo and redo.
    pub bus: CommandBus,
    /// Loaded scripts, by project-relative path.
    pub scripts: BTreeMap<String, String>,
    /// Tick rate the project's animation clips are baked against.
    ///
    /// Mirrors `settings.tick_rate`; kept as its own field because the asset
    /// importer takes it directly.
    pub tick_rate: u32,
    /// What `project.toml` declares about how this project runs.
    pub settings: crate::settings::Settings,
    /// Anything wrong with `project.toml`.
    ///
    /// Held rather than returned for the same reason the kind diagnostics are:
    /// opening a project is infallible, and a broken settings file should
    /// surface where it matters rather than as a panic on startup.
    pub settings_diagnostics: Diagnostics,
    /// The asset catalogue as of the last scan.
    catalog: Catalog,
    /// What the last import produced, if one has run.
    imported: Option<Imported>,
    /// Id generator.
    ///
    /// Seeded rather than drawn from the operating system, so that replaying a
    /// recorded command log produces the same ids and the same file.
    ids: Rng,
}

impl Project {
    /// Open a project rooted at `root`.
    pub fn open(root: impl Into<PathBuf>, id_seed: u64) -> Project {
        let root: PathBuf = root.into();
        // A project's own node kinds, if it declares any. Read here rather than
        // on demand because the registry has to be complete before the first
        // scene is parsed — a kind discovered later is a scene that already
        // failed to load.
        let mut registry = KindRegistry::with_builtins();
        let mut kind_diagnostics = Diagnostics::new();
        let kinds_path = root.join(dimetric_scene::project_kinds::KINDS_FILE);
        if let Ok(text) = std::fs::read_to_string(&kinds_path) {
            kind_diagnostics = dimetric_scene::project_kinds::merge(
                &mut registry,
                &text,
                &kinds_path.display().to_string(),
            );
        }
        let (settings, settings_diagnostics) = crate::settings::Settings::load(&root);
        Project {
            tick_rate: settings.tick_rate,
            settings,
            settings_diagnostics,
            root,
            registry,
            kind_diagnostics,
            open: None,
            bus: CommandBus::new(),
            scripts: BTreeMap::new(),
            catalog: Catalog::default(),
            imported: None,
            ids: Rng::new(id_seed, 0x1d1e),
        }
    }

    /// The asset catalogue, as of the last [`Project::scan_assets`].
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// What the last [`Project::import_assets`] produced.
    pub fn imported(&self) -> Option<&Imported> {
        self.imported.as_ref()
    }

    /// Rescan `assets/` and report which assets differ from the last scan.
    ///
    /// Cheap enough to call every frame, and it compares content rather than
    /// modification times: a build step that rewrites a file byte for byte
    /// should not trigger a reload, and a file restored from a backup should.
    pub fn scan_assets(&mut self) -> Vec<String> {
        let fresh = Catalog::scan(&self.root);
        let changed = fresh.changed_since(&self.catalog);
        self.catalog = fresh;
        changed
    }

    /// Import every asset, caching the result in `.import/`.
    ///
    /// The tick rate is baked into animation clips here rather than applied at
    /// runtime, so a project that changes its tick rate has to reimport — which
    /// is the trade the design document makes on purpose.
    pub fn import_assets(&mut self) -> &Imported {
        if self.catalog.is_empty() {
            self.scan_assets();
        }
        let imported = dimetric_assets::import(&self.catalog, self.tick_rate);
        let _ = dimetric_assets::cache::write_metas(&self.catalog, &imported);
        let _ = dimetric_assets::cache::write_cache(&self.root, &imported);
        self.imported.insert(imported)
    }

    /// The animation clips the project's assets imported to.
    ///
    /// Empty until [`Project::import_assets`] has run, which is what a caller
    /// building a simulation should do first.
    /// The baked fonts, for a simulation that measures text.
    ///
    /// The built-in font is included, so `ui.measure` answers for a project
    /// with no font assets — the same fallback a `Label` gets.
    pub fn fonts(&self) -> dimetric_sim::text::Fonts {
        let mut fonts = dimetric_sim::text::Fonts::new();
        let (builtin, _) = dimetric_assets::builtin_font::builtin();
        fonts.insert(
            dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
            builtin,
        );
        if let Some(imported) = &self.imported {
            for (name, artifact) in &imported.artifacts {
                if let dimetric_assets::Artifact::Font { font, .. } = artifact {
                    fonts.insert(name.clone(), font.clone());
                }
            }
        }
        fonts
    }

    /// The animation clips the importer produced, for the simulation.
    pub fn clips(&self) -> dimetric_sim::anim::Clips {
        let Some(imported) = &self.imported else {
            return Default::default();
        };
        imported
            .artifacts
            .iter()
            .filter_map(|(name, artifact)| match artifact {
                dimetric_assets::Artifact::Animation { clips, .. } => {
                    Some((name.clone(), clips.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Assets whose cached artifacts are behind their source.
    pub fn stale_assets(&self) -> Vec<&str> {
        self.catalog
            .entries()
            .filter(|e| e.is_stale())
            .map(|e| e.name.as_str())
            .collect()
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
            // A scene failing on an unknown kind, when the kinds file that was
            // meant to declare it is itself broken, should say so here rather
            // than leave somebody hunting a typo in the scene.
            let mut diagnostics = self.kind_diagnostics.clone();
            diagnostics.extend(out.diagnostics);
            return Err(diagnostics);
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
                self.scan_assets();
                let imported = self.import_assets();
                let name = dimetric_assets::cache::asset_name(path);
                let failure = imported
                    .failures
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, why)| why.clone());
                match failure {
                    Some(why) => Err(Diagnostics(vec![
                        Diagnostic::new(Code::ASSET_MISSING, why).with_field("asset", name)
                    ])),
                    None => Ok(()),
                }
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

    /// Every prefab under `prefabs/`, resolved and ready to spawn.
    ///
    /// Resolved here rather than in the simulation because flattening an
    /// instance needs the project's other scenes, and a tick has no
    /// filesystem. A prefab that will not load is reported and left out, so one
    /// broken file does not stop the run.
    pub fn templates(&self) -> (dimetric_sim::spawn::Templates, Diagnostics) {
        let mut templates = dimetric_sim::spawn::Templates::new();
        let mut diagnostics = Diagnostics::new();
        let dir = self.root.join("prefabs");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return (templates, diagnostics);
        };
        // Sorted, so what is loaded — and what a duplicate name resolves to —
        // never depends on directory order.
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();

        let sources = DiskScenes {
            root: self.root.clone(),
            registry: self.registry.clone(),
        };
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) != Some(SCENE_EXTENSION) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let display = path.display().to_string();
            let out = dimetric_scene::parse(&text, &display, &self.registry);
            let Some(doc) = out.doc else {
                diagnostics.extend(out.diagnostics);
                continue;
            };
            let (resolved, resolve_diagnostics) =
                dimetric_scene::resolve(&doc.scene, &sources, &self.registry);
            diagnostics.extend(resolve_diagnostics);
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            // Both spellings, so a script can say either the bare name or the
            // path a scene reference uses.
            templates.insert(format!("prefabs/{name}"), resolved.clone());
            templates.insert(name, resolved);
        }
        (templates, diagnostics)
    }

    /// The simulation config this project declared.
    ///
    /// One place, because there are several ways to start this project's
    /// simulation — the player, the editor's playback, a headless capture —
    /// and any of them running on the engine's defaults while the project
    /// asked for something else is a different game. `dim frame capture` did
    /// exactly that, and nothing could tell, because a project whose settings
    /// happen to equal the defaults is photographed correctly either way.
    pub fn sim_config(&self) -> dimetric_sim::SimConfig {
        dimetric_sim::SimConfig {
            tick_rate: self.settings.tick_rate,
            canvas: self.settings.canvas,
            resolution: self.settings.resolution,
        }
    }

    /// The internal resolution to render at, and what an override costs.
    ///
    /// `[render] resolution` is documented as the resolution the world is
    /// drawn at, and it is in the state hash because a script unprojects a
    /// click through it. So the renderer has to draw at the same number the
    /// simulation picks with: if they differ, a click lands somewhere other
    /// than under the cursor, and nothing says so.
    ///
    /// They were independent. `RenderSettings::internal_resolution` defaulted
    /// to `(480, 270)` on its own, which is why a project could declare
    /// `resolution = [1920, 1080]`, pick against it, and still be drawn at
    /// 480×270.
    ///
    /// A `--internal` override is still allowed, for a one-off capture at a
    /// size worth looking at. It is not allowed to be silent: overriding it
    /// re-creates exactly the disagreement above for as long as the flag is
    /// there, so this hands back a warning to go with it. What it must not do
    /// is move the simulation's resolution to match — that is hashed, and a
    /// flag that quietly changed the run would make a recording replayable
    /// only by someone who passed the same flag.
    pub fn render_resolution(
        &self,
        override_with: Option<(u32, u32)>,
    ) -> ((u32, u32), Option<Diagnostic>) {
        let declared = self.settings.resolution;
        match override_with {
            None => (declared, None),
            Some(size) if size == declared => (size, None),
            Some(size) => (
                size,
                Some(
                    Diagnostic::new(
                        Code::SETTINGS_OVERRIDDEN,
                        format!(
                            "drawing at {}x{} while the project's `[render] resolution` \
                             is {}x{}; the simulation still picks against {}x{}, so a \
                             click will not land where the cursor is",
                            size.0, size.1, declared.0, declared.1, declared.0, declared.1
                        ),
                    )
                    .with_severity(dimetric_core::Severity::Warning),
                ),
            ),
        }
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
