//! Drawing a project.
//!
//! The host owns the run loop, so it owns "run to tick N and draw what is
//! there". Loading the textures a scene needs lives here too, because it is a
//! project-level question rather than a rendering one.
//!
//! Textures come from the project's import cache, which is where decoding,
//! `.meta` settings and packing already happened. Anything a scene references
//! that the catalogue does not have falls back to reading a PNG straight out of
//! `assets/`, so a file dropped in a folder mid-session draws before the next
//! import runs.

use std::path::Path;

use dimetric_assets::sheet::Framed;
use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_render::atlas::{load_png, placeholder, solid};
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, Interpolation, Projection, RenderSettings,
    Renderer,
};
use dimetric_scene::{Color, Scene, Value};
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig, SimState};

use crate::project::Project;

/// Largest atlas this build packs into.
///
/// Well inside the 2048 that downlevel limits guarantee, so a project that
/// renders on a developer's machine also renders on a modest laptop.
const ATLAS_WIDTH: u32 = 2048;

/// Load every texture a scene refers to and pack them into one atlas.
///
/// A reference with no file behind it is a warning, not a failure: the
/// placeholder draws in its place, and a designer with one broken path should
/// still be able to look at the room.
pub fn build_atlas(project: &Project, scene: &Scene) -> (Atlas, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let mut sources = vec![
        Framed {
            image: placeholder(16),
            frames: 1,
        },
        Framed {
            image: solid(Color::WHITE),
            frames: 1,
        },
    ];

    let imported = project.imported();
    for name in dimetric_render::extract::required_assets(scene) {
        // The cache first. It holds animation strips as well as stills, and it
        // holds them already decoded — and it knows how many frames a strip
        // has, which is what lets the renderer slice one.
        if let Some((image, frames)) = imported.and_then(|i| cached_image(i, &name)) {
            let mut image = image.clone();
            image.name = name;
            sources.push(Framed { image, frames });
            continue;
        }
        let path = texture_path(&project.root, &name);
        match load_png(&path) {
            Ok(mut source) => {
                source.name = name;
                sources.push(Framed {
                    image: source,
                    frames: 1,
                });
            }
            Err(e) => diagnostics.push(
                Diagnostic::new(Code::ASSET_MISSING, e.to_string())
                    .with_field("asset", name)
                    .with_field("path", path.display().to_string())
                    .with_severity(dimetric_core::Severity::Warning),
            ),
        }
    }
    (Atlas::pack_framed(sources, ATLAS_WIDTH), diagnostics)
}

/// The pixels an imported asset contributes to the atlas, and its frame count.
fn cached_image<'a>(
    imported: &'a dimetric_assets::Imported,
    name: &str,
) -> Option<(&'a dimetric_assets::Image, u32)> {
    match imported.artifacts.get(name)? {
        dimetric_assets::Artifact::Image(image) => Some((image, 1)),
        dimetric_assets::Artifact::Animation {
            sheet, frame_count, ..
        } => Some((sheet, *frame_count)),
        _ => None,
    }
}

/// Where a texture named `sprites/hero` is expected to live.
fn texture_path(root: &Path, name: &str) -> std::path::PathBuf {
    if name.ends_with(".png") {
        root.join("assets").join(name)
    } else {
        root.join("assets").join(format!("{name}.png"))
    }
}

/// The camera a scene wants, from its first `Camera2D` marked current.
///
/// Falls back to a view centred on the origin, so a scene with no camera still
/// renders something rather than refusing to draw.
pub fn scene_camera(scene: &Scene, viewport: (u32, u32)) -> Camera {
    let mut camera = Camera::new(viewport);
    let chosen = scene
        .walk()
        .into_iter()
        .filter_map(|id| scene.get(id).map(|n| (id, n)))
        .filter(|(_, node)| node.kind == "Camera2D")
        .find(|(_, node)| {
            node.get("current")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });

    if let Some((id, node)) = chosen {
        camera.center = scene.world_of(id).map(|t| t.pos).unwrap_or_default();
        if let Some(zoom) = node.get("zoom").and_then(Value::as_scalar) {
            // I3-exempt: render boundary.
            camera.zoom = zoom.to_f32().max(0.01);
        }
        if node.get("projection").and_then(Value::as_str) == Some("Isometric") {
            camera.projection = Projection::Isometric;
        }
        camera.pixel_snap = node
            .get("pixel_snap")
            .and_then(Value::as_bool)
            .unwrap_or(true);
    }
    camera
}

/// What to capture.
pub struct CaptureRequest {
    /// Tick to run to before drawing.
    pub tick: u64,
    /// Run seed.
    pub seed: u64,
    /// Input to drive the run, if any.
    pub input: Option<InputLog>,
    /// Output size in pixels.
    pub size: (u32, u32),
    /// Rendering settings.
    pub settings: RenderSettings,
}

/// A captured frame.
pub struct CapturedFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA bytes.
    pub pixels: Vec<u8>,
    /// Draw calls the frame cost.
    pub draw_calls: usize,
    /// Sprites submitted.
    pub sprites: usize,
    /// What the adapter reported.
    pub adapter: String,
    /// Anything that went wrong but did not stop the capture.
    pub diagnostics: Diagnostics,
}

/// Run a project to a tick and draw the result offscreen.
pub fn capture(
    project: &mut Project,
    request: CaptureRequest,
) -> Result<CapturedFrame, Diagnostics> {
    // Import before drawing, so a capture always shows what is on disk rather
    // than whatever the cache held when the session started.
    project.import_assets();
    let (scene, mut diagnostics) = project.runtime_scene()?;
    diagnostics.extend(project.load_scripts());

    let mut host = LuaHost::new(SimConfig::default().tick_rate).map_err(one)?;
    for (path, source) in &project.scripts {
        if let Err(d) = host.load(path, source) {
            diagnostics.push(d);
        }
    }

    let (atlas, asset_diagnostics) = build_atlas(project, &scene);
    diagnostics.extend(asset_diagnostics);

    // With the project's clips, so an animated sprite shows the frame it would
    // be on rather than the one it started on.
    let mut sim = Sim::new(scene, request.seed, Box::new(host), SimConfig::default())
        .with_clips(project.clips());
    let log = request
        .input
        .unwrap_or_else(|| InputLog::new(request.seed, env!("CARGO_PKG_VERSION"), 1));

    // Keep the state one tick back, so the captured frame can be interpolated
    // the same way a live frame would be. Capturing at a whole tick means alpha
    // is zero and the previous state contributes nothing, but going through the
    // same path keeps capture and display honest about each other.
    let mut previous: Option<SimState> = None;
    for tick in 0..request.tick {
        previous = Some(sim.snapshot());
        sim.step(log.frame(tick));
    }
    diagnostics.extend(sim.take_diagnostics());

    let state = sim.state();
    let camera = scene_camera(&state.scene, request.settings.internal_resolution);
    let frame = extract(
        &state.scene,
        &atlas,
        &camera,
        previous.as_ref().map(|p| Interpolation {
            previous: &p.scene,
            alpha: 0.0,
        }),
    );

    let instance = headless_instance();
    let mut renderer = Renderer::new(&instance, None, &atlas, request.settings).map_err(|e| {
        Diagnostics(vec![Diagnostic::new(
            Code::NOT_IMPLEMENTED,
            format!("{e}; headless rendering needs a graphics adapter, and a software one such as Mesa's lavapipe is enough"),
        )])
    })?;
    let target = Capture::new(&renderer, request.size);
    let pixels = target
        .render(&mut renderer, &frame)
        .map_err(|e| Diagnostics(vec![Diagnostic::new(Code::COMMAND_REJECTED, e.to_string())]))?;

    Ok(CapturedFrame {
        width: target.size().0,
        height: target.size().1,
        pixels,
        draw_calls: frame.draw_calls(),
        sprites: frame.sprites.len(),
        adapter: renderer.adapter_info.name.clone(),
        diagnostics,
    })
}

/// Draw a scene offscreen, with no simulation at all.
///
/// What an editor viewport needs: the scene as it stands, including edits that
/// have not been saved. The simulation's own state goes through the same path
/// while playing, so what the editor shows in play mode is the state it is
/// actually running rather than a re-run from tick zero.
pub fn draw_scene(
    project: &Project,
    scene: &Scene,
    size: (u32, u32),
    settings: RenderSettings,
) -> Result<CapturedFrame, Diagnostics> {
    let (atlas, diagnostics) = build_atlas(project, scene);
    let camera = scene_camera(scene, settings.internal_resolution);
    let frame = extract(scene, &atlas, &camera, None);

    let instance = headless_instance();
    let mut renderer = Renderer::new(&instance, None, &atlas, settings).map_err(|e| {
        Diagnostics(vec![Diagnostic::new(
            Code::NOT_IMPLEMENTED,
            format!("{e}; headless rendering needs a graphics adapter, and a software one such as Mesa's lavapipe is enough"),
        )])
    })?;
    let target = Capture::new(&renderer, size);
    let pixels = target
        .render(&mut renderer, &frame)
        .map_err(|e| Diagnostics(vec![Diagnostic::new(Code::COMMAND_REJECTED, e.to_string())]))?;

    Ok(CapturedFrame {
        width: target.size().0,
        height: target.size().1,
        pixels,
        draw_calls: frame.draw_calls(),
        sprites: frame.sprites.len(),
        adapter: renderer.adapter_info.name.clone(),
        diagnostics,
    })
}

fn one(d: Diagnostic) -> Diagnostics {
    Diagnostics(vec![d])
}
