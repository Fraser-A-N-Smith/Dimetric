//! A `Label` can draw a dark ring under its glyphs.
//!
//! A label drew its glyphs once, in its `modulate`, and nothing else — so the
//! only way to put dark around a letter was to draw the whole label again. The
//! game drew each board label five times: four copies a step off, then the drop
//! shade and the ink. That is four extra nodes per float and per actor slot,
//! written every frame a float is up, to make a red "-4" readable over a pale
//! figure.
//!
//! The ordering is the part worth testing. Every quad of a label carries the
//! same sort key — the tie-break field *is* the node — so the outline's copies
//! and the ink are separated only by the order extraction produced, which
//! `batch::build` settles by sorting stably. Per-glyph interleaving would let
//! the second letter's outline draw over the first letter's ink.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{Color, KindRegistry, Scene};

fn atlas() -> Atlas {
    let (font, page) = dimetric_assets::builtin_font::builtin();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert(
        dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
        font,
    );
    Atlas::pack(
        vec![
            dimetric_render::atlas::Source {
                name: dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
                width: page.width,
                height: page.height,
                pixels: page.pixels,
            },
            dimetric_render::atlas::solid(Color::WHITE),
        ],
        512,
    )
    .with_fonts(fonts)
}

/// One world label with whatever extra lines are given.
fn scene(extra: &str) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_text0000\"\nkind = \"Label\"\nname = \"Float\"\n\
         parent = \"n_root0000\"\ntext = \"-4\"\n{extra}"
    );
    let out = dimetric_scene::parse(&text, "o.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut scene = out.doc.unwrap().scene;
    scene.update_world_transforms();
    scene
}

fn uid() -> NodeUid {
    NodeUid::parse("n_text0000").expect("an id")
}

/// Every quad the label produced, in draw order, as `(offset, colour)`.
fn quads(scene: &Scene, zoom: f32) -> Vec<(Vec2Fx, [u8; 4])> {
    let atlas = atlas();
    let mut camera = Camera::new((320, 180));
    camera.zoom = zoom;
    let mut frame = extract(scene, &atlas, &camera, None);
    // Through the batcher, because the draw order is what the batcher's sort
    // decides and testing the push order would test the wrong thing.
    let _ = dimetric_render::batch::build(&mut frame.sprites);
    frame
        .sprites
        .iter()
        .filter(|i| i.node == uid())
        .map(|i| (i.screen_offset, i.modulate))
        .collect()
}

#[test]
fn a_label_without_an_outline_draws_one_quad_a_glyph() {
    // The control. Every scene written before this, and every label that does
    // not ask, must be exactly as it was.
    for extra in ["", "outline_size = 0\n"] {
        let got = quads(&scene(extra), 1.0);
        assert_eq!(got.len(), 2, "{extra:?}: two inked glyphs in \"-4\"");
        for (_, colour) in &got {
            assert_eq!(*colour, [0xFF, 0xFF, 0xFF, 0xFF], "{extra:?}");
        }
    }
}

#[test]
fn an_outline_of_one_adds_eight_copies_a_glyph() {
    // The filled square rather than the eight compass points -- which at size
    // one are the same set, and at size two are not.
    let got = quads(&scene("outline_size = 1\n"), 1.0);
    assert_eq!(got.len(), 2 * 9, "eight copies and the ink, per glyph");
    let got2 = quads(&scene("outline_size = 2\n"), 1.0);
    assert_eq!(got2.len(), 2 * 25, "(2n+1)^2 - 1 copies, filled");
}

#[test]
fn every_copy_of_the_outline_draws_under_every_glyph_of_the_ink() {
    // The ordering defect this is really about. Interleaving per glyph would
    // put the second letter's outline over the first letter's ink wherever two
    // letters are close enough for their outlines to overlap.
    let got = quads(&scene("outline_size = 1\n"), 1.0);
    let ink = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let last_outline = got
        .iter()
        .rposition(|(_, colour)| *colour != ink)
        .expect("some outline");
    let first_ink = got
        .iter()
        .position(|(_, colour)| *colour == ink)
        .expect("some ink");
    assert!(
        last_outline < first_ink,
        "the outline has to be finished before the ink starts: \
         last outline at {last_outline}, first ink at {first_ink}"
    );
}

#[test]
fn the_outline_is_black_by_default_and_follows_the_labels_alpha() {
    // Absent is opaque black, which is what a dark outline is for.
    let got = quads(&scene("outline_size = 1\n"), 1.0);
    let outline = got
        .iter()
        .find(|(_, c)| *c != [0xFF, 0xFF, 0xFF, 0xFF])
        .expect("an outline quad")
        .1;
    assert_eq!(outline, [0x00, 0x00, 0x00, 0xFF]);

    // And a label that fades takes its outline with it, or a vanishing damage
    // number leaves a black ring behind.
    let faded = quads(&scene("outline_size = 1\nmodulate = \"#ffffff80\"\n"), 1.0);
    let outline = faded
        .iter()
        .find(|(_, c)| c[0] == 0x00)
        .expect("an outline quad")
        .1;
    assert_eq!(outline[3], 0x80, "the outline's alpha follows the ink's");

    // The hue stays authored, so a white flash on the text does not flash the
    // ring that makes it readable.
    let tinted = quads(
        &scene("outline_size = 1\nmodulate = \"#ff0000ff\"\noutline_color = \"#102030ff\"\n"),
        1.0,
    );
    let outline = tinted
        .iter()
        .find(|(_, c)| *c != [0xFF, 0x00, 0x00, 0xFF])
        .expect("an outline quad")
        .1;
    assert_eq!(outline, [0x10, 0x20, 0x30, 0xFF]);
}

#[test]
fn the_outline_steps_a_whole_pixel_around_each_glyph() {
    // Measured against the ink's own offset: the eight copies sit at the eight
    // whole offsets around it, no more and no less.
    let got = quads(&scene("outline_size = 1\n"), 1.0);
    let ink = [0xFFu8, 0xFF, 0xFF, 0xFF];
    // The first glyph's ink offset, and the outline offsets that surround it.
    let ink_at = got.iter().find(|(_, c)| *c == ink).expect("some ink").0;
    let mut around: Vec<(i32, i32)> = got
        .iter()
        .filter(|(_, c)| *c != ink)
        .map(|(at, _)| {
            (
                (at.x - ink_at.x).to_int_trunc(),
                (at.y - ink_at.y).to_int_trunc(),
            )
        })
        .filter(|(dx, dy)| dx.abs() <= 1 && dy.abs() <= 1)
        .collect();
    around.sort_unstable();
    around.dedup();
    assert_eq!(
        around,
        [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1)
        ],
        "the eight whole offsets around the glyph"
    );
}

#[test]
fn an_oversized_outline_is_clamped_rather_than_costing_a_frame() {
    // The sample count is quadratic, so an authored mistake -- a size meant as
    // a colour, a slider with no bound -- should cost a thick outline rather
    // than a hundred thousand quads.
    let got = quads(&scene("outline_size = 40\n"), 1.0);
    assert_eq!(got.len(), 2 * 81, "clamped to four: (2*4+1)^2 == 81");
}

#[test]
fn a_screen_size_labels_outline_stays_one_pixel_at_any_zoom() {
    // The outline's step is a screen offset like the advance and goes through
    // the same unmagnification, so it is in the font's own pixels whatever the
    // board's zoom -- which is the only way an outline keeps matching the
    // glyphs it surrounds.
    let on_screen = |extent: Fx, zoom: f32| extent.to_f64() as f32 * 2.0 * zoom / 320.0;
    let step_at = |zoom: f32, extra: &str| {
        let got = quads(&scene(extra), zoom);
        let ink = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let ink_at = got.iter().find(|(_, c)| *c == ink).expect("ink").0;
        let nearest = got
            .iter()
            .filter(|(_, c)| *c != ink)
            .map(|(at, _)| (at.x - ink_at.x).abs())
            .filter(|d| *d > Fx::ZERO)
            .min()
            .expect("an outline step");
        on_screen(nearest, zoom)
    };

    let flagged = "outline_size = 1\nscreen_size = true\n";
    let one = step_at(1.0, flagged);
    for zoom in [0.5, 2.0, 3.0] {
        let got = step_at(zoom, flagged);
        assert!(
            (got - one).abs() < 1e-5,
            "zoom {zoom} stepped {got} instead of {one}"
        );
    }

    // And without the flag it scales with the board, like the glyphs it is
    // drawn around.
    let plain = "outline_size = 1\n";
    let at_one = step_at(1.0, plain);
    let at_three = step_at(3.0, plain);
    assert!(
        (at_three - at_one * 3.0).abs() < 1e-5,
        "{at_three} should be three times {at_one}"
    );
}

#[test]
fn a_ui_label_gets_an_outline_too() {
    // The other walk. A label parented into the UI tree is drawn in canvas
    // pixels by a different pass through the same function.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_pane0000\"\nkind = \"Panel\"\nname = \"Pane\"\n\
         parent = \"n_root0000\"\noffset_right = 100.0\noffset_bottom = 40.0\n\
         \n[[node]]\nid = \"n_text0000\"\nkind = \"Label\"\nname = \"Caption\"\n\
         parent = \"n_pane0000\"\ntext = \"-4\"\noutline_size = 1\n";
    let out = dimetric_scene::parse(text, "u.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(
        &scene,
        &atlas,
        &camera,
        None,
        Canvas {
            width: 320,
            height: 180,
        },
    );
    let mine: Vec<_> = frame.ui.iter().filter(|i| i.node == uid()).collect();
    assert_eq!(mine.len(), 2 * 9, "eight copies and the ink, per glyph");
}
