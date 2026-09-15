//! Baking a font, and the properties the rest of the engine leans on.
//!
//! These need a real font file. Rather than commit one — a binary blob with
//! its own licence, in a repository whose whole habit is that a diff shows a
//! real change — the test finds one on the machine and says so plainly if it
//! cannot. It does not silently pass: a suite that quietly skips is a suite
//! that reports green having tested nothing.

use dimetric_assets::font::{bake, DEFAULT_CHARSET};

/// Somewhere to find a TTF, in the order they are usually installed.
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
    "C:/Windows/Fonts/arial.ttf",
];

fn a_font() -> Vec<u8> {
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            return bytes;
        }
    }
    panic!(
        "no system font found; looked in:\n  {}\nInstall one (on Debian: \
         `apt-get install fonts-dejavu-core`) — this test bakes a real font \
         rather than committing one.",
        CANDIDATES.join("\n  ")
    );
}

#[test]
fn baking_produces_a_page_and_a_glyph_for_every_character() {
    let (font, page) = bake(&a_font(), 16, DEFAULT_CHARSET).expect("bakes");

    assert_eq!(font.size, 16);
    assert!(font.line_height > 0, "a line height of zero draws nothing");
    assert!(
        font.ascent > 0,
        "ascent {} should be above the baseline",
        font.ascent
    );

    for c in DEFAULT_CHARSET.chars() {
        assert!(font.glyphs.contains_key(&c), "{c:?} was not baked");
    }
    assert!(page.width > 0 && page.height > 0);
    assert_eq!(
        page.pixels.len(),
        (page.width * page.height * 4) as usize,
        "the page is RGBA"
    );
}

#[test]
fn every_glyph_lies_inside_the_page() {
    // An off-page rect samples whatever was packed next to it, which shows up
    // as one letter wearing a piece of another.
    let (font, page) = bake(&a_font(), 16, DEFAULT_CHARSET).expect("bakes");
    for (c, g) in &font.glyphs {
        assert!(
            g.x + g.width <= page.width && g.y + g.height <= page.height,
            "{c:?} at {},{} sized {}x{} runs off a {}x{} page",
            g.x,
            g.y,
            g.width,
            g.height,
            page.width,
            page.height
        );
    }
}

#[test]
fn glyphs_do_not_overlap_each_other() {
    let (font, _) = bake(&a_font(), 16, DEFAULT_CHARSET).expect("bakes");
    let boxes: Vec<_> = font
        .glyphs
        .iter()
        .filter(|(_, g)| g.width > 0 && g.height > 0)
        .collect();
    for (i, (ac, a)) in boxes.iter().enumerate() {
        for (bc, b) in boxes.iter().skip(i + 1) {
            let apart = a.x + a.width <= b.x
                || b.x + b.width <= a.x
                || a.y + a.height <= b.y
                || b.y + b.height <= a.y;
            assert!(apart, "{ac:?} and {bc:?} overlap on the page");
        }
    }
}

#[test]
fn a_space_advances_and_has_no_ink() {
    let (font, _) = bake(&a_font(), 16, DEFAULT_CHARSET).expect("bakes");
    let space = font.glyphs[&' '];
    assert!(
        space.advance > 0,
        "a space with no advance runs words together"
    );
    assert_eq!((space.width, space.height), (0, 0), "a space has no bitmap");
}

#[test]
fn the_same_font_bakes_to_the_same_bytes() {
    // The cache is keyed by the source's content hash, so an import that
    // varied run to run would mean a golden image that fails at random.
    let bytes = a_font();
    let (first_font, first_page) = bake(&bytes, 16, DEFAULT_CHARSET).expect("bakes");
    let (again_font, again_page) = bake(&bytes, 16, DEFAULT_CHARSET).expect("bakes");
    assert_eq!(first_font, again_font);
    assert_eq!(first_page.pixels, again_page.pixels);
}

#[test]
fn the_charset_decides_the_page_rather_than_the_order_it_was_written() {
    // Sorted and deduplicated before rasterising, so two spellings of the same
    // set produce the same atlas and the same cache entry.
    let bytes = a_font();
    let (a, a_page) = bake(&bytes, 16, "abc").expect("bakes");
    let (b, b_page) = bake(&bytes, 16, "ccba").expect("bakes");
    assert_eq!(a, b);
    assert_eq!(a_page.pixels, b_page.pixels);
}

#[test]
fn an_unusable_size_is_refused_rather_than_guessed_at() {
    assert!(bake(&a_font(), 0, "a").is_err());
    assert!(bake(&a_font(), 9999, "a").is_err());
}

#[test]
fn something_that_is_not_a_font_says_so() {
    let err = bake(b"this is not a font", 16, "a").expect_err("refused");
    assert!(err.to_string().contains("not a readable font"), "{err}");
}
