//! Play-in-editor and the replay scrubber.
//!
//! # Why playing does not touch the scene
//!
//! Play mode runs the *resolved* scene — prefabs flattened, scripts loaded — in
//! a simulation of its own, and the edited scene is not part of it. Pressing
//! stop therefore costs nothing: there is nothing to undo, because nothing was
//! written. An editor that played by mutating the scene it was editing has to
//! remember to put everything back, and one day it does not.
//!
//! # Why the scrubber can go backwards
//!
//! A tick is a pure function of the state it starts from and the input it is
//! given, so any tick can be reached by starting from a snapshot and stepping.
//! Scrubbing backwards restores the nearest snapshot at or before the target
//! and steps forward from there — which is exactly what rollback netcode would
//! do, and is why the determinism work came before the editor.

use dimetric_core::Diagnostics;
use dimetric_host::Project;
use dimetric_sim::{InputFrame, InputLog, LuaHost, Sim, SimConfig, SimState};

/// How often a snapshot is kept while playing.
///
/// Every tick would be the fastest to scrub and the heaviest to hold; every
/// hundred would be cheap and slow. Sixty is a second of play at the default
/// rate, so the worst scrub costs a second of simulation.
pub const SNAPSHOT_INTERVAL: u64 = 60;

/// What the editor is doing with the simulation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Editing. Nothing is running.
    #[default]
    Editing,
    /// Running.
    Playing,
    /// Running, but not advancing.
    Paused,
}

/// Play-in-editor state.
#[derive(Default)]
pub struct Playback {
    /// What the editor is doing.
    mode: Mode,
    sim: Option<Sim>,
    /// Snapshots taken while playing, by the tick they were taken at.
    keyframes: Vec<(u64, SimState)>,
    /// Input recorded this session, so a session can be replayed or saved.
    log: Option<InputLog>,
    /// Furthest tick reached, which is what the scrubber's range is.
    furthest: u64,
}

impl Playback {
    /// What the editor is doing.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether a simulation exists at all.
    pub fn is_running(&self) -> bool {
        self.sim.is_some()
    }

    /// The running simulation, for a viewport to draw.
    pub fn sim(&self) -> Option<&Sim> {
        self.sim.as_ref()
    }

    /// The tick the simulation is on.
    pub fn tick(&self) -> u64 {
        self.sim.as_ref().map(|s| s.state().tick.0).unwrap_or(0)
    }

    /// The furthest tick reached, which is the scrubber's upper bound.
    pub fn furthest(&self) -> u64 {
        self.furthest
    }

    /// Start playing the project's current scene.
    ///
    /// The scene is resolved and copied. Nothing here writes back, so stopping
    /// needs no cleanup.
    pub fn play(&mut self, project: &mut Project) -> Result<(), Diagnostics> {
        if self.mode == Mode::Paused && self.sim.is_some() {
            self.mode = Mode::Playing;
            return Ok(());
        }
        project.import_assets();
        let clips = project.clips();
        let (scene, mut diagnostics) = project.runtime_scene()?;
        diagnostics.extend(project.load_scripts());

        let mut host =
            LuaHost::new(SimConfig::default().tick_rate).map_err(|d| Diagnostics(vec![d]))?;
        for d in host.load_all(
            project
                .scripts
                .iter()
                .map(|(p, s)| (p.as_str(), s.as_str())),
        ) {
            diagnostics.push(d);
        }

        let (templates, template_diagnostics) = project.templates();
        diagnostics.extend(template_diagnostics);
        let sim = Sim::new(scene, 0, Box::new(host), SimConfig::default())
            .with_clips(clips)
            .with_templates(templates);
        self.keyframes = vec![(0, sim.snapshot())];
        self.log = Some(InputLog::new(0, env!("CARGO_PKG_VERSION"), 1));
        self.furthest = 0;
        self.sim = Some(sim);
        self.mode = Mode::Playing;

        if diagnostics.has_errors() {
            return Err(diagnostics);
        }
        Ok(())
    }

    /// Stop, and throw the simulation away.
    pub fn stop(&mut self) {
        self.mode = Mode::Editing;
        self.sim = None;
        self.keyframes.clear();
        self.log = None;
        self.furthest = 0;
    }

    /// Stop advancing, without leaving play mode.
    pub fn pause(&mut self) {
        if self.mode == Mode::Playing {
            self.mode = Mode::Paused;
        }
    }

    /// Advance one tick, whether playing or paused.
    pub fn step(&mut self) {
        self.advance(InputFrame::default());
    }

    /// Advance one tick with real input.
    pub fn advance(&mut self, input: InputFrame) {
        let Some(sim) = self.sim.as_mut() else {
            return;
        };
        sim.step(input);
        let tick = sim.state().tick.0;
        self.furthest = self.furthest.max(tick);
        if tick % SNAPSHOT_INTERVAL == 0 && !self.keyframes.iter().any(|(t, _)| *t == tick) {
            self.keyframes.push((tick, sim.snapshot()));
            self.keyframes.sort_by_key(|(t, _)| *t);
        }
    }

    /// Move to a tick, forwards or backwards.
    ///
    /// Backwards means restoring the nearest snapshot at or before the target
    /// and stepping. Nothing is re-derived from wall-clock time, so scrubbing
    /// to a tick twice lands on the same state both times.
    pub fn scrub_to(&mut self, target: u64) {
        let Some(sim) = self.sim.as_mut() else {
            return;
        };
        let target = target.min(self.furthest);
        if target < sim.state().tick.0 {
            let Some((_, snapshot)) = self.keyframes.iter().rfind(|(t, _)| *t <= target) else {
                return;
            };
            sim.restore(snapshot.clone());
        }
        while sim.state().tick.0 < target {
            sim.step(InputFrame::default());
        }
        // Scrubbing is inspection, so it leaves the editor paused rather than
        // running on from wherever you dropped the handle.
        if self.mode == Mode::Playing {
            self.mode = Mode::Paused;
        }
    }

    /// Snapshots held, for a status line.
    pub fn keyframes(&self) -> usize {
        self.keyframes.len()
    }
}
