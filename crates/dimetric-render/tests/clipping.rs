//! Descendants of a clipping control are drawn only inside it.
//!
//! The companion to `dimetric-scene/tests/clipping.rs`, which holds the hit
//! test to the same rectangle. This is the drawing half: a quad entirely
//! outside is dropped, one straddling an edge keeps the part inside, and its
//! UV is cut by exactly the fraction that came off — so half a glyph is half a
//! glyph rather than a squashed whole one.
//!
//! Cutting the quad on the CPU rather than setting a scissor rectangle is
//! deliberate and `extract::clip_items` argues it: every UI quad is
//! axis-aligned and unrotated, so the output is pixel-identical, the whole
//! clipped list stays in one batch instead of costing a draw call per clip, the
//! GPU backend needs no change at all, and — the reason that matters here — it
//! can be checked on a machine with no graphics hardware.

use dimetric_core::Fx;
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{KindRegistry, Scene};

const SIDE: u32 = 16;

/// An icon, the solid pixel panels are filled with, and the engine's own font.
///
/// All three are needed: a `Panel` is a quad over the one white texel, and a
/// `Label` falls back to the built-in font, so an atlas missing either draws
/// nothing at all and a clipping test would pass for the wrong reason.
fn atlas() -> Atlas {
    let (font, page) = dimetric_assets::builtin_font::builtin();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert(
        dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
        font,
    );
    Atlas::pack(
        vec![
            Source {
                name: "sprites/icon".into(),
                width: SIDE,
                height: SIDE,
                pixels: [0xFF; 4].repeat((SIDE * SIDE) as usize),
            },
            Source {
                name: dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
                width: page.width,
                height: page.height,
                pixels: page.pixels,
            },
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
            dimetric_render::atlas::placeholder(8),
        ],
        512,
    )
    .with_fonts(fonts)
}

fn canvas() -> Canvas {
    Canvas {
        width: 320,
        height: 180,
    }
}

/// A 100x100 viewport at y 40 with a 40-tall icon scrolled 30 pixels out of
/// the top of it, so y 10..50 against a clip of y 40..140.
fn scene(clip: bool) -> Scene {
    let flag = match clip {
        true => "clip_children = true\n",
        false => "",
    };
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_view0000\"\nkind = \"Panel\"\nname = \"View\"\n\
         parent = \"n_root0000\"\n{flag}\
         offset_left = 0.0\noffset_top = 40.0\noffset_right = 100.0\noffset_bottom = 140.0\n\
         \n[[node]]\nid = \"n_icon0000\"\nkind = \"TextureRect\"\nname = \"Icon\"\n\
         parent = \"n_view0000\"\ntexture = \"asset:sprites/icon\"\n\
         offset_left = 0.0\noffset_top = -30.0\noffset_right = 100.0\noffset_bottom = 10.0\n"
    );
    let out = dimetric_scene::parse(&text, "c.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn ui_items(scene: &Scene) -> Vec<dimetric_render::batch::DrawItem> {
    let atlas = atlas();
    let camera = Camera::new((canvas().width as u32, canvas().height as u32));
    extract::extract_with_canvas(scene, &atlas, &camera, None, canvas()).ui
}

fn named(
    items: &[dimetric_render::batch::DrawItem],
    scene: &Scene,
    path: &str,
) -> Vec<dimetric_render::batch::DrawItem> {
    let id = scene.resolve_path(path).expect(path);
    let uid = scene.get(id).expect(path).uid;
    items.iter().copied().filter(|i| i.node == uid).collect()
}

/// A draw item's rectangle in canvas pixels: the centre the shader projects
/// plus the offset it adds afterwards.
fn rect_of(item: &dimetric_render::batch::DrawItem) -> (i32, i32, i32, i32) {
    let center = item.pos + item.screen_offset;
    let min = center - item.size / Fx::from_int(2);
    (
        min.x.to_int_trunc(),
        min.y.to_int_trunc(),
        item.size.x.to_int_trunc(),
        item.size.y.to_int_trunc(),
    )
}

#[test]
fn an_unclipped_control_draws_its_whole_rectangle() {
    // The control, and the shape every scene written before clipping has.
    let scene = scene(false);
    let items = ui_items(&scene);
    let icon = named(&items, &scene, "/Root/View/Icon");
    assert_eq!(icon.len(), 1);
    assert_eq!(
        rect_of(&icon[0]),
        (0, 10, 100, 40),
        "the icon's own rectangle, overflowing the viewport above it"
    );
}

#[test]
fn a_clip_trims_the_quad_to_the_part_inside() {
    let scene = scene(true);
    let items = ui_items(&scene);
    let icon = named(&items, &scene, "/Root/View/Icon");
    assert_eq!(icon.len(), 1, "it is partly inside, so it still draws");
    assert_eq!(
        rect_of(&icon[0]),
        (0, 40, 100, 10),
        "only y 40..50, the part inside the viewport"
    );
    // And the viewport itself is untouched: a control's own flag clips what is
    // inside it, never itself.
    let view = named(&items, &scene, "/Root/View");
    assert_eq!(rect_of(&view[0]), (0, 40, 100, 100));
}

#[test]
fn the_uv_is_cut_by_the_fraction_that_came_off() {
    // The part that makes it a clip rather than a squash. Three quarters of the
    // icon's height was trimmed from the top, so three quarters of its v range
    // has to go with it — otherwise the whole texture is drawn into a quarter
    // of the space and the player sees a squashed icon, not a clipped one.
    let whole = ui_items(&scene(false));
    let whole = named(&whole, &scene(false), "/Root/View/Icon");
    let cut = ui_items(&scene(true));
    let cut = named(&cut, &scene(true), "/Root/View/Icon");

    let [u0, v0, u1, v1] = whole[0].uv;
    let [cu0, cv0, cu1, cv1] = cut[0].uv;
    assert_eq!((cu0, cu1), (u0, u1), "nothing was trimmed horizontally");
    assert_eq!(cv1, v1, "nor off the bottom");

    // 30 of 40 pixels came off the top.
    let expected = v0 + (v1 - v0) * 0.75;
    assert!(
        (cv0 - expected).abs() < 1e-6,
        "v_min should be three quarters along: {cv0} vs {expected}"
    );
}

#[test]
fn a_control_entirely_outside_its_clip_draws_nothing() {
    // Scrolled right out of the top. There is no quad left, so the item is
    // dropped rather than drawn with a zero extent — a degenerate quad is a
    // draw call that paints nothing, and the batcher should never see one.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_view0000\"\nkind = \"Panel\"\nname = \"View\"\n\
         parent = \"n_root0000\"\nclip_children = true\n\
         offset_left = 0.0\noffset_top = 40.0\noffset_right = 100.0\noffset_bottom = 140.0\n\
         \n[[node]]\nid = \"n_icon0000\"\nkind = \"TextureRect\"\nname = \"Icon\"\n\
         parent = \"n_view0000\"\ntexture = \"asset:sprites/icon\"\n\
         offset_left = 0.0\noffset_top = -100.0\noffset_right = 100.0\noffset_bottom = -60.0\n";
    let out = dimetric_scene::parse(text, "g.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;
    let items = ui_items(&scene);
    assert!(
        named(&items, &scene, "/Root/View/Icon").is_empty(),
        "nothing of it is inside"
    );
    assert_eq!(
        named(&items, &scene, "/Root/View").len(),
        1,
        "and the viewport is still drawn"
    );
}

#[test]
fn a_caption_is_clipped_with_its_panel_glyph_by_glyph() {
    // A label parented into the UI tree is drawn by the UI pass and is not a
    // control, so it is not in the layout — which is exactly why the clip map
    // considers every node and not only controls. A caption that overflowed
    // the button it belongs to is what a clip is for.
    //
    // The panel is 20 pixels wide and the caption is far wider, so some glyphs
    // are whole, one is cut and the rest are gone.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"Panel\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\nclip_children = true\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 20.0\noffset_bottom = 20.0\n\
         \n[[node]]\nid = \"n_cap00000\"\nkind = \"Label\"\nname = \"Caption\"\n\
         parent = \"n_card0000\"\ntext = \"MMMMMMMMMM\"\n";
    let out = dimetric_scene::parse(text, "l.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let clipped = out.doc.unwrap().scene;

    let wide = text.replace("offset_right = 20.0", "offset_right = 300.0");
    let out = dimetric_scene::parse(&wide, "l.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let roomy = out.doc.unwrap().scene;

    let narrow = named(&ui_items(&clipped), &clipped, "/Root/Card/Caption");
    let full = named(&ui_items(&roomy), &roomy, "/Root/Card/Caption");
    assert_eq!(full.len(), 10, "ten inked glyphs with room for them");
    assert!(
        narrow.len() < full.len(),
        "the caption overflows a 20-pixel card, so glyphs are dropped: {} of {}",
        narrow.len(),
        full.len()
    );
    assert!(!narrow.is_empty(), "and the ones inside survive");
    for glyph in &narrow {
        let (x, _, w, _) = rect_of(glyph);
        assert!(
            x >= 0 && x + w <= 20,
            "a glyph escaped the card: x {x} w {w}"
        );
    }
}
