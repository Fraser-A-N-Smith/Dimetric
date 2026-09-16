//! Projection and the camera.
//!
//! The engine's third structural idea: **perspective is a camera matrix.** The
//! world is free-form 2D and the simulation never knows which projection is in
//! use. Top-down is the identity; isometric is a 2:1 shear applied at render
//! time. One flag on `Camera2D`, no engine fork, and a game can switch between
//! them without touching a line of gameplay code.
//!
//! Everything here is presentation. It reads simulation state and never writes
//! it (I7), which is why floats are allowed below this line and nowhere above.
use dimetric_core::Vec2Fx;

// The projection itself lives in `dimetric-core` now, because a script
// picking a world cell from a canvas pixel has to invert it, and a second copy
// of the maths in Lua would drift — the symptom being clicks landing one cell
// off at certain camera positions, which reproduces for nobody. Re-exported
// here so this module still reads as "projection and the camera".
pub use dimetric_core::Projection;

/// The float side of a projection: everything the drawing path needs.
///
/// An extension trait rather than inherent methods, because [`Projection`]
/// lives in `dimetric-core` now and that crate is covered by the I3 lint. An
/// `f32` there would need excusing line by line; here it needs no excuse at
/// all, because this is the render boundary and that is what the boundary is
/// for.
pub trait ProjectionRender {
    /// Map a world position to screen space.
    fn to_screen(self, world: Vec2Fx) -> (f32, f32);
    /// Map a screen position back to world space.
    ///
    /// Used by the editor for picking, where a float is fine because nothing
    /// downstream of it is hashed. A *script* picking a cell uses
    /// `Projection::unproject`, which is exact.
    fn to_world(self, screen: (f32, f32)) -> (f32, f32);
    /// The 2x2 part of the projection, row-major.
    fn matrix(self) -> [f32; 4];
}

impl ProjectionRender for Projection {
    fn to_screen(self, world: Vec2Fx) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (x, y) = world.to_f32_pair();
        match self {
            Projection::TopDown => (x, y),
            Projection::Isometric => (x - y, (x + y) * 0.5),
        }
    }

    fn to_world(self, screen: (f32, f32)) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (sx, sy) = screen;
        match self {
            Projection::TopDown => (sx, sy),
            Projection::Isometric => (sy + sx * 0.5, sy - sx * 0.5),
        }
    }

    fn matrix(self) -> [f32; 4] {
        match self {
            Projection::TopDown => [1.0, 0.0, 0.0, 1.0],
            Projection::Isometric => [1.0, -1.0, 0.5, 0.5],
        }
    }
}

/// A view onto the world.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    /// Centre of the view, in world space.
    pub center: Vec2Fx,
    /// Projection in use.
    pub projection: Projection,
    /// Scale factor. Larger zooms in.
    pub zoom: f32,
    /// Viewport size in pixels.
    pub viewport: (u32, u32),
    /// Round the view origin to whole pixels.
    ///
    /// A project setting, not an assumption: a pixel-art game needs it and a
    /// smooth-scrolling one does not want it.
    pub pixel_snap: bool,
}

impl Camera {
    /// A camera at the origin.
    pub fn new(viewport: (u32, u32)) -> Camera {
        Camera {
            center: Vec2Fx::ZERO,
            projection: Projection::TopDown,
            zoom: 1.0,
            viewport,
            pixel_snap: true,
        }
    }

    /// Map a world position to pixel coordinates in the viewport.
    pub fn world_to_screen(&self, world: Vec2Fx) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (wx, wy) = self.projection.to_screen(world);
        let (cx, cy) = self.projection.to_screen(self.center);
        let (mut ox, mut oy) = (cx, cy);
        if self.pixel_snap {
            // Snapping the camera rather than each sprite keeps sprites from
            // jittering relative to each other, which is the artefact that
            // makes unsnapped pixel art look wrong.
            ox = ox.round();
            oy = oy.round();
        }
        (
            (wx - ox) * self.zoom + self.viewport.0 as f32 * 0.5,
            (wy - oy) * self.zoom + self.viewport.1 as f32 * 0.5,
        )
    }

    /// Map a pixel position in the viewport back to world space.
    pub fn screen_to_world(&self, screen: (f32, f32)) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (cx, cy) = self.projection.to_screen(self.center);
        let x = (screen.0 - self.viewport.0 as f32 * 0.5) / self.zoom + cx;
        let y = (screen.1 - self.viewport.1 as f32 * 0.5) / self.zoom + cy;
        self.projection.to_world((x, y))
    }

    /// The view-projection matrix, column-major, as a GPU expects it.
    ///
    /// Orthographic with y running down the screen, centred on the camera.
    ///
    /// Note the transpose. [`Projection::matrix`] is row-major and WGSL columns
    /// are `(a, c)` and `(b, d)`, not `(a, b)` and `(c, d)`. Getting that
    /// backwards produces a picture that still looks plausible — the transpose
    /// of a 2:1 shear is another shear with the same determinant — which is why
    /// it lives here once, tested, instead of being written out at each call
    /// site.
    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        // I3-exempt: render boundary.
        let (width, height) = (self.viewport.0.max(1) as f32, self.viewport.1.max(1) as f32);
        let [a, b, c, d] = self.projection.matrix();
        let (mut cx, mut cy) = self.projection.to_screen(self.center);
        if self.pixel_snap {
            cx = cx.round();
            cy = cy.round();
        }
        let sx = 2.0 * self.zoom / width;
        let sy = -2.0 * self.zoom / height;
        [
            [a * sx, c * sy, 0.0, 0.0],
            [b * sx, d * sy, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-cx * sx, -cy * sy, 0.0, 1.0],
        ]
    }

    /// How many clip-space units one world unit of sprite size is worth.
    ///
    /// Separate from [`view_projection`](Camera::view_projection) because the
    /// projection applies to a sprite's *position* and not to its shape. Under
    /// the 2:1 shear a character drawn as an upright quad stays upright and
    /// only moves along the projected axes — which is how the genre works, and
    /// what the artwork is already drawn for. Shearing the quad as well turns
    /// every character into a parallelogram.
    pub fn pixel_scale(&self) -> [f32; 2] {
        // I3-exempt: render boundary.
        let (width, height) = (self.viewport.0.max(1) as f32, self.viewport.1.max(1) as f32);
        [2.0 * self.zoom / width, -2.0 * self.zoom / height]
    }

    /// The projection UI draws through: canvas pixels straight to clip space.
    ///
    /// No camera in it at all, which is the point — a health bar does not
    /// scroll when the player walks. The canvas is a fixed virtual resolution
    /// the project declares, and the composite scales it to the window, so
    /// nothing here depends on how big anybody's monitor is.
    pub fn canvas_projection(canvas: dimetric_scene::ui::Canvas) -> [[f32; 4]; 4] {
        // I3-exempt: render boundary.
        let (w, h) = (canvas.width.max(1) as f32, canvas.height.max(1) as f32);
        // x: 0..w becomes -1..1. y: 0..h becomes 1..-1, because canvas space
        // runs downward and clip space runs up.
        [
            [2.0 / w, 0.0, 0.0, 0.0],
            [0.0, -2.0 / h, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ]
    }

    /// Clip-space units per canvas pixel of quad size.
    pub fn canvas_pixel_scale(canvas: dimetric_scene::ui::Canvas) -> [f32; 2] {
        // I3-exempt: render boundary.
        [
            2.0 / canvas.width.max(1) as f32,
            -2.0 / canvas.height.max(1) as f32,
        ]
    }

    /// Interpolate between two simulation states for display.
    ///
    /// Essential for a 60 Hz simulation on a 144 Hz display, and strictly a
    /// read: the interpolated value is drawn and thrown away, never written
    /// back into a transform physics will read (I7).
    pub fn interpolate(previous: Vec2Fx, current: Vec2Fx, alpha: f32) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (px, py) = previous.to_f32_pair();
        let (cx, cy) = current.to_f32_pair();
        let a = alpha.clamp(0.0, 1.0);
        (px + (cx - px) * a, py + (cy - py) * a)
    }
}
