//! Text layout, which has to be the same everywhere.
//!
//! The font used here is built by hand rather than rasterised. That is the
//! point: layout is integer arithmetic over metrics that were baked at import,
//! so it can be tested without a font file, a rasteriser, or a GPU — and if it
//! ever stops being integer arithmetic these tests are what notices.

use dimetric_assets::font::{Font, Glyph};
use dimetric_render::text::{layout, Align};

/// A font with three glyphs of known size.
///
/// `A` is 4 wide and advances 5, `i` is 1 wide and advances 2, and a space has
/// no ink and advances 3. Deliberately uneven, so a bug that assumes a
/// monospaced font shows up.
fn font() -> Font {
    let mut glyphs = std::collections::BTreeMap::new();
    glyphs.insert(
        'A',
        Glyph {
            x: 0,
            y: 0,
            width: 4,
            height: 6,
            bearing_x: 0,
            bearing_y: -6,
            advance: 5,
        },
    );
    glyphs.insert(
        'i',
        Glyph {
            x: 8,
            y: 0,
            width: 1,
            height: 6,
            bearing_x: 1,
            bearing_y: -6,
            advance: 2,
        },
    );
    glyphs.insert(
        ' ',
        Glyph {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            bearing_x: 0,
            bearing_y: 0,
            advance: 3,
        },
    );
    Font {
        size: 8,
        line_height: 10,
        ascent: 6,
        descent: -2,
        glyphs,
    }
}

#[test]
fn measuring_sums_advances_not_widths() {
    // 5 + 2 + 5 = 12. Using bitmap widths would give 4 + 1 + 4 = 9 and every
    // letter would sit on top of the last.
    assert_eq!(font().measure("AiA"), 12);
    assert_eq!(font().measure(""), 0);
    assert_eq!(font().measure("A A"), 13);
}

#[test]
fn a_space_advances_the_pen_and_draws_nothing() {
    let laid = layout(&font(), "A A", Align::Left);
    assert_eq!(laid.glyphs.len(), 2, "the space produced a quad");
    assert_eq!(laid.glyphs[0].x, 0);
    assert_eq!(laid.glyphs[1].x, 8, "5 for the A, 3 for the space");
}

#[test]
fn bearing_places_a_glyph_relative_to_the_pen() {
    let laid = layout(&font(), "i", Align::Left);
    assert_eq!(laid.glyphs[0].x, 1, "bearing_x shifts it right of the pen");
    assert_eq!(laid.glyphs[0].y, 0, "ascent 6 plus bearing -6");
}

#[test]
fn alignment_moves_the_line_rather_than_the_glyphs_within_it() {
    let f = font();
    let left = layout(&f, "AA", Align::Left);
    let centre = layout(&f, "AA", Align::Center);
    let right = layout(&f, "AA", Align::Right);

    assert_eq!(left.glyphs[0].x, 0);
    assert_eq!(centre.glyphs[0].x, -5, "width 10, so it starts at -5");
    assert_eq!(right.glyphs[0].x, -10);

    // Spacing within the line is untouched by alignment.
    for laid in [&left, &centre, &right] {
        assert_eq!(laid.glyphs[1].x - laid.glyphs[0].x, 5);
    }
}

#[test]
fn a_newline_starts_a_line_and_the_block_is_as_wide_as_its_widest() {
    let laid = layout(&font(), "A\nAiA", Align::Left);
    assert_eq!(laid.width, 12, "the second line is the wider one");
    assert_eq!(laid.height, 20, "two lines at a line height of 10");

    let second_line_top = laid.glyphs[1].y;
    assert_eq!(second_line_top, 10, "the second line is one line down");
}

#[test]
fn an_unbaked_character_falls_back_rather_than_vanishing() {
    // Text that quietly disappears is worse than text that is visibly wrong.
    // This font has no `?` either, so there is nothing to draw — but the
    // fallback path is what stops a missing glyph deleting the rest.
    let mut f = font();
    f.glyphs.insert(
        '?',
        Glyph {
            x: 16,
            y: 0,
            width: 3,
            height: 6,
            bearing_x: 0,
            bearing_y: -6,
            advance: 4,
        },
    );
    let laid = layout(&f, "AzA", Align::Left);
    assert_eq!(laid.glyphs.len(), 3, "the unknown letter drew a fallback");
    assert_eq!(laid.glyphs[1].glyph.x, 16, "and the fallback was `?`");
}

#[test]
fn layout_is_the_same_every_time() {
    // The property that matters. Integer arithmetic over baked metrics has no
    // source of machine variation in it, and this is the guard on that.
    let f = font();
    let once = layout(&f, "AiA\n Ai", Align::Center);
    for _ in 0..64 {
        assert_eq!(layout(&f, "AiA\n Ai", Align::Center), once);
    }
}
