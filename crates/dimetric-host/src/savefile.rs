//! Writing a run to disk and reading it back.
//!
//! # Two kinds of persistence, and why conflating them is the bug
//!
//! A **run** is simulation state: the floor, the adventurer, the RNG streams.
//! Saving one and restoring it has to be exact, or the resumed run is a
//! different run. It is hashed, snapshotted and rewindable, and it lives here.
//!
//! A **profile** is what accumulates across runs — knowledge, awards, unlocks.
//! It must never enter the hash, because it differs between two players
//! playing the same seed. That lives in [`crate::profile`], deliberately
//! nowhere near this file, because the one thing that must not happen is a
//! profile finding its way into a save and thence into the state.
//!
//! # Why the scene is written as a scene
//!
//! The obvious route is to derive `Serialize` across the state graph and emit
//! one blob. For everything except the tree that is what happens. The tree
//! gets written as canonical `.dim` text instead, for two reasons.
//!
//! I2 already guarantees that loading a scene and writing it back is
//! byte-identical, and that guarantee is tested. A second serialisation of the
//! same tree would be a second thing to keep exact, and the day the two
//! disagree is the day a save loads a subtly different world.
//!
//! And a save that is text is a save somebody can read. This engine's whole
//! bet is that a bug arrives as a seed and a diff; a binary savefile is the
//! one place that would stop being true.
//!
//! # Why a stale save fails rather than loading wrong
//!
//! A save carries the engine version that wrote it and a format version.
//! Loading a run recorded against different code is exactly the situation
//! where "mostly works" is worse than refusing: the state would restore, the
//! game would continue, and the divergence would surface somewhere else
//! entirely.

use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid, Tick, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene, Value};
use dimetric_sim::tween::Tweens;
use dimetric_sim::SimState;
use serde::{Deserialize, Serialize};

/// What the header must say.
pub const SAVE_FORMAT: &str = "dimetric-save";
/// The format's version. Bumped when the shape below changes incompatibly.
pub const SAVE_VERSION: u32 = 1;

/// The scene, written beside the state as its own file.
pub const SCENE_FILE: &str = "scene.dim";
/// Everything that is not the tree.
pub const STATE_FILE: &str = "state.toml";

/// A run, as it goes to disk.
///
/// Everything here is simulation state. Anything that is *not* — the sound
/// list, the log lines, the broadphase — is absent rather than skipped, which
/// is the same discipline those fields already follow inside `SimState`: a
/// field that has to be remembered-not-to-save is a field somebody saves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveFile {
    /// Always [`SAVE_FORMAT`].
    pub format: String,
    /// Always [`SAVE_VERSION`] for a save this build can read.
    pub version: u32,
    /// The engine that wrote it.
    pub engine: String,
    /// The project-relative scene the run was on, for diagnostics.
    pub scene: String,
    /// The run's seed.
    pub seed: u64,
    /// Ticks elapsed.
    pub tick: u64,
    /// How many spawns have happened.
    pub spawn_count: u64,
    /// Named RNG streams, at their current positions.
    pub rng: dimetric_core::RngStreams,
    /// What the running scene was handed when it was loaded.
    pub carry: Value,
    /// Per-node script variables.
    pub vars: Vec<(String, Vec<(String, Value)>)>,
    /// Per-body velocity.
    pub velocity: Vec<(String, Vec2Fx)>,
    /// Per-node animation playback.
    pub anim: Vec<(String, dimetric_sim::AnimState)>,
    /// Per-node cosmetic tweens.
    pub tweens: Vec<(String, Vec<dimetric_sim::tween::Tween>)>,
    /// Nodes that have already had `on_ready`.
    pub readied: Vec<String>,
    /// `Sound` nodes that have already started themselves.
    pub autoplayed: Vec<String>,
    /// Input for the tick before the one the save was taken on.
    pub previous_input: dimetric_sim::InputFrame,
    /// The UI canvas the run was laid out against.
    pub canvas: [i32; 2],
    /// The resolution the world was drawn at.
    ///
    /// Hashed, like the canvas, because a script unprojects a click through
    /// it — so a run restored at a different resolution is a different run.
    /// Defaulted rather than required so a save written before this field
    /// existed loads exactly as it did then.
    #[serde(default = "default_resolution")]
    pub resolution: [u32; 2],
}

/// What `SimState::new` starts a run at, for a save that predates the field.
fn default_resolution() -> [u32; 2] {
    [480, 270]
}

impl SaveFile {
    /// Capture a running simulation.
    pub fn capture(state: &SimState, scene_path: &str) -> SaveFile {
        let uid = |u: &NodeUid| u.to_text();
        SaveFile {
            format: SAVE_FORMAT.to_string(),
            version: SAVE_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            scene: scene_path.to_string(),
            seed: state.rng.seed(),
            tick: state.tick.0,
            spawn_count: state.spawn_count,
            rng: state.rng.clone(),
            carry: state.carry.clone(),
            vars: state
                .vars
                .iter()
                .map(|(u, table)| {
                    (
                        uid(u),
                        table.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    )
                })
                .collect(),
            velocity: state.velocity.iter().map(|(u, v)| (uid(u), *v)).collect(),
            anim: state
                .anim
                .iter()
                .map(|(u, a)| (uid(u), a.clone()))
                .collect(),
            tweens: state
                .tweens
                .iter()
                .map(|(u, t)| (uid(u), t.clone()))
                .collect(),
            readied: state.readied.iter().map(uid).collect(),
            autoplayed: state.autoplayed.iter().map(uid).collect(),
            previous_input: state.previous_input.clone(),
            canvas: [state.canvas.width, state.canvas.height],
            resolution: [state.resolution.0, state.resolution.1],
        }
    }

    /// Rebuild simulation state from a save and the scene it names.
    ///
    /// The scene comes in separately because it was written as its own file;
    /// see the module note.
    pub fn restore(&self, scene: Scene) -> Result<SimState, Diagnostic> {
        if self.format != SAVE_FORMAT {
            return Err(Diagnostic::new(
                Code::SAVE_UNREADABLE,
                format!("not a Dimetric save: header says {:?}", self.format),
            ));
        }
        if self.version != SAVE_VERSION {
            return Err(Diagnostic::new(
                Code::SAVE_VERSION,
                format!(
                    "save is format version {} and this build reads {SAVE_VERSION}",
                    self.version
                ),
            ));
        }
        if self.engine != env!("CARGO_PKG_VERSION") {
            // A refusal rather than a warning, unlike an input log's engine
            // field. A log that replays wrong announces itself as a
            // divergence; a save that restores wrong just keeps playing.
            return Err(Diagnostic::new(
                Code::SAVE_VERSION,
                format!(
                    "save was written by engine {} and this is {}; \
                     a save does not survive a rules change",
                    self.engine,
                    env!("CARGO_PKG_VERSION")
                ),
            ));
        }

        let uid = |text: &str| {
            NodeUid::parse(text).map_err(|_| {
                Diagnostic::new(Code::SAVE_UNREADABLE, format!("{text:?} is not a node id"))
            })
        };

        let mut state = SimState::new(scene, self.seed);
        state.tick = Tick(self.tick);
        state.spawn_count = self.spawn_count;
        state.rng = self.rng.clone();
        state.carry = self.carry.clone();
        state.canvas = dimetric_scene::ui::Canvas {
            width: self.canvas[0],
            height: self.canvas[1],
        };
        state.resolution = (self.resolution[0], self.resolution[1]);
        state.previous_input = self.previous_input.clone();

        for (node, table) in &self.vars {
            let entry = state.vars.entry(uid(node)?).or_default();
            for (k, v) in table {
                entry.insert(k.clone(), v.clone());
            }
        }
        for (node, v) in &self.velocity {
            state.velocity.insert(uid(node)?, *v);
        }
        for (node, a) in &self.anim {
            state.anim.insert(uid(node)?, a.clone());
        }
        let mut tweens = Tweens::new();
        for (node, t) in &self.tweens {
            tweens.insert(uid(node)?, t.clone());
        }
        state.tweens = tweens;
        for node in &self.readied {
            state.readied.push(uid(node)?);
        }
        for node in &self.autoplayed {
            state.autoplayed.push(uid(node)?);
        }

        state.scene.update_world_transforms();
        Ok(state)
    }
}

/// Write a run to a directory.
///
/// The registry is the project's, not the built-ins: a game that declares its
/// own kinds would otherwise write a scene whose `Enemy` nodes no longer
/// validate, and the save would fail to load with a puzzle rather than an
/// error about the thing that actually went wrong.
pub fn save(
    dir: &Path,
    state: &SimState,
    scene_path: &str,
    registry: &KindRegistry,
) -> Result<(), Diagnostic> {
    let io = |e: std::io::Error, what: &str| {
        Diagnostic::new(Code::SAVE_UNREADABLE, format!("writing {what}: {e}"))
    };
    std::fs::create_dir_all(dir).map_err(|e| io(e, &dir.display().to_string()))?;

    // The tree through its own writer, so a save round-trips for exactly the
    // reason a scene file does (I2).
    let scene_text = dimetric_scene::write::to_canonical_text(&state.scene, registry, None);
    std::fs::write(dir.join(SCENE_FILE), scene_text).map_err(|e| io(e, SCENE_FILE))?;

    let file = SaveFile::capture(state, scene_path);
    let text = toml::to_string_pretty(&file)
        .map_err(|e| Diagnostic::new(Code::SAVE_UNREADABLE, format!("encoding the save: {e}")))?;
    std::fs::write(dir.join(STATE_FILE), text).map_err(|e| io(e, STATE_FILE))?;
    Ok(())
}

/// Read a run back.
pub fn load(dir: &Path, registry: &KindRegistry) -> Result<(SimState, Diagnostics), Diagnostic> {
    let read = |name: &str| -> Result<String, Diagnostic> {
        std::fs::read_to_string(dir.join(name)).map_err(|e| {
            Diagnostic::new(
                Code::SAVE_UNREADABLE,
                format!("reading {}: {e}", dir.join(name).display()),
            )
        })
    };

    let file: SaveFile = toml::from_str(&read(STATE_FILE)?).map_err(|e| {
        Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!("{STATE_FILE} is not a readable save: {e}"),
        )
    })?;

    let scene_text = read(SCENE_FILE)?;
    let parsed = dimetric_scene::parse(
        &scene_text,
        &dir.join(SCENE_FILE).display().to_string(),
        registry,
    );
    if parsed.diagnostics.has_errors() {
        return Err(Diagnostic::new(
            Code::SAVE_UNREADABLE,
            format!(
                "the scene in this save does not load: {}",
                parsed.diagnostics
            ),
        ));
    }
    let scene = parsed
        .doc
        .ok_or_else(|| Diagnostic::new(Code::SAVE_UNREADABLE, "the save has no scene"))?
        .scene;

    Ok((file.restore(scene)?, parsed.diagnostics))
}

/// Where a named save lives under a project.
pub fn save_dir(root: &Path, name: &str) -> PathBuf {
    root.join("saves").join(name)
}
