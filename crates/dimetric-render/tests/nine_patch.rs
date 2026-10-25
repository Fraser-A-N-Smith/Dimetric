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

// --- G70: a Button can be cut from a sheet -----------------------------------

#[test]
fn a_button_can_be_cut_from_a_sheet_before_it_is_sliced() {
    // `TextureRect` could always do this and `Button` could not, so the two
    // controls drew a sliced texture the same way and only one of them could
    // choose which part: a game themed its panels from one strip and then had
    // to author a separate texture for every button.
    //
    // The frame texture is 12x12. Cutting the left 6x12 half and slicing with
    // 2-pixel margins gives columns 2, 2, 2 in the source -- so the cut has to
    // happen first, or the margins would measure the whole sheet and the middle
    // would be 8 wide instead of 2.
    let scene = scene_with(
        "Button",
        40,
        20,
        "texture = \"asset:ui/frame\"\nregion = [0.0, 0.0, 6.0, 12.0]\n\
         slice = [2.0, 2.0, 2.0, 2.0]\n",
    );
    let quads = card_quads(&scene);
    assert_eq!(
        quads,
        [
            (0, 0, 2, 2),
            (2, 0, 36, 2),
            (38, 0, 2, 2),
            (0, 2, 2, 16),
            (2, 2, 36, 16),
            (38, 2, 2, 16),
            (0, 18, 2, 2),
            (2, 18, 36, 2),
            (38, 18, 2, 2),
        ],
        "corners 2x2 from the cut rectangle's own margins"
    );
}

#[test]
fn a_buttons_region_names_part_of_the_sheet() {
    // The point of the cut: two buttons on one strip must sample different
    // texels. Compared against a `TextureRect` cut the same way, because the
    // two controls are meant to agree exactly.
    let uv_of = |kind: &str, region: &str| {
        let scene = scene_with(
            kind,
            40,
            20,
            &format!("texture = \"asset:ui/frame\"\n{region}"),
        );
        let id = scene.resolve_path("/Root/Card").expect("the card");
        let uid = scene.get(id).expect("the card").uid;
        let atlas = atlas();
        let camera = Camera::new((320, 180));
        let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
        frame.ui.iter().find(|i| i.node == uid).expect("a quad").uv
    };

    let left = uv_of("Button", "region = [0.0, 0.0, 6.0, 12.0]\n");
    let right = uv_of("Button", "region = [6.0, 0.0, 6.0, 12.0]\n");
    assert_ne!(
        left, right,
        "two halves of one strip are not the same texels"
    );
    assert_eq!(
        left,
        uv_of("TextureRect", "region = [0.0, 0.0, 6.0, 12.0]\n"),
        "a Button and a TextureRect cut identically"
    );
    assert_eq!(
        right,
        uv_of("TextureRect", "region = [6.0, 0.0, 6.0, 12.0]\n")
    );

    // And absent is still the whole texture, which is what every scene already
    // written means.
    assert_eq!(
        uv_of("Button", ""),
        uv_of("TextureRect", ""),
        "no region on either is the whole sheet"
    );
}

// --- G71: a nine-slice edge can repeat ---------------------------------------

#[test]
fn stretch_is_the_default_and_is_what_it_always_was() {
    // The control. `slice_mode` absent and `"Stretch"` have to be the same
    // picture, and the same picture as before the property existed.
    let base = "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n";
    let absent = card_quads(&scene_with("TextureRect", 100, 60, base));
    let named = card_quads(&scene_with(
        "TextureRect",
        100,
        60,
        &format!("{base}slice_mode = \"Stretch\"\n"),
    ));
    assert_eq!(absent, named);
    assert_eq!(
        absent.len(),
        9,
        "nine quads, as a stretched slice always was"
    );
}

#[test]
fn tiling_repeats_whole_cells_along_an_edge_at_a_constant_pitch() {
    // The 12x12 frame with 4-pixel margins has a 4-wide middle column, so a
    // 36-wide panel is nine repeats across its top edge: 4 corner, 7 x 4 of
    // edge, 4 corner. Every one whole, every one 4 wide -- which is the pitch
    // a rivet edge needs and a stretched edge cannot have.
    let scene = scene_with(
        "TextureRect",
        36,
        20,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let quads = card_quads(&scene);
    let top_edge: Vec<_> = quads
        .iter()
        .copied()
        .filter(|(x, y, _, _)| *y == 0 && *x > 0 && *x < 32)
        .collect();
    assert_eq!(
        top_edge,
        [
            (4, 0, 4, 4),
            (8, 0, 4, 4),
            (12, 0, 4, 4),
            (16, 0, 4, 4),
            (20, 0, 4, 4),
            (24, 0, 4, 4),
            (28, 0, 4, 4),
        ],
        "seven whole 4-pixel repeats at a constant pitch"
    );
    // The corners are untouched by the mode, in both modes.
    assert!(quads.contains(&(0, 0, 4, 4)), "{quads:?}");
    assert!(quads.contains(&(32, 0, 4, 4)), "{quads:?}");
}

#[test]
fn the_last_repeat_is_cut_to_fit_rather_than_squashed() {
    // A 30-wide panel leaves 22 pixels of top edge, which is five whole
    // 4-pixel repeats and a 2-pixel remainder. The remainder keeps the pitch
    // and loses its tail: a pattern meets the corner cleanly instead of
    // compressing into it, which is the whole difference between tiling and
    // stretching in pieces.
    let scene = scene_with(
        "TextureRect",
        30,
        20,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let quads = card_quads(&scene);
    let widths: Vec<i32> = quads
        .iter()
        .filter(|(x, y, _, _)| *y == 0 && *x > 0 && *x < 26)
        .map(|(_, _, w, _)| *w)
        .collect();
    assert_eq!(
        widths,
        [4, 4, 4, 4, 4, 2],
        "five whole repeats and a 2-pixel remainder"
    );
    // Nothing escapes the control, however the remainder falls.
    for (x, y, w, h) in &quads {
        assert!(
            *x >= 0 && *y >= 0 && x + w <= 30 && y + h <= 20,
            "{x},{y},{w},{h}"
        );
    }
}

#[test]
fn a_cut_repeat_has_its_uvs_cut_by_the_same_fraction() {
    // The part that makes it a cut rather than a squash. The last repeat is
    // half a cell wide, so it must sample half a cell -- otherwise the whole
    // rivet is drawn into half the space and the pitch breaks at the corner.
    let scene = scene_with(
        "TextureRect",
        30,
        20,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let id = scene.resolve_path("/Root/Card").expect("the card");
    let uid = scene.get(id).expect("the card").uid;
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
    // Top edge, in order: the whole repeats then the cut one.
    let edge: Vec<_> = frame
        .ui
        .iter()
        .filter(|i| i.node == uid)
        .filter(|i| {
            let min = i.pos - i.size / Fx::from_int(2);
            min.y.to_int_trunc() == 0 && min.x.to_int_trunc() > 0 && min.x.to_int_trunc() < 26
        })
        .collect();
    assert_eq!(edge.len(), 6);

    let span = |uv: [f32; 4]| uv[2] - uv[0];
    let whole = span(edge[0].uv);
    for (index, item) in edge.iter().enumerate().take(5) {
        assert!(
            (span(item.uv) - whole).abs() < 1e-6,
            "repeat {index} should sample a whole cell"
        );
    }
    assert!(
        (span(edge[5].uv) - whole / 2.0).abs() < 1e-6,
        "the 2-pixel remainder of a 4-pixel cell samples half a cell: {} vs {}",
        span(edge[5].uv),
        whole / 2.0
    );
    assert_eq!(
        edge[5].uv[0], edge[0].uv[0],
        "cut from the far edge, so the pattern still starts where it should"
    );
}

#[test]
fn tiling_runs_down_the_side_edges_too() {
    // The vertical half of the same rule: the middle row of the left and right
    // columns is that edge, and repeats down.
    let scene = scene_with(
        "TextureRect",
        20,
        36,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let left_edge: Vec<_> = card_quads(&scene)
        .into_iter()
        .filter(|(x, y, _, _)| *x == 0 && *y > 0 && *y < 32)
        .collect();
    assert_eq!(
        left_edge,
        [
            (0, 4, 4, 4),
            (0, 8, 4, 4),
            (0, 12, 4, 4),
            (0, 16, 4, 4),
            (0, 20, 4, 4),
            (0, 24, 4, 4),
            (0, 28, 4, 4),
        ],
        "seven whole repeats down the left edge"
    );
}

#[test]
fn the_middle_stays_stretched_even_when_tiling() {
    // The choice made, and the arithmetic behind it. A 300x150 panel with a
    // 4-pixel middle cell would be 73 x 36 = 2628 quads if the middle tiled
    // too. A frame's middle is usually flat, so it stretches in both modes and
    // a tiled frame costs quads along its edges only.
    let scene = scene_with(
        "TextureRect",
        36,
        36,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let quads = card_quads(&scene);
    let middle: Vec<_> = quads
        .iter()
        .copied()
        .filter(|(x, y, _, _)| *x == 4 && *y == 4)
        .collect();
    assert_eq!(
        middle,
        [(4, 4, 28, 28)],
        "one stretched quad, not forty-nine tiled ones"
    );
    // Four corners, seven per edge, one middle.
    assert_eq!(quads.len(), 4 + 7 * 4 + 1);
}

#[test]
fn a_tiled_frame_still_batches_as_one_draw() {
    // Every cell shares the node's sort key, so tiling costs vertices and not
    // draw calls -- which is what makes the quad count acceptable at all.
    let scene = scene_with(
        "Button",
        120,
        24,
        "texture = \"asset:ui/frame\"\nslice = [4.0, 4.0, 4.0, 4.0]\n\
         slice_mode = \"Tile\"\n",
    );
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
    assert!(frame.ui.len() > 9, "it really did tile: {}", frame.ui.len());
    assert_eq!(frame.ui_batches.len(), 1, "{:?}", frame.ui_batches);
}

#[test]
fn tiling_without_a_slice_changes_nothing() {
    // `slice_mode` is only read when `slice` is set: there are no edges to
    // repeat without margins, and a lone mode must not quietly become one.
    let scene = scene_with(
        "TextureRect",
        100,
        60,
        "texture = \"asset:ui/frame\"\nslice_mode = \"Tile\"\n",
    );
    assert_eq!(card_quads(&scene), [(0, 0, 100, 60)]);
}
