//! The font the engine always has.
//!
//! These tests are mostly about legibility, which sounds unusual for a test
//! suite until you remember what the alternative was: a rasteriser guessing at
//! seven pixels, closing up the bowl of a `?` and filling in an `e`. The
//! glyphs are hand-drawn precisely so that "does it look like the letter" is
//! a question with a checkable answer.

use dimetric_assets::builtin_font::{builtin, BUILTIN_FONT, GAP, GLYPH_HEIGHT, GLYPH_WIDTH};

#[test]
fn every_printable_ascii_character_has_a_glyph() {
    // A gap here is a character that silently turns into a `?` on screen.
    let (font, _) = builtin();
    for c in 32u8..127 {
        let c = c as char;
        assert!(
            font.glyphs.contains_key(&c),
            "{c:?} ({}) has no glyph",
            c as u32
        );
    }
    assert_eq!(font.glyphs.len(), 95);
}

#[test]
fn a_space_advances_the_pen_and_draws_nothing() {
    // Reporting a width for a space would put an empty quad in every gap
    // between words, which the batcher would faithfully draw.
    let (font, _) = builtin();
    let space = font.glyphs[&' '];
    assert_eq!(space.width, 0);
    assert_eq!(space.height, 0);
    assert_eq!(space.advance, GLYPH_WIDTH as i32 + GAP);
}

#[test]
fn every_visible_character_actually_has_ink() {
    // A glyph that was authored as seven blank rows would measure correctly
    // and draw nothing, which is the worst way for a typo in the table to
    // hide.
    let (font, page) = builtin();
    for (c, glyph) in &font.glyphs {
        if *c == ' ' {
            continue;
        }
        let mut lit = 0;
        for y in 0..glyph.height {
            for x in 0..glyph.width {
                let px = ((glyph.y + y) * page.width + glyph.x + x) as usize;
                if page.pixels[px * 4 + 3] > 0 {
                    lit += 1;
                }
            }
        }
        assert!(lit > 0, "{c:?} is blank");
    }
}

/// Render one character as the rows that were authored for it.
fn art(c: char) -> Vec<String> {
    let (font, page) = builtin();
    let glyph = font.glyphs[&c];
    (0..GLYPH_HEIGHT)
        .map(|y| {
            (0..GLYPH_WIDTH)
                .map(|x| {
                    let px = ((glyph.y + y) * page.width + glyph.x + x) as usize;
                    if page.pixels[px * 4 + 3] > 0 {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_letters_look_like_the_letters() {
    // Spot checks against the shapes as drawn. A rasteriser cannot pass this;
    // that is the point of not using one.
    assert_eq!(
        art('A'),
        vec![".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#", "....."]
    );
    assert_eq!(
        art('0'),
        vec![".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.", "....."],
        "zero needs its slash or it is an O"
    );
    assert_eq!(
        art('?'),
        vec![".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..", "....."],
        "the bowl has to stay open at this size"
    );
}

#[test]
fn a_zero_is_distinguishable_from_a_capital_o() {
    // The pair that matters most for a debug overlay full of numbers.
    assert_ne!(art('0'), art('O'));
}

#[test]
fn characters_that_are_easily_confused_are_drawn_differently() {
    for (a, b) in [
        ('1', 'l'),
        ('1', 'I'),
        ('8', 'B'),
        ('5', 'S'),
        ('2', 'Z'),
        (',', '.'),
        (':', ';'),
    ] {
        assert_ne!(art(a), art(b), "{a:?} and {b:?} are the same shape");
    }
}

#[test]
fn a_g_is_drawn_with_its_tail_below_the_line() {
    // The bug the seven-row cell had: every lowercase body reached the bottom
    // of the glyph, so there was nowhere to descend to and a `g` sat on the
    // baseline like a capital.
    assert_eq!(
        art('g'),
        vec![".....", ".....", ".####", "#...#", "#...#", ".####", "....#", ".###."]
    );
}

#[test]
fn descenders_go_below_the_baseline_and_nothing_else_does() {
    // Row six is under the baseline. A `g` that did not reach it would sit on
    // the line like a capital, and an `n` that did would look like it fell.
    let has_bottom_row = |c: char| art(c)[GLYPH_HEIGHT as usize - 1].contains('#');
    for c in ['g', 'j', 'p', 'q', 'y'] {
        assert!(has_bottom_row(c), "{c:?} should descend");
    }
    for c in ['n', 'o', 'a', 'e'] {
        assert!(!has_bottom_row(c), "{c:?} should sit on the baseline");
    }
}

#[test]
fn the_metrics_place_text_where_the_page_says_it_is() {
    let (font, page) = builtin();
    // One row of glyphs, so the page is as wide as the alphabet and as tall as
    // one glyph.
    assert_eq!(page.height, GLYPH_HEIGHT);
    assert_eq!(page.width, GLYPH_WIDTH * 95);

    // Measuring is the property the whole font system exists for: an integer,
    // the same on every machine.
    assert_eq!(font.measure("AB"), 2 * (GLYPH_WIDTH as i32 + GAP));
    // A descender must not make a glyph advance further than its neighbours.
    assert_eq!(font.measure("gg"), font.measure("oo"));
    assert_eq!(font.measure(""), 0);
    assert!(font.line_height > GLYPH_HEIGHT, "lines would touch");
}

#[test]
fn the_page_is_white_with_the_shape_in_the_alpha() {
    // So a label's `modulate` tints the built-in font exactly as it tints an
    // imported one, rather than the built-in needing its own colour path.
    let (_, page) = builtin();
    for texel in page.pixels.chunks(4) {
        assert_eq!(&texel[..3], &[0xff, 0xff, 0xff], "colour is in the alpha");
    }
    assert_eq!(page.name, BUILTIN_FONT);
}
