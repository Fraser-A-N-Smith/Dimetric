//! Turning a string into glyph quads.
//!
//! The arithmetic here is integer throughout, on metrics that were baked at
//! import. That is the whole reason [`dimetric_assets::font`] bakes them: a
//! layout computed from integers is the same layout on every machine, so
//! anything that depends on where text lands — a label that centres itself
//! today, a button that sizes to its caption tomorrow — cannot drift between
//! one player's machine and another's.
//!
//! Rasterisation, by contrast, is pure presentation. Nothing downstream of a
//! glyph's *appearance* reaches the simulation, which is why the bitmap can
//! come from a third-party rasteriser and the metrics cannot.

use dimetric_assets::font::{Font, Glyph};

/// How lines sit horizontally relative to the anchor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// Lines start at the anchor.
    #[default]
    Left,
    /// Lines are centred on the anchor.
    Center,
    /// Lines end at the anchor.
    Right,
}

impl Align {
    /// Parse the property value.
    pub fn parse(name: &str) -> Align {
        match name {
            "Center" => Align::Center,
            "Right" => Align::Right,
            _ => Align::Left,
        }
    }
}

/// One glyph, placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    /// The glyph's rectangle on the font page.
    pub glyph: Glyph,
    /// Left edge, in pixels from the layout origin.
    pub x: i32,
    /// Top edge, in pixels down from the layout origin.
    pub y: i32,
}

/// A laid-out string.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    /// Every glyph that has ink. Spaces advance the pen and produce nothing.
    pub glyphs: Vec<Placed>,
    /// Width of the widest line.
    pub width: i32,
    /// Total height, line height times line count.
    pub height: i32,
}

/// Lay out text, in integer pixels, relative to an origin at the top-left.
///
/// A glyph with no ink — a space, or anything the font did not bake — moves
/// the pen and produces nothing to draw, so a run of spaces costs no quads.
pub fn layout(font: &Font, text: &str, align: Align) -> Layout {
    let mut out = Layout::default();
    if font.line_height == 0 {
        return out;
    }
    let line_height = font.line_height as i32;

    for (row, line) in text.split('\n').enumerate() {
        let width = font.measure(line);
        out.width = out.width.max(width);
        let start = match align {
            Align::Left => 0,
            // Halving rounds towards zero, which is the same rounding on every
            // machine — the reason this is integer division and not a scalar.
            Align::Center => -width / 2,
            Align::Right => -width,
        };

        let mut pen = start;
        let top = row as i32 * line_height;
        for c in line.chars() {
            let Some(glyph) = font.glyph(c) else { continue };
            if glyph.width > 0 && glyph.height > 0 {
                out.glyphs.push(Placed {
                    glyph: *glyph,
                    x: pen + glyph.bearing_x,
                    y: top + font.ascent + glyph.bearing_y,
                });
            }
            pen += glyph.advance;
        }
    }

    out.height = text.split('\n').count() as i32 * line_height;
    out
}
