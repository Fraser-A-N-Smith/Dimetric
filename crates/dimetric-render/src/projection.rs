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

use dimetric_core::{Fx, Vec2Fx};
use serde::{Deserialize, Serialize};

/// How world space maps to screen space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Projection {
    /// World units are screen pixels. The identity.
    #[default]
    TopDown,
    /// The 2:1 shear the pixel-art genre actually ships.
    ///
    /// Despite what it is universally called, this is not isometric. True
    /// isometric projection puts 120° between all three axes; a 2:1 tile ratio
    /// is *dimetric*, where one axis foreshortens differently from the others.
    /// The engine is named for the projection it really uses; the enum keeps
    /// the name people search for.
    Isometric,
}

impl Projection {
    /// Map a world position to screen space.
    pub fn to_screen(self, world: Vec2Fx) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (x, y) = world.to_f32_pair();
        match self {
            Projection::TopDown => (x, y),
            Projection::Isometric => (x - y, (x + y) * 0.5),
        }
    }

    /// Map a screen position back to world space.
    ///
    /// The inverse exists so that picking — clicking a tile in the editor —
    /// is one function call rather than a second, subtly different, matrix
    /// somebody wrote from memory.
    pub fn to_world(self, screen: (f32, f32)) -> (f32, f32) {
        // I3-exempt: render boundary.
        let (sx, sy) = screen;
        match self {
            Projection::TopDown => (sx, sy),
            Projection::Isometric => (sy + sx * 0.5, sy - sx * 0.5),
        }
    }

    /// The 2x2 part of the projection, row-major.
    pub fn matrix(self) -> [f32; 4] {
        match self {
            Projection::TopDown => [1.0, 0.0, 0.0, 1.0],
            Projection::Isometric => [1.0, -1.0, 0.5, 0.5],
        }
    }

    /// Depth ordering for a world position under this projection.
    ///
    /// Y-sorting is the default for both projections: in top-down, something
    /// further down the screen is nearer; in dimetric, so is something further
    /// along both axes.
    pub fn depth_of(self, world: Vec2Fx) -> Fx {
        match self {
            Projection::TopDown => world.y,
            Projection::Isometric => world.x + world.y,
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
