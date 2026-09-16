//! The font the engine always has.
//!
//! Every other font comes from a project's assets, baked at import. This one
//! is compiled in, because "no font" should not mean "no text". A debug
//! overlay, a frame counter, or the message explaining that the real font
//! failed to load are all things worth being able to draw before a project has
//! any assets at all.
//!
//! # Why it is authored rather than generated
//!
//! The first attempt downsampled a real typeface to seven pixels. At that size
//! a rasteriser is guessing: the bowl of a `?` closed up, `e` filled in, and
//! tuning the thresholds glyph by glyph was slower than drawing them. Below
//! about ten pixels a typeface stops being an outline problem and becomes a
//! pixel one, which is why the bitmap fonts of the era were drawn by hand.
//!
//! So they are drawn here, as text, one row per line. That costs more source
//! than a hex blob and buys the thing this repository cares about: a change to
//! a letter shows up in a diff as that letter changing shape. A binary would
//! show as a wall of hex nobody can review.
//!
//! # Its licence is that there isn't one
//!
//! A TTF carries its own terms and cannot simply be committed. These glyphs are
//! five pixels wide and as close to the obvious rendering of each letter as
//! makes no difference — there is no expression here to own, which is the
//! second reason to draw them rather than to convert somebody else's.

use std::collections::BTreeMap;

use crate::font::{Font, Glyph};

/// Glyph bitmap width, in pixels.
pub const GLYPH_WIDTH: u32 = 5;
/// Glyph bitmap height, in pixels.
pub const GLYPH_HEIGHT: u32 = 8;
/// One pixel of space between one glyph and the next.
///
/// Part of the advance rather than of the bitmap, so a run of glyphs does not
/// carry a column of blank pixels into the atlas.
pub const GAP: i32 = 1;
/// The name a scene uses to ask for it.
///
/// Not a path, because it is not a file. A project that happens to import an
/// asset called `builtin` gets its own, since a project's assets are looked up
/// first — this is the fallback, not a reservation.
pub const BUILTIN_FONT: &str = "builtin";

/// Every glyph, five wide and eight tall, `#` for ink.
///
/// Rows 0 to 6 are the body and the baseline sits under row 6; row 7 is below
/// it, for the tails of `g`, `j`, `p`, `q`, `y` and a comma. Seven rows was the
/// first attempt and it had no room to descend: the lowercase bodies reached
/// the bottom of the cell, so a `g` sat on the line like a capital.
#[rustfmt::skip]
const GLYPHS: &[(char, [&str; GLYPH_HEIGHT as usize])] = &[
    (' ', [".....", ".....", ".....", ".....", ".....", ".....", ".....", "....."]),
    ('!', ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..", "....."]),
    ('"', [".#.#.", ".#.#.", ".....", ".....", ".....", ".....", ".....", "....."]),
    ('#', [".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#.", "....."]),
    ('$', ["..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#..", "....."]),
    ('%', ["##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##", "....."]),
    ('&', [".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#", "....."]),
    ('\'', ["..#..", "..#..", ".....", ".....", ".....", ".....", ".....", "....."]),
    ('(', ["...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#.", "....."]),
    (')', [".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#...", "....."]),
    ('*', [".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", ".....", "....."]),
    ('+', [".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....", "....."]),
    (',', [".....", ".....", ".....", ".....", ".....", ".....", "..##.", "..#.."]),
    ('-', [".....", ".....", ".....", "#####", ".....", ".....", ".....", "....."]),
    ('.', [".....", ".....", ".....", ".....", ".....", ".##..", ".##..", "....."]),
    ('/', ["....#", "...#.", "...#.", "..#..", ".#...", ".#...", "#....", "....."]),
    ('0', [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.", "....."]),
    ('1', ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.", "....."]),
    ('2', [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####", "....."]),
    ('3', ["#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.", "....."]),
    ('4', ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.", "....."]),
    ('5', ["#####", "#....", "####.", "....#", "....#", "#...#", ".###.", "....."]),
    ('6', ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.", "....."]),
    ('7', ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...", "....."]),
    ('8', [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.", "....."]),
    ('9', [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..", "....."]),
    (':', [".....", ".##..", ".##..", ".....", ".##..", ".##..", ".....", "....."]),
    (';', [".....", ".##..", ".##..", ".....", ".##..", ".##..", "..#..", ".#..."]),
    ('<', ["...#.", "..#..", ".#...", "#....", ".#...", "..#..", "...#.", "....."]),
    ('=', [".....", ".....", "#####", ".....", "#####", ".....", ".....", "....."]),
    ('>', [".#...", "..#..", "...#.", "....#", "...#.", "..#..", ".#...", "....."]),
    ('?', [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..", "....."]),
    ('@', [".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".###.", "....."]),
    ('A', [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#", "....."]),
    ('B', ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.", "....."]),
    ('C', [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.", "....."]),
    ('D', ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.", "....."]),
    ('E', ["#####", "#....", "#....", "###..", "#....", "#....", "#####", "....."]),
    ('F', ["#####", "#....", "#....", "###..", "#....", "#....", "#....", "....."]),
    ('G', [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###.", "....."]),
    ('H', ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#", "....."]),
    ('I', [".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.", "....."]),
    ('J', ["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..", "....."]),
    ('K', ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#", "....."]),
    ('L', ["#....", "#....", "#....", "#....", "#....", "#....", "#####", "....."]),
    ('M', ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#", "....."]),
    ('N', ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#", "....."]),
    ('O', [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.", "....."]),
    ('P', ["####.", "#...#", "#...#", "####.", "#....", "#....", "#....", "....."]),
    ('Q', [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#", "....."]),
    ('R', ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#", "....."]),
    ('S', [".####", "#....", "#....", ".###.", "....#", "....#", "####.", "....."]),
    ('T', ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "....."]),
    ('U', ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.", "....."]),
    ('V', ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..", "....."]),
    ('W', ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#", "....."]),
    ('X', ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#", "....."]),
    ('Y', ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..", "....."]),
    ('Z', ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####", "....."]),
    ('[', ["..##.", "..#..", "..#..", "..#..", "..#..", "..#..", "..##.", "....."]),
    ('\\', ["#....", ".#...", ".#...", "..#..", "...#.", "...#.", "....#", "....."]),
    (']', [".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".##..", "....."]),
    ('^', ["..#..", ".#.#.", "#...#", ".....", ".....", ".....", ".....", "....."]),
    ('_', [".....", ".....", ".....", ".....", ".....", ".....", ".....", "#####"]),
    ('`', [".#...", "..#..", ".....", ".....", ".....", ".....", ".....", "....."]),
    ('a', [".....", ".....", ".###.", "....#", ".####", "#...#", ".####", "....."]),
    ('b', ["#....", "#....", "####.", "#...#", "#...#", "#...#", "####.", "....."]),
    ('c', [".....", ".....", ".###.", "#....", "#....", "#....", ".###.", "....."]),
    ('d', ["....#", "....#", ".####", "#...#", "#...#", "#...#", ".####", "....."]),
    ('e', [".....", ".....", ".###.", "#...#", "#####", "#....", ".###.", "....."]),
    ('f', ["..##.", ".#..#", ".#...", "###..", ".#...", ".#...", ".#...", "....."]),
    ('g', [".....", ".....", ".####", "#...#", "#...#", ".####", "....#", ".###."]),
    ('h', ["#....", "#....", "####.", "#...#", "#...#", "#...#", "#...#", "....."]),
    ('i', ["..#..", ".....", ".##..", "..#..", "..#..", "..#..", ".###.", "....."]),
    ('j', ["...#.", ".....", "..##.", "...#.", "...#.", "...#.", "#..#.", ".##.."]),
    ('k', ["#....", "#....", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "....."]),
    ('l', [".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.", "....."]),
    ('m', [".....", ".....", "##.#.", "#.#.#", "#.#.#", "#...#", "#...#", "....."]),
    ('n', [".....", ".....", "####.", "#...#", "#...#", "#...#", "#...#", "....."]),
    ('o', [".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###.", "....."]),
    ('p', [".....", ".....", "####.", "#...#", "#...#", "####.", "#....", "#...."]),
    ('q', [".....", ".....", ".####", "#...#", "#...#", ".####", "....#", "....#"]),
    ('r', [".....", ".....", "#.##.", "##..#", "#....", "#....", "#....", "....."]),
    ('s', [".....", ".....", ".####", "#....", ".###.", "....#", "####.", "....."]),
    ('t', [".#...", ".#...", "###..", ".#...", ".#...", ".#..#", "..##.", "....."]),
    ('u', [".....", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#", "....."]),
    ('v', [".....", ".....", "#...#", "#...#", "#...#", ".#.#.", "..#..", "....."]),
    ('w', [".....", ".....", "#...#", "#...#", "#.#.#", "#.#.#", ".#.#.", "....."]),
    ('x', [".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "....."]),
    ('y', [".....", ".....", "#...#", "#...#", "#...#", ".####", "....#", ".###."]),
    ('z', [".....", ".....", "#####", "...#.", "..#..", ".#...", "#####", "....."]),
    ('{', ["...##", "..#..", "..#..", ".#...", "..#..", "..#..", "...##", "....."]),
    ('|', ["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "....."]),
    ('}', ["##...", "..#..", "..#..", "...#.", "..#..", "..#..", "##...", "....."]),
    ('~', [".....", ".#..#", "#.#.#", "#..#.", ".....", ".....", ".....", "....."]),
];

/// The built-in font and its glyph page, ready to pack like any other asset.
///
/// Deliberately the same pair the importer produces for a real font, so
/// everything downstream treats this as an ordinary baked font and no code
/// path exists that only the fallback takes.
pub fn builtin() -> (Font, crate::Image) {
    let (font, alpha, width, height) = bake();
    // White, with the coverage in the alpha channel: a label's `modulate`
    // then tints it like any other sprite, so the built-in font takes a colour
    // the same way an imported one does.
    let mut pixels = Vec::with_capacity(alpha.len() * 4);
    for a in alpha {
        pixels.extend_from_slice(&[0xff, 0xff, 0xff, a]);
    }
    (
        font,
        crate::Image {
            name: BUILTIN_FONT.to_string(),
            width,
            height,
            pixels,
        },
    )
}

/// The metrics and an 8-bit coverage bitmap, one row of glyphs.
fn bake() -> (Font, Vec<u8>, u32, u32) {
    let width = GLYPH_WIDTH * GLYPHS.len() as u32;
    let height = GLYPH_HEIGHT;
    let mut pixels = vec![0u8; (width * height) as usize];
    let mut glyphs = BTreeMap::new();

    for (index, (ch, rows)) in GLYPHS.iter().enumerate() {
        let x = GLYPH_WIDTH * index as u32;
        let mut ink = false;
        for (row, bits) in rows.iter().enumerate() {
            for (col, c) in bits.chars().enumerate() {
                if c == '#' {
                    ink = true;
                    let px = x + col as u32;
                    pixels[(row as u32 * width + px) as usize] = 0xff;
                }
            }
        }
        glyphs.insert(
            *ch,
            Glyph {
                x,
                y: 0,
                // A space has an advance and no ink. Reporting a width for it
                // would put an empty quad in every gap between words, which
                // the batcher would then faithfully draw.
                width: if ink { GLYPH_WIDTH } else { 0 },
                height: if ink { GLYPH_HEIGHT } else { 0 },
                bearing_x: 0,
                // The bitmap starts one row above the baseline's ascent, and
                // `bearing_y` is measured down from the baseline to the
                // bitmap's top: six rows sit above it.
                bearing_y: -(GLYPH_HEIGHT as i32 - 1),
                advance: GLYPH_WIDTH as i32 + GAP,
            },
        );
    }

    let font = Font {
        size: GLYPH_HEIGHT,
        // One blank row between lines, so a descender and the next line's
        // capitals do not touch.
        line_height: GLYPH_HEIGHT + 1,
        // Seven rows above the baseline, one below it.
        ascent: GLYPH_HEIGHT as i32 - 1,
        descent: 1,
        glyphs,
    };
    (font, pixels, width, height)
}
