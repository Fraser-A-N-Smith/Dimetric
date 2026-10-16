//! A panel can have a frame instead of being a flat rectangle.
//!
//! `Panel` and `Button` drew one flat fill and `TextureRect` stretched the
//! whole texture over its rectangle, so a bordered window, a card or a tooltip
//! had no shape in the engine at all: a texture stretched to a dialog's size
//! stretches its border with it, and a one-pixel outline becomes a smear.
//!
//! `docs/ENGINE-GAPS.md` declined nine-patch panels once, as "layout
//! convenience and visual polish", alongside scroll and grid containers. The
//! containers stay declined. This one came back and is built, on the same test
//! the clipping entry was reconsidered under: it is draw-time only, has no
//! layout policy in it, and adds no node kind — nine quads out of the function
//! that already produced one.
//!
//! A game *can* approximate it today with nine `TextureRect` children, each
//! with its own `region` and anchors. What that costs is nine nodes per panel:
//! nine entries in the UI layout map, nine walks of the hit test, and nine
//! nodes' worth of properties in the scene's hash — paid by every panel in the
//! game. And it stops short of a `Button`, which is the test below that cannot
//! be worked around at all.

use dimetric_core::Fx;
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{KindRegistry, Scene};

/// A 12x12 frame texture: a 4-pixel border all round, so the margins and the
/// middle are all 4 and the arithmetic is easy to read.
const FRAME: u32 = 12;
const MARGIN: u32 = 4;

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "ui/frame".into(),
                width: FRAME,
                height: FRAME,
                pixels: [0xFF; 4].repeat((FRAME * FRAME) as usize),
            },
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
            dimetric_render::atlas::placeholder(8),
        ],
        128,
    )
}

fn canvas() -> Canvas {
    Canvas {
        width: 320,
        height: 180,
    }
}

fn parse(text: &str) -> Scene {
    let out = dimetric_scene::parse(text, "p.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// One control of `kind` filling `width` x `height`, with whatever extra
/// properties are given.
fn scene_with(kind: &str, width: i32, height: i32, extra: &str) -> Scene {
    parse(&format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"{kind}\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\n{extra}\
         offset_left = 0.0\noffset_top = 0.0\n\
         offset_right = {width}.0\noffset_bottom = {height}.0\n"
    ))
}

/// The quads belonging to the card, which is everything but the root's own.
fn card_quads(scene: &Scene) -> Vec<(i32, i32, i32, i32)> {
    let id = scene.resolve_path("/Root/Card").expect("the card");
    let uid = scene.get(id).expect("the card").uid;
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(scene, &atlas, &camera, None, canvas());
    frame
        .ui
        .iter()
        .filter(|i| i.node == uid)
        .map(|i| {
            let min = i.pos + i.screen_offset - i.size / Fx::from_int(2);
            (
                min.x.to_int_trunc(),
                min.y.to_int_trunc(),
                i.size.x.to_int_trunc(),
                i.size.y.to_int_trunc(),
            )
        })
        .collect()
}

#[test]
fn without_a_slice_a_texture_rect_still_stretches_the_whole_texture() {
    // The control. Every scene written before this did exactly one quad, and
    // must go on doing so.
    let scene = scene_with("TextureRect", 100, 60, "texture = \"asset:ui/frame\"\n");
    assert_eq!(card_quads(&scene), [(0, 0, 100, 60)]);
}

#[test]
fn a_slice_draws_nine_quads_with_the_corners_unscaled() {
    // 100x60 with 4-pixel margins: columns 4, 92, 4 and rows 4, 52, 4.
    let scene = scene_with(
        "TextureRect",
        100,
        60,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n",
    );
    assert_eq!(
        card_quads(&scene),
        [
            (0, 0, 4, 4),
            (4, 0, 92, 4),
            (96, 0, 4, 4),
            (0, 4, 4, 52),
            (4, 4, 92, 52),
            (96, 4, 4, 52),
            (0, 56, 4, 4),
            (4, 56, 92, 4),
            (96, 56, 4, 4),
        ],
        "corners 4x4, edges stretched on one axis, the middle on both"
    );
}

#[test]
fn each_cell_samples_its_own_corner_of_the_texture() {
    // The part that makes it a frame rather than nine copies. The top-left
    // quad has to sample the texture's top-left 4x4 and nothing else, or the
    // border art lands in the wrong place.
    let scene = scene_with(
        "TextureRect",
        100,
        60,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n",
    );
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let id = scene.resolve_path("/Root/Card").expect("the card");
    let uid = scene.get(id).expect("the card").uid;
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
    let cells: Vec<[f32; 4]> = frame
        .ui
        .iter()
        .filter(|i| i.node == uid)
        .map(|i| i.uv)
        .collect();
    assert_eq!(cells.len(), 9);

    let whole = atlas.region("ui/frame").expect("the frame is packed");
    let [u0, v0, u1, v1] = whole.uv;
    let third = |a: f32, b: f32, n: f32| a + (b - a) * (n * MARGIN as f32 / FRAME as f32);
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;

    // Top-left: the first third of both axes.
    assert!(
        close(cells[0][0], u0)
            && close(cells[0][1], v0)
            && close(cells[0][2], third(u0, u1, 1.0))
            && close(cells[0][3], third(v0, v1, 1.0)),
        "top-left samples the texture's top-left: {:?}",
        cells[0]
    );
    // Centre: the middle third of both.
    assert!(
        close(cells[4][0], third(u0, u1, 1.0))
            && close(cells[4][2], third(u0, u1, 2.0))
            && close(cells[4][1], third(v0, v1, 1.0))
            && close(cells[4][3], third(v0, v1, 2.0)),
        "the middle samples the middle: {:?}",
        cells[4]
    );
    // Bottom-right: the last third of both.
    assert!(
        close(cells[8][0], third(u0, u1, 2.0))
            && close(cells[8][2], u1)
            && close(cells[8][1], third(v0, v1, 2.0))
            && close(cells[8][3], v1),
        "bottom-right samples the texture's bottom-right: {:?}",
        cells[8]
    );
}

#[test]
fn a_control_exactly_as_big_as_its_corners_drops_the_middle() {
    // 8x8 with 4-pixel margins leaves no middle at all. Four quads, not nine
    // with five of them empty: a degenerate quad is a draw the batcher has to
    // carry and paints nothing.
    let scene = scene_with(
        "TextureRect",
        8,
        8,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n",
    );
    assert_eq!(
        card_quads(&scene),
        [(0, 0, 4, 4), (4, 0, 4, 4), (0, 4, 4, 4), (4, 4, 4, 4)],
        "the four corners and nothing between them"
    );
}

#[test]
fn margins_bigger_than_the_control_shrink_in_proportion() {
    // 4x4 with 4-pixel margins: the two corners want 8 pixels and have 4. They
    // shrink to 2 each rather than overlapping, so the frame stays a frame as
    // it collapses instead of drawing its right edge through its left one.
    let scene = scene_with(
        "TextureRect",
        4,
        4,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n",
    );
    let got = card_quads(&scene);
    assert_eq!(
        got,
        [(0, 0, 2, 2), (2, 0, 2, 2), (0, 2, 2, 2), (2, 2, 2, 2)]
    );
    // Nothing escapes the control, which is the property that matters.
    for (x, y, w, h) in got {
        assert!(
            x >= 0 && y >= 0 && x + w <= 4 && y + h <= 4,
            "{x},{y},{w},{h}"
        );
    }
}

#[test]
fn an_all_zero_slice_reads_as_no_slice() {
    // It would draw the whole texture as one middle cell -- which is what no
    // slice already does -- with eight empty quads for company.
    let scene = scene_with(
        "TextureRect",
        100,
        60,
        "texture = \"asset:ui/frame\"\nslice = [0.0, 0.0, 0.0, 0.0]\n",
    );
    assert_eq!(card_quads(&scene), [(0, 0, 100, 60)]);
}

#[test]
fn a_button_can_be_skinned_and_keeps_its_state_colour_as_the_tint() {
    // The piece nine child nodes cannot do. Only the engine writes `state`,
    // and nothing in a scene can read a parent's property, so a button skinned
    // out of `TextureRect` children would need a script each to recolour them.
    let base = "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n";
    let idle = scene_with("Button", 100, 60, base);
    assert_eq!(card_quads(&idle).len(), 9, "a skinned button is nine quads");

    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let tint = |scene: &Scene| {
        let id = scene.resolve_path("/Root/Card").expect("the card");
        let uid = scene.get(id).expect("the card").uid;
        let frame = extract::extract_with_canvas(scene, &atlas, &camera, None, canvas());
        frame
            .ui
            .iter()
            .find(|i| i.node == uid)
            .expect("a quad")
            .modulate
    };

    // `state` is what the simulation writes every tick: 0 idle, 1 hovered, 2
    // pressed. The same texture, three tints, no scene change.
    let hovered = scene_with("Button", 100, 60, &format!("{base}state = 1\n"));
    let pressed = scene_with("Button", 100, 60, &format!("{base}state = 2\n"));
    let (a, b, c) = (tint(&idle), tint(&hovered), tint(&pressed));
    assert_ne!(a, b, "hovering changes the tint");
    assert_ne!(b, c, "pressing changes it again");
    assert!(b[0] > a[0], "hover lightens: {a:?} -> {b:?}");
    assert!(c[0] < a[0], "press darkens: {a:?} -> {c:?}");
}

#[test]
fn an_unskinned_button_is_still_one_flat_quad() {
    // The control for the button half. A `slice` with no `texture` is nine
    // quads of the same colour, so it is ignored.
    let plain = scene_with("Button", 100, 60, "");
    assert_eq!(card_quads(&plain), [(0, 0, 100, 60)]);
    let sliced = scene_with("Button", 100, 60, "slice = [4.0, 4.0, 4.0, 4.0]\n");
    assert_eq!(
        card_quads(&sliced),
        [(0, 0, 100, 60)],
        "slicing the one white texel would be nine quads of one colour"
    );
}

#[test]
fn a_sliced_frame_batches_as_one_draw() {
    // The reason this is nine quads from one node rather than nine nodes.
    // Every cell shares the node's sort key, so the frame costs no more draw
    // calls than the flat rectangle it replaces.
    let scene = scene_with(
        "TextureRect",
        100,
        60,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n",
    );
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
    // Nine, not ten: the root is a bare `Control` and draws nothing itself.
    assert_eq!(frame.ui.len(), 9, "the nine cells");
    assert_eq!(
        frame.ui_batches.len(),
        1,
        "all of it in one batch: {:?}",
        frame.ui_batches
    );
}
