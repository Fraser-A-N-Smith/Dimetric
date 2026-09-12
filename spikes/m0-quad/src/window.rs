//! The windowed path: winit, a surface, and a fixed-timestep loop.
//!
//! The loop uses the engine's real [`Accumulator`], because the point is to
//! find out whether it behaves against a real display rather than only in a
//! unit test. It is also the one place in the engine allowed to read a clock
//! (I5): what crosses into the simulation is a tick count and nothing else.

use std::time::Instant;

use dimetric_host::run::Accumulator;
use dimetric_render::Projection;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::gpu::{spike_camera, Renderer, FORMAT};

/// Open a window and draw until it is closed.
pub fn run(width: u32, height: u32, projection: Projection) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("no event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        size: (width, height),
        projection,
        state: None,
        accumulator: Accumulator::new(60),
        last_frame: Instant::now(),
        ticks: 0,
        error: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop failed: {e}"))?;
    match app.error {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// Live window state. Created on `resumed`, because on some platforms a window
/// cannot exist before then.
struct State {
    window: std::sync::Arc<Window>,
    surface: wgpu::Surface<'static>,
    renderer: Renderer,
    config: wgpu::SurfaceConfiguration,
}

struct App {
    size: (u32, u32),
    projection: Projection,
    state: Option<State>,
    accumulator: Accumulator,
    last_frame: Instant,
    ticks: u64,
    error: Option<String>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.create(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(message) => {
                self.error = Some(message);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    state.config.width = size.width;
                    state.config.height = size.height;
                    state
                        .surface
                        .configure(&state.renderer.device, &state.config);
                }
            }
            WindowEvent::RedrawRequested => {
                // Real time enters here and nowhere else. The accumulator turns
                // it into a whole number of fixed ticks, and that number is all
                // the simulation would ever see.
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_frame).as_secs_f64();
                self.last_frame = now;
                self.ticks += self.accumulator.advance(elapsed) as u64;

                if let Err(message) = draw(state, self.projection) {
                    self.error = Some(message);
                    event_loop.exit();
                    return;
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

impl App {
    fn create(&self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        let attributes = Window::default_attributes()
            .with_title("Dimetric M0 spike")
            .with_inner_size(winit::dpi::LogicalSize::new(self.size.0, self.size.1));
        let window = std::sync::Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("could not create a window: {e}"))?,
        );

        // The windowed path needs a display handle; the headless path
        // deliberately has none, which is why it works on a build runner.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(window.clone()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("could not create a surface: {e}"))?;
        let renderer = pollster::block_on(Renderer::new(&instance, Some(&surface)))?;
        eprintln!(
            "m0: {} via {:?}",
            renderer.adapter_info.name, renderer.adapter_info.backend
        );

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: FORMAT,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&renderer.device, &config);

        Ok(State {
            window,
            surface,
            renderer,
            config,
        })
    }
}

fn draw(state: &mut State, projection: Projection) -> Result<(), String> {
    use wgpu::CurrentSurfaceTexture as Acquired;

    let frame = match state.surface.get_current_texture() {
        Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
        // None of these is an error. A surface is lost or outdated on a resize
        // or a display change, occluded when the window is minimised, and times
        // out under load — all routine, all recovered by reconfiguring or by
        // skipping the frame. Treating them as failures would make the spike
        // quit the first time someone dragged the window to another monitor.
        Acquired::Lost | Acquired::Outdated => {
            state
                .surface
                .configure(&state.renderer.device, &state.config);
            return Ok(());
        }
        Acquired::Timeout | Acquired::Occluded => return Ok(()),
        Acquired::Validation => return Err("the surface raised a validation error".into()),
    };

    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    state.renderer.set_camera(&spike_camera(
        (state.config.width, state.config.height),
        projection,
    ));

    let mut encoder =
        state
            .renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("m0 frame"),
            });
    // The same call the headless path makes. If these ever diverge, a
    // screenshot stops being evidence.
    state.renderer.draw(&mut encoder, &view);
    state.renderer.queue.submit([encoder.finish()]);
    state.renderer.queue.present(frame);
    Ok(())
}
