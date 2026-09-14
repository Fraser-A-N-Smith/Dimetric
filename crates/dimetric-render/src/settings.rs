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
    pub fn placement(&self, output: (u32, u32)) -> (f32, f32, f32) {
        let (iw, ih) = self.internal_resolution;
        if iw == 0 || ih == 0 {
            return (1.0, 0.0, 0.0);
        }
        let fit = (output.0 as f32 / iw as f32).min(output.1 as f32 / ih as f32);
        let scale = if self.integer_upscale {
            fit.floor().max(1.0)
        } else {
            fit.max(f32::MIN_POSITIVE)
        };
        let drawn = (iw as f32 * scale, ih as f32 * scale);
        (
            scale,
            ((output.0 as f32 - drawn.0) * 0.5).floor(),
            ((output.1 as f32 - drawn.1) * 0.5).floor(),
        )
    }
}
