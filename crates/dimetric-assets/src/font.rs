//! Fonts: rasterised once at import, measured in integers for ever after.
//!
//! # Why a font is an import rather than a runtime concern
//!
//! A glyph bitmap is presentation — nothing in the simulation cares what the
//! letter A looks like. Its *metrics* are not, as soon as anything measures
//! text: a label that centres itself, a button that sizes to its caption, a
//! script asking how wide a string is. If those numbers came out of a
//! rasteriser at runtime they would be floats produced by hinting and
//! subpixel positioning, and two machines running different versions of a
//! font library would lay text out differently and diverge.
//!
//! So the rasteriser runs here, once, at a fixed pixel size, and what gets
//! cached is a page of glyphs plus a table of **integer** metrics. Integers
//! rather than fixed point because at a baked pixel size every metric already
//! is one: advance, bearing and line height are whole pixels, exactly
//! representable, and identical on every machine that reads the cache.
//!
//! The consequence worth knowing: a font is baked at one size. Drawing it at
//! another scales the bitmap, which is what a pixel-art engine wants anyway.
//! Import the same file twice at two sizes if you need two.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::image::Image;

/// The characters baked by default: printable ASCII.
///
/// Everything from space to `~`. A project needing more says so in the
/// `.meta` rather than the engine guessing, because every glyph baked is
/// atlas space spent whether or not the game ever draws it.
pub const DEFAULT_CHARSET: &str = concat!(
    " !\"#$%&'()*+,-./0123456789:;<=>?",
    "@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_",
    "`abcdefghijklmnopqrstuvwxyz{|}~",
);

/// Where one glyph sits on the page, and how to place it.
///
/// All integers, all pixels. See the module note on why.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Glyph {
    /// Left edge of the glyph's bitmap on the page.
    pub x: u32,
    /// Top edge of the glyph's bitmap on the page.
    pub y: u32,
    /// Bitmap width. Zero for a space, which has an advance and no ink.
    pub width: u32,
    /// Bitmap height.
    pub height: u32,
    /// Offset from the pen position to the bitmap's left edge.
    pub bearing_x: i32,
    /// Offset from the baseline down to the bitmap's top edge.
    pub bearing_y: i32,
    /// How far the pen moves after drawing this glyph.
    pub advance: i32,
}

/// A baked font: one page of glyphs and the numbers needed to place them.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Font {
    /// The size it was baked at, in pixels.
    pub size: u32,
    /// Distance from one baseline to the next.
    pub line_height: u32,
    /// Distance from the baseline to the top of the tallest glyph.
    pub ascent: i32,
    /// Distance from the baseline to the bottom of the lowest glyph.
    pub descent: i32,
    /// Every baked glyph, by character.
    pub glyphs: BTreeMap<char, Glyph>,
}

impl Font {
    /// The glyph for a character, or the one to draw in its place.
    ///
    /// A character that was not baked falls back to `?` and then to nothing.
    /// Returning nothing for an unbaked character would silently delete text,
    /// and text that quietly disappears is worse than text that is visibly
    /// wrong.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    /// Width of a single line, in pixels.
    ///
    /// Integer arithmetic throughout, so this is the same number everywhere
    /// and safe for anything that measures text.
    pub fn measure(&self, text: &str) -> i32 {
        text.chars()
            .filter(|c| *c != '\n')
            .filter_map(|c| self.glyph(c))
            .map(|g| g.advance)
            .sum()
    }

    /// Width of the widest line and the number of lines.
    pub fn measure_block(&self, text: &str) -> (i32, u32) {
        let mut widest = 0;
        let mut lines = 0;
        for line in text.split('\n') {
            widest = widest.max(self.measure(line));
            lines += 1;
        }
        (widest, lines)
    }
}

/// What went wrong baking a font.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    /// The file was not a font the rasteriser could read.
    #[error("not a readable font: {0}")]
    Unreadable(String),
    /// The requested size was zero or absurd.
    #[error("font size {0} is not usable; pick something between 4 and 512")]
    Size(u32),
}

/// Rasterise a font file into a page and a metric table.
///
/// The page is a single-channel coverage map widened to RGBA white, so a
/// label's `modulate` colours it and one page serves every colour of text in
/// the game.
pub fn bake(bytes: &[u8], size: u32, charset: &str) -> Result<(Font, Image), FontError> {
    if !(4..=512).contains(&size) {
        return Err(FontError::Size(size));
    }
    let face = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .map_err(|e| FontError::Unreadable(e.to_string()))?;

    // Rasterise every character first, then place them. Sorted and
    // deduplicated, so the page is a function of the charset's *contents*
    // rather than the order someone happened to type them in — the same font
    // and the same characters bake to the same bytes.
    let mut wanted: Vec<char> = charset.chars().filter(|c| !c.is_control()).collect();
    wanted.sort_unstable();
    wanted.dedup();

    let mut raster: Vec<(char, fontdue::Metrics, Vec<u8>)> = Vec::with_capacity(wanted.len());
    for c in wanted {
        let (metrics, coverage) = face.rasterize(c, size as f32);
        raster.push((c, metrics, coverage));
    }

    // A simple shelf: glyphs are all about one size, so anything cleverer
    // would buy a few percent of a texture that is already small.
    let columns = (raster.len() as f64).sqrt().ceil().max(1.0) as u32;
    let cell_w = raster
        .iter()
        .map(|(_, m, _)| m.width as u32)
        .max()
        .unwrap_or(1)
        + 1;
    let cell_h = raster
        .iter()
        .map(|(_, m, _)| m.height as u32)
        .max()
        .unwrap_or(1)
        + 1;
    let page_w = (columns * cell_w).max(1);
    let rows = raster.len().div_ceil(columns as usize) as u32;
    let page_h = (rows * cell_h).max(1);

    let mut pixels = vec![0u8; (page_w * page_h * 4) as usize];
    let mut glyphs = BTreeMap::new();
    for (index, (c, metrics, coverage)) in raster.iter().enumerate() {
        let col = index as u32 % columns;
        let row = index as u32 / columns;
        let x = col * cell_w;
        let y = row * cell_h;

        for gy in 0..metrics.height {
            for gx in 0..metrics.width {
                let alpha = coverage[gy * metrics.width + gx];
                let px = x as usize + gx;
                let py = y as usize + gy;
                let at = (py * page_w as usize + px) * 4;
                // White with the glyph's coverage as alpha: the label's tint
                // does the colouring, so one page serves every colour.
                pixels[at] = 255;
                pixels[at + 1] = 255;
                pixels[at + 2] = 255;
                pixels[at + 3] = alpha;
            }
        }

        glyphs.insert(
            *c,
            Glyph {
                x,
                y,
                width: metrics.width as u32,
                height: metrics.height as u32,
                bearing_x: metrics.xmin,
                // fontdue measures from the baseline upward; everything here
                // measures downward from the top of the line, which is the
                // direction a screen goes.
                bearing_y: -(metrics.height as i32 + metrics.ymin),
                advance: metrics.advance_width.round() as i32,
            },
        );
    }

    let line = face.horizontal_line_metrics(size as f32);
    let (ascent, descent, gap) = match line {
        Some(m) => (
            m.ascent.round() as i32,
            m.descent.round() as i32,
            m.line_gap.round() as i32,
        ),
        None => (size as i32, 0, 0),
    };

    Ok((
        Font {
            size,
            line_height: (ascent - descent + gap).max(1) as u32,
            ascent,
            descent,
            glyphs,
        },
        Image {
            name: String::new(),
            width: page_w,
            height: page_h,
            pixels,
        },
    ))
}
