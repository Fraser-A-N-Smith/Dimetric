//! A game being played.
//!
//! Owns the project, the simulation and the atlas, and knows two verbs: advance
//! a tick, and extract a frame. Everything about it is the path `dim run` and
//! `dim frame capture` already take — deliberately, because a game that was
//! played and a game that was replayed have to be the same game or none of the
//! rest of this engine means anything.

use dimetric_audio::Device;
use dimetric_core::{Angle, Code, Diagnostic, Diagnostics, Fx, Vec2Fx};
use dimetric_host::render::{build_atlas, scene_camera};
use dimetric_host::speaker::Speaker;
use dimetric_host::Project;
use dimetric_render::{Atlas, Camera, Frame, Interpolation, RenderSettings};
use dimetric_sim::profile::Profile;
use dimetric_sim::{InputFrame, InputLog, LuaHost, PlayerInput, Sim, SimConfig, SimState};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// How to start a session.
pub struct SessionConfig {
    /// Run seed.
    pub seed: u64,
    /// Scene to open, relative to the project root.
    pub scene: String,
    /// Where to write the input log when the session ends, if anywhere.
    pub record: Option<std::path::PathBuf>,
    /// How to draw.
    pub settings: RenderSettings,
    /// Where sound goes. `Silent` still runs the mixer and makes no noise.
    pub device: Device,
    /// The project root to read and write `profile.toml` under, if this
    /// session is a real one.
    ///
    /// `None` means the session never touches the file: it starts from an
    /// empty profile and throws away whatever it accumulates. That is what a
    /// test wants, and it is what a **replay** must have — a replay that read
    /// somebody's unlocks would reproduce a recording only on the machine
    /// that made it, and a replay that wrote them could spend their Crowns.
    pub profile: Option<PathBuf>,
}

/// A running game.
pub struct Session {
    sim: Sim,
    /// The state one tick back, so a drawn frame can sit between two ticks.
    previous: Option<SimState>,
    atlas: Atlas,
    speaker: Speaker,
    settings: RenderSettings,
    log: InputLog,
    recording: bool,
    record_to: Option<std::path::PathBuf>,
    /// Where the profile came from and the table scripts are reading, kept
    /// together so `finish` cannot write one project's profile into another's
    /// directory.
    profile: Option<(PathBuf, Rc<RefCell<Profile>>)>,
    tick: u64,
    /// Everything that went wrong so far and did not stop the session.
    pub diagnostics: Diagnostics,
}

impl Session {
    /// Open a project and get it ready to play.
    pub fn open(project: &mut Project, config: SessionConfig) -> Result<Session, Diagnostics> {
        project.load_scene(&config.scene)?;
        project.import_assets();

        let (scene, mut diagnostics) = project.runtime_scene()?;
        diagnostics.extend(project.load_scripts());

        // The project's declared settings. A session that ran on the defaults
        // while the project asked for something else would produce recordings
        // the project itself could not replay.
        diagnostics.extend(project.settings_diagnostics.clone());
        let sim_config = SimConfig {
            tick_rate: project.settings.tick_rate,
            canvas: project.settings.canvas,
            resolution: project.settings.resolution,
        };
        let mut host = LuaHost::new(sim_config.tick_rate).map_err(|d| Diagnostics(vec![d]))?;
        host.set_fonts(project.fonts());

        // The profile, before the first tick, into the very table `profile.get`
        // reads. Taken here rather than after `Sim::new` because the host is
        // boxed into the simulation and the handle is the only way back to it.
        //
        // A profile that fails to parse stops the session. Silently starting
        // somebody from nothing because their unlocks would not load is the
        // worst available handling of the one file they cannot rebuild.
        let profile = match &config.profile {
            None => None,
            Some(root) => {
                let loaded =
                    dimetric_host::profile_store::load(root).map_err(|d| Diagnostics(vec![d]))?;
                let handle = host.profile_handle();
                *handle.borrow_mut() = loaded;
                Some((root.clone(), handle))
            }
        };
        for d in host.load_all(
            project
                .scripts
                .iter()
                .map(|(p, s)| (p.as_str(), s.as_str())),
        ) {
            diagnostics.push(d);
        }

        let (atlas, asset_diagnostics) = build_atlas(project, &scene);
        diagnostics.extend(asset_diagnostics);
        let (templates, template_diagnostics) = project.templates();
        diagnostics.extend(template_diagnostics);

        let sim = Sim::new(scene, config.seed, Box::new(host), sim_config)
            .with_clips(project.clips())
            .with_templates(templates);

        let mut speaker = Speaker::open(project, config.device);
        diagnostics.extend(std::mem::replace(
            &mut speaker.diagnostics,
            Diagnostics::new(),
        ));

        Ok(Session {
            sim,
            previous: None,
            atlas,
            speaker,
            settings: config.settings,
            log: InputLog::new(config.seed, env!("CARGO_PKG_VERSION"), 1),
            recording: config.record.is_some(),
            record_to: config.record,
            profile,
            tick: 0,
            diagnostics,
        })
    }

    /// The tick rate the simulation runs at.
    pub fn tick_rate(&self) -> u32 {
        self.sim.config().tick_rate
    }

    /// Ticks simulated so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The state hash, as `dim run` and `dim replay` report it.
    pub fn hash(&self) -> dimetric_core::StateHash {
        self.sim.hash()
    }

    /// Rendering settings, which decide the internal resolution.
    pub fn settings(&self) -> RenderSettings {
        self.settings
    }

    /// The canvas the UI is laid out against.
    ///
    /// The window needs it to turn a cursor position into the canvas pixel the
    /// simulation will hit-test, and getting a different one here than the
    /// simulation uses would put every click in the wrong place.
    pub fn canvas(&self) -> dimetric_scene::ui::Canvas {
        self.sim.config().canvas
    }

    /// The atlas the renderer was built against.
    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// Advance one tick on the given input, honouring a scene load if a
    /// script asked for one.
    ///
    /// Takes the project because loading needs the disk, and because a
    /// separate "settle" call a caller had to remember would mean
    /// `scene.request_load` silently doing nothing on the day somebody forgot
    /// it.
    pub fn step(&mut self, project: &mut Project, input: PlayerInput) {
        let frame = InputFrame {
            players: vec![input],
        };
        if self.recording {
            self.log.push(frame.clone());
        }
        self.previous = Some(self.sim.snapshot());
        self.sim.step(frame);
        self.tick += 1;
        self.diagnostics.extend(self.sim.take_diagnostics());

        // After the step, over the state it produced: the simulation says what
        // it would play and this is what decides whether anything is heard.
        self.speaker.tick(&self.sim.state());
        self.diagnostics.extend(std::mem::replace(
            &mut self.speaker.diagnostics,
            Diagnostics::new(),
        ));

        // Between ticks, never inside one (I8). A new floor brings its own
        // art, so the atlas is rebuilt — the old one holds the last floor's
        // tileset and nothing else would draw.
        if dimetric_host::scene_swap::apply_pending_load(
            project,
            &mut self.sim,
            &mut self.diagnostics,
        )
        .is_some()
        {
            let (atlas, diags) = build_atlas(project, &self.sim.state().scene);
            self.atlas = atlas;
            self.diagnostics.extend(diags);
            // The interpolation source is a tree that no longer exists, so a
            // frame drawn against it would try to tween the old floor's nodes
            // into the new floor's.
            self.previous = None;
        }
    }

    /// Take everything the game told the host since the last call.
    ///
    /// This is how a custom runtime hears about an achievement, a settings
    /// change or anything else platform-facing. The engine deliberately cannot
    /// reach a platform SDK — the sandbox has no `package`, no FFI and no `io`
    /// — so the simulation says what happened and the process around it
    /// decides what that means.
    ///
    /// Drained, so calling it twice in a tick gives the events once. A
    /// rollback re-runs ticks and re-emits, so a host that needs exactly-once
    /// deduplicates; Steam already does, and so can a game.
    pub fn drain_events(&mut self) -> Vec<dimetric_sim::event::GameEvent> {
        self.sim.take_events()
    }

    /// How many voices are sounding.
    pub fn voices(&self) -> usize {
        self.speaker.playing()
    }

    /// The frame to draw, `alpha` of the way from the last tick to the next.
    pub fn frame(&self, alpha: f32, viewport: (u32, u32)) -> Frame {
        let state = self.sim.state();
        let camera = self.camera(viewport);
        // The simulation's canvas, not the renderer's default: these two lay
        // the same UI out, and if they disagree the button a player sees is
        // not the button the tick decided they clicked.
        dimetric_render::extract_with_canvas(
            &state.scene,
            &self.atlas,
            &camera,
            self.previous.as_ref().map(|p| Interpolation {
                previous: &p.scene,
                alpha: alpha.clamp(0.0, 1.0),
            }),
            self.sim.config().canvas,
        )
    }

    /// The camera the scene wants.
    pub fn camera(&self, viewport: (u32, u32)) -> Camera {
        scene_camera(&self.sim.state().scene, viewport)
    }

    /// Turn a point on the screen into an aim angle from the player's camera.
    ///
    /// The angle is what a tick reads, so it has to be exact: an aim built from
    /// a float would write a value into the input log that the log could not
    /// represent, and the replay would refuse it. [`Angle::from_vector`] takes
    /// the fixed-point offset and lands on a binary angle.
    pub fn aim_from_screen(&self, screen: (f32, f32), viewport: (u32, u32)) -> Angle {
        // I3-exempt: a mouse position is a render-boundary quantity, and it
        // becomes fixed point here rather than later.
        let half = (viewport.0 as f32 / 2.0, viewport.1 as f32 / 2.0);
        let offset = Vec2Fx::new(
            Fx::from_f64_lossy(f64::from(screen.0 - half.0)),
            Fx::from_f64_lossy(f64::from(screen.1 - half.1)),
        );
        if offset.is_zero() {
            return Angle::ZERO;
        }
        Angle::from_vector(offset.x, offset.y)
    }

    /// Take everything reported since the last time this was called.
    ///
    /// A script error is per-tick and a game keeps running through one, so the
    /// player prints them as they arrive rather than at the end.
    pub fn take_diagnostics(&mut self) -> Diagnostics {
        std::mem::replace(&mut self.diagnostics, Diagnostics::new())
    }

    /// Write the profile back, if this session owns one and it changed.
    ///
    /// Only when it is dirty: a profile written on every exit would rewrite
    /// the file after a session that read it and did nothing, which turns
    /// "when did my save last change" into a question with no answer.
    pub fn save_profile(&mut self) -> Result<Option<PathBuf>, Diagnostic> {
        let Some((root, handle)) = &self.profile else {
            return Ok(None);
        };
        if !handle.borrow().is_dirty() {
            return Ok(None);
        }
        dimetric_host::profile_store::save(root, &handle.borrow())?;
        handle.borrow_mut().mark_clean();
        Ok(Some(dimetric_host::profile_store::profile_path(root)))
    }

    /// Write the recorded log, if this session was recording one.
    ///
    /// A session someone played is an input log like any other: the run
    /// reproduces under `dim replay`, and a bug found by playing arrives as
    /// evidence rather than as a description.
    pub fn finish(&self) -> Result<Option<std::path::PathBuf>, Diagnostic> {
        let Some(path) = &self.record_to else {
            return Ok(None);
        };
        std::fs::write(path, self.log.to_text()).map_err(|e| {
            Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("cannot write {}: {e}", path.display()),
            )
        })?;
        Ok(Some(path.clone()))
    }
}
