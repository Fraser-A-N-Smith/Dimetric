//! `dim-play` — open a project in a window and play it.
//!
//! The window, the surface and the event loop. Everything that decides what the
//! game does is in the library beside this file, so the interesting behaviour is
//! testable on a machine with no display and this is the part that cannot be.

use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use dimetric_host::Project;
use dimetric_player::{Action, Bindings, Clock, Held, Session, SessionConfig};
use dimetric_render::{Renderer, Target};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

/// Play a Dimetric project.
#[derive(Parser, Debug)]
#[command(name = "dim-play", version, about = "Play a Dimetric project")]
struct Args {
    /// Project directory. Defaults to the one beside the executable, which is
    /// where `dim build` puts a packaged game.
    project: Option<String>,
    /// Scene to open, relative to the project root. Defaults to the first one.
    #[arg(long)]
    scene: Option<String>,
    /// Run seed.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Write the session's input log here, so the run can be replayed.
    #[arg(long)]
    record: Option<std::path::PathBuf>,
    /// Internal resolution, as `WIDTHxHEIGHT`.
    #[arg(long)]
    internal: Option<String>,
    /// Window size, as `WIDTHxHEIGHT`.
    #[arg(long, default_value = "1440x810")]
    window: String,
    /// Play with no sound.
    ///
    /// The mixer still runs and still decides what would be heard; nothing
    /// reaches a device. What `--headless` is to drawing.
    #[arg(long)]
    mute: bool,
    /// Close after this many ticks. Without it the window stays open.
    ///
    /// For smoke tests and for recording a fixed-length session: a run that
    /// ends by itself writes its log, and one killed from outside does not.
    #[arg(long)]
    ticks: Option<u64>,
    /// Run `--ticks` ticks with no window and write the frame here.
    ///
    /// `dim frame capture` photographs a *project*; this photographs a
    /// *packaged game*, through the same session, the same clock and the same
    /// atlas rebuilds the runtime uses. Those differ, and the difference is
    /// not academic: a game whose scenes load each other is drawn by the
    /// runtime in a state no other tool can reach, so a screenshot of the
    /// project can be perfect while what a player double-clicks is not.
    #[arg(long)]
    capture: Option<std::path::PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut settings = dimetric_render::RenderSettings::default();
    if let Some(text) = &args.internal {
        settings.internal_resolution = parse_size(text)?;
    }
    let window_size = parse_size(&args.window)?;

    let root = match &args.project {
        Some(root) => std::path::PathBuf::from(root),
        None => beside_the_executable()?,
    };
    // A packaged game carries a manifest saying what it is; a project being
    // developed does not, and falls back to the first scene it has.
    let manifest = std::fs::read_to_string(root.join(dimetric_host::package::MANIFEST)).ok();
    let scene = match (&args.scene, &manifest) {
        (Some(scene), _) => scene.clone(),
        (None, Some(manifest)) => dimetric_host::package::boot_scene(manifest)
            .ok_or_else(|| "the game's manifest names no scene".to_string())?,
        (None, None) => first_scene(&root)?,
    };
    // An explicit --seed wins; otherwise the manifest's, otherwise zero.
    let seed = match (args.seed, &manifest) {
        (0, Some(manifest)) => dimetric_host::package::boot_seed(manifest).unwrap_or(0),
        (seed, _) => seed,
    };

    let mut project = Project::open(&root, 0);
    let project_settings = project.settings.clone();
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed,
            scene,
            record: args.record.clone(),
            settings,
            device: if args.mute {
                dimetric_audio::Device::Silent
            } else {
                dimetric_audio::Device::System
            },
            // A person playing is the one session that is real, so this is
            // the one that owns the profile.
            profile: Some(root.clone()),
        },
    )
    .map_err(|d| d.to_string())?;
    for d in session.diagnostics.iter() {
        eprintln!("{d}");
    }

    // The project's bindings, or the defaults when it declares none. Read
    // before the window opens so a typo in an action name is reported next to
    // the other startup diagnostics rather than the first time somebody
    // presses the key that does nothing.
    let (bindings, binding_problems) = Bindings::from_declared(
        &project_settings.bindings,
        project_settings.bindings_declared,
    );
    for d in &binding_problems {
        eprintln!("{d}");
    }

    // No window at all: step the session and draw one frame offscreen. This
    // runs before the event loop exists, so it works on a machine with no
    // display -- which is the point of having it.
    if let Some(png) = &args.capture {
        let viewport = session.settings().internal_resolution;

        // The renderer is built *before* the session is stepped, which is the
        // order a window does it in and the only order that makes this a
        // faithful photograph. Built afterwards it would pick up whatever
        // atlas the session had ended on, and so would have shown a correct
        // frame for a game that draws nothing but placeholder art -- which is
        // exactly what it did.
        let instance = dimetric_render::headless_instance();
        let mut renderer =
            dimetric_render::Renderer::new(&instance, None, session.atlas(), session.settings())
                .map_err(|e| format!("{e}; capturing needs a graphics adapter"))?;

        let ticks = args.ticks.unwrap_or(0);
        let mut held = Held::new();
        for _ in 0..ticks {
            let input = held.player_input();
            session.step(&mut project, input);
            if session.take_atlas_change() {
                renderer.set_atlas(session.atlas());
            }
        }
        for d in session.take_diagnostics().iter() {
            eprintln!("{d}");
        }

        let frame = session.frame(0.0, viewport);
        let target = dimetric_render::Capture::new(&renderer, viewport);
        let pixels = target
            .render(&mut renderer, &frame)
            .map_err(|e| e.to_string())?;
        dimetric_render::write_png(png, viewport.0, viewport.1, &pixels)
            .map_err(|e| e.to_string())?;
        println!(
            "captured tick {} to {} ({}x{})",
            session.tick(),
            png.display(),
            viewport.0,
            viewport.1
        );
        return Ok(());
    }

    let event_loop = EventLoop::new().map_err(|e| {
        format!("cannot open a display: {e}\n`dim-play` needs a desktop session; for a machine without one, `dim run --headless` simulates and `dim frame capture` draws.")
    })?;
    // Poll rather than Wait: a game draws whether or not anything was typed.
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        clock: Clock::new(session.tick_rate()),
        session,
        bindings,
        held: Held::new(),
        window: None,
        gpu: None,
        last: Instant::now(),
        cursor: (0.0, 0.0),
        pads: dimetric_player::pad::Pads::new(),
        project,
        window_size,
        paused: false,
        stop_after: args.ticks,
    };
    event_loop.run_app(&mut app)?;

    match app.session.save_profile() {
        Ok(Some(path)) => eprintln!("wrote {}", path.display()),
        Ok(None) => {}
        Err(d) => eprintln!("{d}"),
    }
    match app.session.finish() {
        Ok(Some(path)) => eprintln!("recorded {}", path.display()),
        Ok(None) => {}
        Err(d) => eprintln!("{d}"),
    }
    if app.clock.dropped() > 0 {
        // Silently running in slow motion is how "the game feels sluggish"
        // becomes a three-day investigation.
        eprintln!("dropped {} ticks keeping up", app.clock.dropped());
    }
    Ok(())
}

/// The window's surface and the renderer that draws into it.
struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    session: Session,
    clock: Clock,
    bindings: Bindings,
    held: Held,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    last: Instant,
    cursor: (f32, f32),
    /// Gamepads, polled between ticks. Present even without the feature, in
    /// which case it reports none.
    pads: dimetric_player::pad::Pads,
    /// Kept for the whole run because a script can ask for a different scene,
    /// and loading one needs the project's registry, prefabs and disk.
    project: Project,
    window_size: (u32, u32),
    paused: bool,
    stop_after: Option<u64>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Dimetric")
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.window_size.0,
                self.window_size.1,
            ));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("cannot open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        match self.start_gpu(window.clone()) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                eprintln!("{e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(false) => self.held.release_all(),
            WindowEvent::Resized(size) => self.resize((size.width, size.height)),
            WindowEvent::CursorMoved { position, .. } => {
                // I3-exempt: a cursor position is a render-boundary quantity.
                self.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let name = match button {
                    MouseButton::Left => "Mouse0",
                    MouseButton::Right => "Mouse1",
                    MouseButton::Middle => "Mouse2",
                    _ => return,
                };
                self.press(name, state == ElementState::Pressed);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.repeat {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                // winit spells key codes the way the W3C does — `KeyW`,
                // `ArrowUp`, `ShiftLeft` — which is the vocabulary the
                // bindings table is written in.
                self.press(&format!("{code:?}"), event.state == ElementState::Pressed);
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if self.stop_after.is_some_and(|n| self.session.tick() >= n) {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl App {
    fn start_gpu(&mut self, window: Arc<Window>) -> Result<Gpu, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot draw to this window: {e}"))?;
        let renderer = Renderer::new(
            &instance,
            Some(&surface),
            self.session.atlas(),
            self.session.settings(),
        )
        .map_err(|e| format!("{e}"))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(renderer.adapter(), size.width.max(1), size.height.max(1))
            .ok_or_else(|| "this adapter cannot present to this window".to_string())?;
        // Exactly what the composite writes. Anything else is a validation
        // error on the first frame rather than a colour that looks slightly
        // wrong, which is the good kind of failure.
        config.format = renderer.output_format();
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(renderer.device(), &config);
        Ok(Gpu {
            surface,
            config,
            renderer,
        })
    }

    fn resize(&mut self, size: (u32, u32)) {
        let Some(gpu) = &mut self.gpu else { return };
        gpu.config.width = size.0.max(1);
        gpu.config.height = size.1.max(1);
        gpu.surface.configure(gpu.renderer.device(), &gpu.config);
    }

    fn press(&mut self, key: &str, down: bool) {
        let Some(action) = self.bindings.action(key) else {
            return;
        };
        if action == Action::Pause && down {
            self.paused = !self.paused;
            self.held.release_all();
            return;
        }
        self.held.set(action, down);
    }

    /// One frame: spend the time that passed on ticks, then draw.
    fn frame(&mut self) {
        let now = Instant::now();
        let elapsed = now - self.last;
        self.last = now;

        let ticks = self.clock.advance(elapsed);
        if !self.paused {
            let viewport = self.session.settings().internal_resolution;
            for _ in 0..ticks {
                self.held.aim_at(
                    self.session
                        .aim_from_screen(self.cursor_in_world(), viewport),
                );
                // A pad, if there is one, and the cursor in canvas pixels.
                // Both read here, between ticks: a device polled inside a tick
                // would make the tick depend on when the poll happened.
                self.pads.poll(&mut self.held);
                self.held.point_at(dimetric_player::pad::window_to_canvas(
                    (self.cursor.0 as f64, self.cursor.1 as f64),
                    self.window_size(),
                    self.session.canvas(),
                ));
                let input = self.held.player_input();
                self.session.step(&mut self.project, input);
            }
            for d in self.session.take_diagnostics().iter() {
                eprintln!("{d}");
            }
            // A scene swap rebuilds the session's atlas; the renderer uploaded
            // one when it was built and would otherwise keep drawing from it
            // forever. This game's entry scene is a menu, which references no
            // art at all, so without this every floor after it was a field of
            // placeholder magenta -- for the whole session, while the
            // simulation underneath was entirely correct.
            if self.session.take_atlas_change() {
                if let Some(gpu) = &mut self.gpu {
                    gpu.renderer.set_atlas(self.session.atlas());
                }
            }
        }
        self.draw();
    }

    /// The window's size in physical pixels, for turning a cursor position
    /// into a canvas pixel.
    fn window_size(&self) -> (u32, u32) {
        match &self.gpu {
            Some(gpu) => (gpu.config.width, gpu.config.height),
            None => (1, 1),
        }
    }

    /// The cursor, in the internal resolution's pixels rather than the
    /// window's, because that is the space the camera is in.
    fn cursor_in_world(&self) -> (f32, f32) {
        let Some(gpu) = &self.gpu else {
            return self.cursor;
        };
        let settings = self.session.settings();
        let (scale, offset_x, offset_y) = settings.placement((gpu.config.width, gpu.config.height));
        (
            (self.cursor.0 - offset_x) / scale,
            (self.cursor.1 - offset_y) / scale,
        )
    }

    fn draw(&mut self) {
        let Some(gpu) = &mut self.gpu else { return };
        use wgpu::CurrentSurfaceTexture as Acquired;
        let frame = match gpu.surface.get_current_texture() {
            Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
            // Minimised, or between two sizes. Skip the frame; the game keeps
            // ticking and draws again when there is somewhere to draw.
            Acquired::Timeout | Acquired::Occluded => return,
            // The surface changed under us. Reconfigure and try next frame
            // rather than bringing the game down.
            Acquired::Outdated | Acquired::Lost | Acquired::Validation => {
                gpu.surface.configure(gpu.renderer.device(), &gpu.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let extracted = self.session.frame(
            self.clock.alpha(),
            self.session.settings().internal_resolution,
        );
        if let Err(e) = gpu.renderer.render(
            &extracted,
            &Target {
                view: &view,
                size: (gpu.config.width, gpu.config.height),
            },
        ) {
            eprintln!("{e}");
        }
        // wgpu 30 presents through the queue rather than the texture.
        gpu.renderer.queue().present(frame);
    }
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("{text:?} is not WIDTHxHEIGHT"))?;
    Ok((
        w.trim()
            .parse()
            .map_err(|_| format!("{text:?} is not WIDTHxHEIGHT"))?,
        h.trim()
            .parse()
            .map_err(|_| format!("{text:?} is not WIDTHxHEIGHT"))?,
    ))
}

/// Where a packaged game keeps its files: beside the executable.
fn beside_the_executable() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    exe.parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "cannot find myself".to_string())
}

/// The first `.dim` in the project root, so a one-scene project needs no flag.
fn first_scene(root: &std::path::Path) -> Result<String, String> {
    let mut candidates: Vec<String> = std::fs::read_dir(root)
        .map_err(|e| format!("cannot read {}: {e}", root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".dim"))
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| format!("no .dim scene in {}; pass --scene", root.display()))
}
