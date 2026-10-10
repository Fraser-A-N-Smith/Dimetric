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
use dimetric_sim::suspend::SuspendRequest;
use dimetric_sim::{InputFrame, InputLog, PlayerInput, Sim, SimState};
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
    /// The root to keep the one suspended run under, if this session may
    /// suspend and resume.
    ///
    /// `None` means `app.suspend()` and `app.resume()` do nothing and
    /// `app.suspended()` answers false. That is what a test wants and what a
    /// **replay** must have: a recorded session that read a real save would
    /// reproduce only on a machine that happened to have one, and a run that
    /// could write one could have the file it is being compared against
    /// replaced underneath it.
    ///
    /// Separate from [`SessionConfig::profile`] although the windowed runtime
    /// passes the same directory for both. A run and a profile are different
    /// kinds of thing — one is simulation state and one must never be — and
    /// `savefile` and `profile_store` are deliberately separate files on disk
    /// and in the source for that reason. One field covering both would be the
    /// first place that stopped being true.
    pub suspend: Option<PathBuf>,
    /// The day `app.today()` reports, as `YYYY-MM-DD` in UTC.
    ///
    /// `None` reads the real clock, which is what a player's session wants. A
    /// caller naming one is a test, a capture or a replay — anything that has
    /// to produce the same output twice, which a run whose date moved with the
    /// calendar could not. Whatever this resolves to is written into the
    /// recording, so a replay is told what the recording was told.
    pub date: Option<String>,
}

/// A running game.
pub struct Session {
    sim: Sim,
    /// The state one tick back, so a drawn frame can sit between two ticks.
    previous: Option<SimState>,
    atlas: Atlas,
    /// Set when a scene swap rebuilt the atlas, cleared when a host takes it.
    ///
    /// The renderer uploads the atlas once, so a host that never hears about
    /// this keeps drawing the entry scene's art -- which for a game that
    /// starts on a menu is no art at all.
    atlas_changed: bool,
    speaker: Speaker,
    settings: RenderSettings,
    log: InputLog,
    recording: bool,
    record_to: Option<std::path::PathBuf>,
    /// Where the profile came from and the table scripts are reading, kept
    /// together so `finish` cannot write one project's profile into another's
    /// directory.
    profile: Option<(PathBuf, Rc<RefCell<Profile>>)>,
    /// Where the one suspended run lives, when this session may write one.
    suspend_root: Option<PathBuf>,
    /// The project-relative scene the run is on now.
    ///
    /// Tracked rather than derived, because a save names the scene it was
    /// taken on and a script may have moved the run to a different floor since
    /// the session opened. A save that named the entry scene would restore the
    /// right tree — the tree is written out in full — and then report the
    /// wrong floor to anything that read the header, which is what decides
    /// whether a build can continue it.
    scene: String,
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
        let sim_config = project.sim_config();
        let mut host = project.script_host().map_err(|d| Diagnostics(vec![d]))?;
        // The verbs this project declared of its own, and anything wrong with
        // the declaration. Into the recording, because each takes a button bit
        // by its position in the list and a recording stores raw bits — a
        // replay has to be able to refuse a log recorded against a different
        // list rather than read bit 5 as the wrong verb.
        let (actions, action_problems) = project.declared_actions();
        diagnostics.extend(Diagnostics(action_problems));
        // What day it is, read once here and never again. A tick may not read
        // a clock (I5), and a daily run needs a date, so the date is an input
        // the host supplies — and it goes into the recording, because a title
        // screen offering today's run writes the date into a label and picks a
        // seed from it, and both of those are hashed.
        //
        // `config.date` when a caller named one, so a test and a captured
        // frame are reproducible; the real clock otherwise.
        let date = config
            .date
            .clone()
            .unwrap_or_else(dimetric_host::today::today);
        host.set_date(&date);

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

        // Whether there is a run to continue, before the first tick, so a
        // title screen's `on_ready` can already know. A save this build cannot
        // use reads as no save at all — a Continue row that fails when pressed
        // is worse than one that was never shown — and the reason is reported
        // rather than swallowed.
        let mut sim = sim;
        if let Some(root) = &config.suspend {
            let slot = dimetric_host::suspend::probe(root);
            if let dimetric_host::suspend::Slot::Stale(d) = &slot {
                diagnostics.push(d.clone());
            }
            sim.set_suspended(slot.ready());
        }

        Ok(Session {
            sim,
            previous: None,
            atlas,
            atlas_changed: false,
            speaker,
            settings: config.settings,
            log: InputLog::new(config.seed, env!("CARGO_PKG_VERSION"), 1)
                .with_actions(actions)
                .with_date(date),
            recording: config.record.is_some(),
            record_to: config.record,
            profile,
            suspend_root: config.suspend,
            scene: config.scene,
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

    /// Whether the atlas changed since this was last asked, clearing the flag.
    ///
    /// A host calls this after stepping and, when it is true, hands
    /// [`Session::atlas`] to `Renderer::set_atlas`. Skipping it leaves the GPU
    /// holding the art of whichever scene the session started on.
    pub fn take_atlas_change(&mut self) -> bool {
        std::mem::take(&mut self.atlas_changed)
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
        if let Some(loaded) = dimetric_host::scene_swap::apply_pending_load(
            project,
            &mut self.sim,
            &mut self.diagnostics,
        ) {
            self.scene = loaded;
            self.rebuild_for_new_tree(project);
        }

        // After the swap, so a run that changed floor and asked to be
        // suspended in the same tick writes the floor it ended on.
        self.apply_suspend_request(project);
    }

    /// Rebuild everything that was derived from the tree that just went away.
    fn rebuild_for_new_tree(&mut self, project: &mut Project) {
        let (atlas, diags) = build_atlas(project, &self.sim.state().scene);
        self.atlas = atlas;
        self.atlas_changed = true;
        self.diagnostics.extend(diags);
        // The interpolation source is a tree that no longer exists, so a frame
        // drawn against it would try to tween the old floor's nodes into the
        // new floor's.
        self.previous = None;
    }

    /// Honour whatever a script asked of the suspended-run slot.
    ///
    /// Between ticks, like a scene load and for the same reason: a tick that
    /// wrote its own state to disk, or replaced it with somebody else's, would
    /// stop being a pure function of the state it started from (I8).
    ///
    /// A session with no save root does nothing at all. That is the replay's
    /// behaviour and a test's, and it is why a recorded run cannot depend on a
    /// file beside it.
    fn apply_suspend_request(&mut self, project: &mut Project) {
        let Some(request) = self.sim.take_suspend_request() else {
            return;
        };
        let Some(root) = self.suspend_root.clone() else {
            // Asked for by a script in a session that has nowhere to put one.
            // A warning rather than silence: `quit` has nothing to report when
            // a headless run ignores it, and this does — the game asked for
            // something and did not get it.
            self.diagnostics.push(Diagnostic::new(
                Code::SUSPEND_REFUSED,
                format!(
                    "a script asked to {} a run and this session keeps no suspended                      run; a replay and a headless run deliberately do not",
                    verb(request)
                ),
            ));
            return;
        };

        match request {
            SuspendRequest::Suspend => self.suspend_to(&root, project),
            SuspendRequest::Resume => self.resume_from(&root, project),
            SuspendRequest::Discard => {
                if let Err(d) = dimetric_host::suspend::discard(&root) {
                    self.diagnostics.push(d);
                }
                self.sim.set_suspended(false);
            }
        }
    }

    /// Write the run out. The quit `app.suspend()` carried with it is already
    /// set, so the runtime leaves its event loop after the frame that asked.
    fn suspend_to(&mut self, root: &std::path::Path, project: &mut Project) {
        let scene = self.scene.clone();
        let written =
            dimetric_host::suspend::write(root, &self.sim.state(), &scene, &project.registry);
        match written {
            Ok(_) => {
                self.sim.set_suspended(true);
                // The profile too, and in this order: a player who chose "save
                // and quit" means both, and the profile is the file they cannot
                // rebuild. Written here rather than left to the runtime's exit
                // path so the two land together even if the process is killed
                // between this tick and the next frame.
                if let Err(d) = self.save_profile() {
                    self.diagnostics.push(d);
                }
            }
            Err(d) => {
                // The game asked to stop and could not be saved. The quit
                // still stands — refusing to exit would trap the player — but
                // the reason is reported rather than lost.
                self.diagnostics.push(d);
                self.sim
                    .set_suspended(dimetric_host::suspend::probe(root).ready());
            }
        }
    }

    /// Replace the run with the suspended one, and consume it.
    ///
    /// The save is deleted only once the state is actually installed: a resume
    /// that failed half way should leave the run where it was rather than lose
    /// it. That is what makes this "exactly once per save" rather than "at most
    /// once, and nothing if anything goes wrong".
    fn resume_from(&mut self, root: &std::path::Path, project: &mut Project) {
        let (state, header, diags) = match dimetric_host::suspend::read(root, &project.registry) {
            Ok(triple) => triple,
            Err(d) => {
                self.diagnostics.push(d);
                // Whatever is there is not usable, so the game should stop
                // offering it.
                self.sim
                    .set_suspended(dimetric_host::suspend::probe(root).ready());
                return;
            }
        };
        self.diagnostics.extend(diags);

        // The project's notion of which scene is open, so a later save names
        // the floor the run is actually on and a `scene.request_load` resolves
        // against the same project state a fresh run would. The tree itself
        // comes out of the save in full, so a project whose scene file has
        // moved on still resumes — it is reported, not fatal.
        if let Err(d) = project.load_scene(&header.scene) {
            self.diagnostics.extend(d);
            self.diagnostics.push(Diagnostic::new(
                Code::SUSPEND_REFUSED,
                format!(
                    "resumed anyway: {:?} is no longer a scene in this project, so the                      run is the one in the save and not the one on disk",
                    header.scene
                ),
            ));
        }
        // Scripts, for the same reason a scene swap reloads them: the resumed
        // floor may reference scripts the scene this session opened on never
        // mentioned, and `require`'s cache is dropped on any reload.
        self.diagnostics.extend(project.load_scripts());

        let hash = state.hash();
        let tick = state.tick.0;
        let seed = state.rng.seed();
        self.sim.restore(state);
        self.scene = header.scene.clone();
        self.tick = tick;
        self.rebuild_for_new_tree(project);

        // A recording of this session is a recording of the resumed run, not
        // of the menu that preceded it. The frames before the resume belong to
        // a different state lineage and replaying them would reproduce
        // something nobody played, so they go — and the log names the save it
        // continued, by the hash that save restores to, which is what lets a
        // replay check it was given the right one.
        //
        // The save itself is kept beside the log, because the slot is about to
        // be emptied and a recording that names a run nobody has any more is a
        // recording of nothing. Written from the state rather than copied, so
        // the sidecar is a canonical save of exactly what was restored.
        if self.recording {
            self.log.frames.clear();
            self.log.resumed_from(hash, tick, seed);
            if let Some(log_path) = &self.record_to {
                let beside = dimetric_host::suspend::sidecar_save(log_path);
                if let Err(d) = dimetric_host::savefile::save(
                    &beside,
                    &self.sim.state(),
                    &header.scene,
                    &project.registry,
                ) {
                    self.diagnostics.push(d);
                }
            }
        }

        // Consumed. One run per save is the design: suspend writes it, resume
        // takes it, and there is no version left for a player to fall back to
        // after playing on and losing.
        if let Err(d) = dimetric_host::suspend::discard(root) {
            self.diagnostics.push(d);
        }
        self.sim.set_suspended(false);
    }

    /// Whether a run this build can continue is waiting.
    ///
    /// Presentation, like a profile value: the runtime may want to say so, and
    /// nothing in the simulation is allowed to depend on it except through
    /// `app.suspended()`, which is an input the host supplies.
    pub fn suspended(&self) -> bool {
        self.suspend_root
            .as_deref()
            .is_some_and(|root| dimetric_host::suspend::probe(root).ready())
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

    /// Apply the events the engine owns, and say how many it took.
    ///
    /// Separate from [`Session::drain_events`], and deliberately not folded
    /// into it. The drained list is the whole record of what the simulation
    /// said, and a host that mirrors a volume to an OS mixer or a Steam
    /// overlay has to find its kinds still in it — so this reads the list
    /// rather than consuming from it, and a kind the engine does not claim is
    /// left exactly where it was.
    ///
    /// The engine claims one kind today: `audio.bus_volume`, because the mixer
    /// is in here and nothing else can reach it. A game's Options screen owns
    /// the setting in its profile and says what it is; this is the other half.
    /// `docs/API.md` has the payload.
    ///
    /// Nothing here reaches the simulation. A headless run never calls it —
    /// there is no device to set — and a replay must not, for the same reason
    /// it ignores `app.quit()`.
    pub fn apply_events(&mut self, events: &[dimetric_sim::event::GameEvent]) -> usize {
        let taken = events
            .iter()
            .filter(|event| self.speaker.apply_event(event))
            .count();
        self.diagnostics.extend(std::mem::replace(
            &mut self.speaker.diagnostics,
            Diagnostics::new(),
        ));
        taken
    }

    /// The actions this project declared of its own, in order.
    ///
    /// What a binding table resolves a name against, and what a recording of
    /// this session carries. Read off the session rather than off the settings
    /// so there is one answer: the cap and the refusals have already been
    /// applied.
    pub fn declared_actions(&self) -> Vec<String> {
        self.log.actions.clone()
    }

    /// A bus's gain, for a host or a test that wants to see what an event did.
    pub fn bus_gain(&self, bus: dimetric_audio::Bus) -> f32 {
        self.speaker.bus_gain(bus)
    }

    /// Whether a script asked the application to quit, clearing the request.
    ///
    /// Read between ticks, like a scene load: the tick finishes over the state
    /// it started with and the host acts afterwards (I8). It is not simulation
    /// state and is never hashed — a run in which somebody chose Quit must hash
    /// the same as one where they closed the window, or a recorded session
    /// would replay differently depending on how it ended.
    ///
    /// Whoever is driving decides what quitting means: the windowed runtime
    /// leaves its event loop; a headless run and a replay do nothing, because
    /// there is nothing to leave and honouring it would let a script cut a
    /// recorded run short.
    pub fn quit_requested(&mut self) -> bool {
        self.sim.take_quit()
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
        dimetric_render::extract_at(
            &state.scene,
            &self.atlas,
            &camera,
            self.previous.as_ref().map(|p| Interpolation {
                previous: &p.scene,
                alpha: alpha.clamp(0.0, 1.0),
            }),
            self.sim.config().canvas,
            // Whole ticks. `alpha` moves a sprite between two positions, and
            // an animated tile deliberately does not move with it: a cycle
            // that advanced on the accumulator remainder would make a capture
            // depend on how busy the machine was.
            state.tick,
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

/// What a request would have done, for the diagnostic that says it did not.
fn verb(request: SuspendRequest) -> &'static str {
    match request {
        SuspendRequest::Suspend => "suspend",
        SuspendRequest::Resume => "resume",
        SuspendRequest::Discard => "discard",
    }
}
