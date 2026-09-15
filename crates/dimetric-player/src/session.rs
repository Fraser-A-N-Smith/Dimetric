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
use dimetric_render::{extract, Atlas, Camera, Frame, Interpolation, RenderSettings};
use dimetric_sim::{InputFrame, InputLog, LuaHost, PlayerInput, Sim, SimConfig, SimState};

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

        let sim_config = SimConfig::default();
        let mut host = LuaHost::new(sim_config.tick_rate).map_err(|d| Diagnostics(vec![d]))?;
        for (path, source) in &project.scripts {
            if let Err(d) = host.load(path, source) {
                diagnostics.push(d);
            }
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

    /// The atlas the renderer was built against.
    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// Advance one tick on the given input.
    pub fn step(&mut self, input: PlayerInput) {
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
    }

    /// How many voices are sounding.
    pub fn voices(&self) -> usize {
        self.speaker.playing()
    }

    /// The frame to draw, `alpha` of the way from the last tick to the next.
    pub fn frame(&self, alpha: f32, viewport: (u32, u32)) -> Frame {
        let state = self.sim.state();
        let camera = self.camera(viewport);
        extract(
            &state.scene,
            &self.atlas,
            &camera,
            self.previous.as_ref().map(|p| Interpolation {
                previous: &p.scene,
                alpha: alpha.clamp(0.0, 1.0),
            }),
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
