//! Measuring text, inside the simulation.
//!
//! # Why this is not presentation
//!
//! Drawing a glyph is presentation. Knowing how wide a string *will be* is not,
//! the moment anything measures it: a tooltip sized to its own text, a button
//! that fits its caption, a label centred in a panel. Those decide a control's
//! rectangle, layout runs inside the tick, and a rectangle decides what a click
//! lands on — so a measurement is a simulation decision and has to be the same
//! on every machine.
//!
//! It already is. `dimetric_assets::font` bakes a font at import into a glyph
//! page and a table of **integer** metrics, precisely so that measuring is
//! integer arithmetic with no rasteriser in it. This is the sandbox reaching
//! that table; there is no new maths here, and that is the point.
//!
//! The fonts arrive the way animation clips do — handed to the simulation by
//! the host, because a run with fonts and one without would be different runs.

use std::collections::BTreeMap;

use dimetric_assets::font::Font;

/// Baked fonts, by the name a scene refers to them with.
pub type Fonts = BTreeMap<String, Font>;

/// How wide and tall a string will be, in pixels.
///
/// Newlines start a new line, as they do when it is drawn: width is the widest
/// line and height is the line height times the number of lines. A font that
/// is not loaded measures as nothing rather than as an error, for the same
/// reason a label with no font draws nothing — a missing font is a project
/// problem, not a reason to stop a tick.
pub fn measure(fonts: &Fonts, font: &str, text: &str) -> (i32, i32) {
    let Some(font) = fonts.get(font) else {
        return (0, 0);
    };
    let lines = text.split('\n');
    let mut width = 0;
    let mut count = 0;
    for line in lines {
        width = width.max(font.measure(line));
        count += 1;
    }
    (width, font.line_height as i32 * count)
}
