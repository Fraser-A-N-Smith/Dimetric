//! Turning a canvas pixel into a world position, in fixed point.
//!
//! A tactics game can be driven entirely from the keyboard, with a cursor node
//! moved by the direction keys, and that is a legitimate genre design rather
//! than a dodge. Mouse targeting is a genre expectation, though, and getting
//! from `ui.pointer()` to a world cell means inverting the camera: the 2:1
//! dimetric shear, the camera's position, its zoom.
//!
//! Doing that in Lua would mean a second copy of the renderer's maths that
//! nothing keeps in sync, in a game where the projection *is* the view. When it
//! drifted the symptom would be clicks landing one cell off at certain camera
//! positions — a bug that reproduces for nobody.
//!
//! So it is here, in fixed point, off the same [`Projection`] the renderer
//! uses. A picked cell decides what the simulation does, so it is hashed like
//! any other decision and has to be exact.
//!
//! # The chain, and why the resolution is in the contract
//!
//! A canvas pixel is a fraction of the window. The rendered world fills the
//! same window, so that fraction lands at the same fraction of the *render
//! resolution* — which is not the canvas size, and need not be. From there it
//! is the ordinary camera inverse: shift to the view's centre, divide by zoom,
//! unproject, add the camera's world position.
//!
//! The consequence is that the render resolution has stopped being purely
//! presentation. A wider viewport shows more world at the same zoom, so two
//! players whose resolutions differed would pick different cells from the same
//! pointer. It is therefore part of the replay contract, declared in
//! `project.toml` beside the tick rate and the canvas.

use dimetric_core::{Fx, FxWide, Projection, Vec2Fx};
use dimetric_scene::{Scene, Value};

/// The view a scene's current camera describes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    /// Where the camera is looking, in world space.
    pub center: Vec2Fx,
    /// Magnification. Never zero.
    pub zoom: Fx,
    /// How world space maps to screen space.
    pub projection: Projection,
}

impl View {
    /// The zoom to actually divide by.
    ///
    /// `View` is a public struct, so the clamp cannot live only in
    /// [`view_of`]: a caller building one by hand — or a tween passing through
    /// zero — would divide by it. Guarded where the division is.
    fn safe_zoom(self) -> Fx {
        if self.zoom > Fx::ZERO {
            self.zoom
        } else {
            Fx::ONE
        }
    }
}

impl Default for View {
    fn default() -> View {
        View {
            center: Vec2Fx::ZERO,
            zoom: Fx::ONE,
            projection: Projection::TopDown,
        }
    }
}

/// Read the scene's current camera.
///
/// The first `Camera2D` marked current, in tree order; a scene with none gets
/// a view on the origin, which is what the renderer does too. Deliberately the
/// same rule as `dimetric_host::render::scene_camera` — a script that picked a
/// different camera than the one being drawn would be picking cells off a view
/// nobody is looking at.
pub fn view_of(scene: &Scene) -> View {
    let mut view = View::default();
    let chosen = scene
        .walk()
        .into_iter()
        .filter_map(|id| scene.get(id).map(|n| (id, n)))
        .filter(|(_, node)| node.base == "Camera2D")
        .find(|(_, node)| {
            node.get("current")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });

    if let Some((id, node)) = chosen {
        view.center = scene.world_of(id).map(|t| t.pos).unwrap_or_default();
        if let Some(zoom) = node.get("zoom").and_then(Value::as_scalar) {
            // A zero or negative zoom would divide by zero on the way back
            // out. Clamped rather than refused: a camera mid-tween through
            // zero should not stop the game.
            if zoom > Fx::ZERO {
                view.zoom = zoom;
            }
        }
        if node.get("projection").and_then(Value::as_str) == Some("Isometric") {
            view.projection = Projection::Isometric;
        }
    }
    view
}

/// Where a canvas pixel falls in the world.
pub fn to_world(
    view: View,
    canvas: dimetric_scene::ui::Canvas,
    resolution: (u32, u32),
    point: Vec2Fx,
) -> Vec2Fx {
    let (rw, rh) = (resolution.0.max(1) as i32, resolution.1.max(1) as i32);
    let (cw, ch) = (canvas.width.max(1), canvas.height.max(1));

    // Canvas pixel to render pixel, by the fraction of the view it sits at.
    //
    // Through `FxWide`, because the intermediate is the problem: a canvas x of
    // 320 times a width of 480 is 153,600, and `Fx` holds sixteen integer bits.
    // The engine's saturation guard catches that rather than letting it wrap,
    // and an accumulator is exactly what it asks for.
    let screen = Vec2Fx::new(rescale(point.x, rw, cw), rescale(point.y, rh, ch));
    // Relative to the middle of the view, which is where the camera looks.
    let centred = screen - Vec2Fx::new(Fx::from_int(rw) / 2, Fx::from_int(rh) / 2);
    let zoom = view.safe_zoom();
    let unzoomed = Vec2Fx::new(centred.x / zoom, centred.y / zoom);
    view.center + view.projection.unproject(unzoomed)
}

/// Where a world position falls on the canvas.
///
/// The inverse of [`to_world`], for a script placing a label over a monster's
/// head. Not exactly invertible at every input — the dimetric forward
/// direction halves a sum and discards the low bit — but exact to well within a
/// tile, which is what both directions are for.
pub fn to_canvas(
    view: View,
    canvas: dimetric_scene::ui::Canvas,
    resolution: (u32, u32),
    world: Vec2Fx,
) -> Vec2Fx {
    let (rw, rh) = (resolution.0.max(1) as i32, resolution.1.max(1) as i32);
    let (cw, ch) = (canvas.width.max(1), canvas.height.max(1));

    let projected = view.projection.project(world - view.center);
    let zoom = view.safe_zoom();
    let zoomed = Vec2Fx::new(projected.x * zoom, projected.y * zoom);
    let screen = zoomed + Vec2Fx::new(Fx::from_int(rw) / 2, Fx::from_int(rh) / 2);
    Vec2Fx::new(rescale(screen.x, cw, rw), rescale(screen.y, ch, rh))
}

/// `v * num / den`, with the multiply done wide so it cannot saturate.
///
/// Both factors are pixel counts and the product leaves `Fx` range for any
/// sensible pair of them, so this is the one place the arithmetic has to widen.
/// Saturating on the way back is the honest failure: a canvas point that far
/// outside the view is a clamp, not a wrap to the opposite corner.
fn rescale(v: Fx, num: i32, den: i32) -> Fx {
    let wide = v.wide() * FxWide::from_int(num as i64) / FxWide::from_int(den.max(1) as i64);
    wide.narrow_saturating()
}
