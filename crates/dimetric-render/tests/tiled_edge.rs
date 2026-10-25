//! A tiled nine-slice edge, photographed.
//!
//! The quad arithmetic is checked in `nine_patch.rs`. This is the other half:
//! that the quads and their UVs agree on screen, which per-quad assertions
//! cannot prove — two neighbouring quads could each be the right size and
//! sample the wrong part, and the result would be a smear with correct
//! geometry.
//!
//! The texture carries a two-pixel pattern along its top edge: one bright
//! column, one dark. Tiled along a panel eight cells wide, the screen has to
//! show eight bright columns at a pitch of two. Stretched, it shows one bright
//! band half the panel wide — which is the defect `slice_mode` exists for.
//!
//! Needs a graphics adapter, and skips without one unless
//! `DIMETRIC_REQUIRE_GPU` is set.

use dimetric_core::Fx;
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, PresentFilter, RenderSettings, Renderer,
};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{Color, KindRegistry, Scene};

/// A 6x6 sheet with 2-pixel margins, so every band is 2x2.
const SHEET: u32 = 6;
const MARGIN: u32 = 2;
/// The frame is 20 wide, leaving 16 pixels of edge: eight 2-pixel cells.
const WIDTH: i32 = 20;
const HEIGHT: i32 = 12;

/// The sheet: the top edge's cell is one bright column then one dark one, and
/// everything else is mid grey so only the pattern stands out.
fn atlas() -> Atlas {
    let mut pixels = Vec::with_capacity((SHEET * SHEET * 4) as usize);
    for y in 0..SHEET {
        for x in 0..SHEET {
            // The top band is rows 0..2; its middle column is x 2..4.
            let in_top_edge = y < MARGIN && (MARGIN..SHEET - MARGIN).contains(&x);
            let bright = in_top_edge && x == MARGIN;
            pixels.extend_from_slice(match (in_top_edge, bright) {
                (true, true) => &[0xFF, 0xFF, 0xFF, 0xFF],
                (true, false) => &[0x00, 0x00, 0x00, 0xFF],
                _ => &[0x80, 0x80, 0x80, 0xFF],
            });
        }
    }
    Atlas::pack(
        vec![
            Source {
                name: "ui/rivets".into(),
                width: SHEET,
                height: SHEET,
                pixels,
            },
            dimetric_render::atlas::solid(Color::WHITE),
            dimetric_render::atlas::placeholder(8),
        ],
        64,
    )
}

fn canvas() -> Canvas {
    Canvas {
        width: WIDTH,
        height: HEIGHT,
    }
}

/// One canvas pixel to one screen pixel, so a column is a column.
///
/// The width is a parameter because it has to *match* the canvas: render a
/// 21-pixel canvas into a 20-pixel target and the composite resamples, which
/// breaks the measured pitch and says nothing about the tiling.
fn settings(width: i32) -> RenderSettings {
    RenderSettings {
        internal_resolution: (width as u32, HEIGHT as u32),
        integer_upscale: true,
        pixel_snap: true,
        ambient: Color::WHITE,
        present_filter: PresentFilter::Nearest,
    }
}

fn scene(mode: &str) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"TextureRect\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:ui/rivets\"\n\
         slice = [2.0, 2.0, 2.0, 2.0]\n{mode}\
         offset_left = 0.0\noffset_top = 0.0\n\
         offset_right = {WIDTH}.0\noffset_bottom = {HEIGHT}.0\n"
    );
    let out = dimetric_scene::parse(&text, "t.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// Which x positions along the top row came out bright, or `None` with no
/// adapter.
fn bright_columns(mode: &str) -> Option<Vec<u32>> {
    let atlas = atlas();
    let instance = headless_instance();
    let mut renderer = match Renderer::new(&instance, None, &atlas, settings(WIDTH)) {
        Ok(renderer) => renderer,
        Err(e) => {
            assert!(
                std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                "DIMETRIC_REQUIRE_GPU is set but no adapter was available: {e}"
            );
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let capture = Capture::new(&renderer, (WIDTH as u32, HEIGHT as u32));
    let camera = Camera::new((WIDTH as u32, HEIGHT as u32));
    let frame = extract::extract_with_canvas(&scene(mode), &atlas, &camera, None, canvas());
    let pixels = capture.render(&mut renderer, &frame).expect("a frame");

    // Row zero, the top edge. Bright is unmistakable against black and grey.
    let mut out = Vec::new();
    for x in 0..WIDTH as u32 {
        let at = (x * 4) as usize;
        if pixels[at] > 0xC0 {
            out.push(x);
        }
    }
    Some(out)
}

#[test]
fn a_tiled_edge_shows_whole_cells_at_a_constant_pitch() {
    let Some(bright) = bright_columns("slice_mode = \"Tile\"\n") else {
        return;
    };
    // The edge runs x 2..18, eight 2-pixel cells, each starting bright. The
    // corners (x 0..2 and 18..20) are grey.
    assert_eq!(
        bright,
        [2, 4, 6, 8, 10, 12, 14, 16],
        "eight rivets, every other column, right across the panel"
    );
}

#[test]
fn a_stretched_edge_smears_the_same_pattern() {
    // The defect, measured. One cell stretched across the whole edge puts the
    // bright half in one band eight pixels wide -- a pattern that has stopped
    // being a pattern, which is what no amount of authoring could fix.
    let Some(bright) = bright_columns("") else {
        return;
    };
    assert_eq!(
        bright,
        (2..10).collect::<Vec<u32>>(),
        "one bright band half the edge wide, not eight rivets"
    );
}

#[test]
fn the_last_cell_of_a_tiled_edge_is_cut_and_not_squashed() {
    // A panel 21 wide leaves 17 pixels of edge: eight whole cells and a
    // 1-pixel remainder, which is the bright half of a ninth rivet. Squashing
    // the ninth cell into one pixel would land its dark half there instead,
    // and the pitch would break at the corner.
    let atlas = atlas();
    let instance = headless_instance();
    let mut renderer = match Renderer::new(&instance, None, &atlas, settings(WIDTH + 1)) {
        Ok(renderer) => renderer,
        Err(e) => {
            assert!(
                std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                "DIMETRIC_REQUIRE_GPU is set but no adapter was available: {e}"
            );
            return;
        }
    };
    let wide = Canvas {
        width: WIDTH + 1,
        height: HEIGHT,
    };
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"TextureRect\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:ui/rivets\"\n\
         slice = [2.0, 2.0, 2.0, 2.0]\nslice_mode = \"Tile\"\n\
         offset_left = 0.0\noffset_top = 0.0\n\
         offset_right = {}.0\noffset_bottom = {HEIGHT}.0\n",
        WIDTH + 1
    );
    let out = dimetric_scene::parse(&text, "t.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;

    let capture = Capture::new(&renderer, (wide.width as u32, HEIGHT as u32));
    let camera = Camera::new((wide.width as u32, HEIGHT as u32));
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, wide);
    let pixels = capture.render(&mut renderer, &frame).expect("a frame");
    let mut bright = Vec::new();
    for x in 0..wide.width as u32 {
        let at = (x * 4) as usize;
        if pixels[at] > 0xC0 {
            bright.push(x);
        }
    }
    assert_eq!(
        bright,
        [2, 4, 6, 8, 10, 12, 14, 16, 18],
        "the ninth rivet's bright column survives in the one pixel it has"
    );
    let _ = Fx::ONE;
}
