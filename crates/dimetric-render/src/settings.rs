//! Project-level rendering settings.

use dimetric_scene::Color;
use serde::{Deserialize, Serialize};

/// How a project wants its frames produced.
///
/// The pixel-art path is a setting, not an assumption. A game that wants
/// smooth scrolling and a game that wants crunchy pixels need opposite
/// answers, and an engine that only serves one of them has quietly picked a
/// genre.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RenderSettings {
    /// Resolution the world is drawn at, before upscaling.
    ///
    /// Everything renders here and is then scaled to the output. Drawing at
    /// the output resolution and scaling sprites instead is what produces
    /// uneven pixel sizes across a scene.
    pub internal_resolution: (u32, u32),
    /// Scale the internal image to the largest whole multiple that fits.
    ///
    /// Upscaling only. A whole multiple is what gives a pixel-art frame square
    /// pixels when it is drawn larger than it was authored; below one there is
    /// no whole multiple to pick, so a window smaller than the game scales the
    /// frame down by the exact ratio whatever this says. See
    /// [`RenderSettings::placement`].
    pub integer_upscale: bool,
    /// Round the camera to whole pixels.
    pub pixel_snap: bool,
    /// Light level where nothing is lit.
    ///
    /// Opaque white means lights add nothing visible and the light pass is
    /// skipped entirely, which is the default because most scenes have no
    /// lights at all.
    pub ambient: Color,
}

impl Default for RenderSettings {
    fn default() -> RenderSettings {
        RenderSettings {
            internal_resolution: (480, 270),
            integer_upscale: true,
            pixel_snap: true,
            ambient: Color::WHITE,
        }
    }
}

impl RenderSettings {
    /// True when the light pass has anything to contribute.
    pub fn lighting_enabled(&self) -> bool {
        self.ambient != Color::WHITE
    }

    /// Where the internal image lands in an output of `output` pixels.
    ///
    /// Returns `(scale, offset_x, offset_y)`. With integer upscaling the image
    /// is centred and the remainder is left as a border, because a fractional
    /// scale is exactly what makes some pixels one screen-pixel wider than
    /// their neighbours.
    ///
    /// # Nothing is ever cropped
    ///
    /// Integer upscaling used to clamp the scale to 1, which is a whole
    /// multiple and fits nothing: a 1920×1080 game in a 1440×810 window was
    /// drawn at full size and centred, so 240 pixels came off each side and 135
    /// off the top and bottom. A menu button, a right-hand rail and half a
    /// bottom bar were outside the window, and a click aimed at any of them
    /// landed on nothing.
    ///
    /// The clamp was the wrong shape rather than the wrong number. Whole
    /// multiples exist to keep pixels square while scaling *up*; there is no
    /// whole multiple below one, so below one the choice is between a
    /// fractional scale and throwing part of the frame away. A frame scaled by
    /// 0.75 has soft edges; a frame with its edges outside the window has a
    /// button nobody can press.
    ///
    /// So `integer_upscale` applies when the frame fits, and under that the
    /// exact ratio is used. Everything a game draws is always on screen.
    ///
    /// Whatever this returns, both the composite pass and the cursor go through
    /// it — the renderer to place the frame, `dim-play` to turn a window
    /// position into the internal pixel the simulation hit-tests against. One
    /// function, so a click lands on what is drawn under it.
    pub fn placement(&self, output: (u32, u32)) -> (f32, f32, f32) {
        let (iw, ih) = self.internal_resolution;
        if iw == 0 || ih == 0 {
            return (1.0, 0.0, 0.0);
        }
        let fit = (output.0 as f32 / iw as f32).min(output.1 as f32 / ih as f32);
        let scale = match self.integer_upscale && fit >= 1.0 {
            true => fit.floor(),
            false => fit.max(f32::MIN_POSITIVE),
        };
        let drawn = (iw as f32 * scale, ih as f32 * scale);
        (
            scale,
            ((output.0 as f32 - drawn.0) * 0.5).floor(),
            ((output.1 as f32 - drawn.1) * 0.5).floor(),
        )
    }

    /// A point in the output turned into the internal pixel under it.
    ///
    /// The inverse of [`RenderSettings::placement`], and here beside it so the
    /// two cannot drift. A cursor position is in the window's pixels and the
    /// simulation hit-tests in the internal resolution's, so every click goes
    /// through this — which is what makes "a click lands on what is drawn under
    /// it" a property of one function rather than of two that agree today.
    ///
    /// The result is not clamped. A point on the letterbox is outside the frame
    /// and lands outside the internal rectangle, which is the honest answer:
    /// nothing is drawn there, so nothing should be hit.
    pub fn window_to_internal(&self, point: (f32, f32), output: (u32, u32)) -> (f32, f32) {
        let (scale, offset_x, offset_y) = self.placement(output);
        ((point.0 - offset_x) / scale, (point.1 - offset_y) / scale)
    }
}
