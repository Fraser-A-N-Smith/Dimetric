//! Getting from a canvas pixel to a world cell and back.
//!
//! The test the request asked for is the round trip: a cell that goes out
//! through the projection and comes back should be the cell it started in. A
//! second copy of this arithmetic in Lua would drift from the renderer's, and
//! the symptom would be clicks landing one cell off at certain camera
//! positions — which reproduces for nobody, so it is worth pinning here.

use dimetric_core::{Fx, Projection, Vec2Fx};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::camera::{to_canvas, to_world, view_of, View};

const CANVAS: Canvas = Canvas {
    width: 320,
    height: 180,
};
const RESOLUTION: (u32, u32) = (480, 270);

fn scene_with(projection: &str, at: &str, zoom: &str) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\n\
         [[node]]\nid = \"n_cam00000\"\nkind = \"Camera2D\"\nname = \"Eye\"\n\
         parent = \"n_root0000\"\npos = [{at}]\ncurrent = true\nzoom = {zoom}\n\
         projection = \"{projection}\"\n"
    );
    let out = dimetric_scene::parse(&text, "cam.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// The cell a world position falls in, on a 16-unit grid.
///
/// Rounding down rather than to nearest, which is what a grid lookup does:
/// world 15.9 and world 0.1 are both cell zero.
fn cell(p: Vec2Fx) -> (i32, i32) {
    let size = Fx::from_int(16);
    let floor = |v: Fx| {
        (v / size).trunc().round_int() - i32::from(v < Fx::ZERO && (v / size).trunc() != v / size)
    };
    (floor(p.x), floor(p.y))
}

#[test]
fn the_middle_of_the_canvas_is_where_the_camera_is_looking() {
    for projection in ["TopDown", "Isometric"] {
        let view = view_of(&scene_with(projection, "40.0, 24.0", "1.0"));
        let middle = to_world(view, CANVAS, RESOLUTION, Vec2Fx::from_ints(160, 90));
        assert_eq!(
            middle,
            Vec2Fx::from_ints(40, 24),
            "{projection}: the centre of the view should be the camera"
        );
    }
}

#[test]
fn a_cell_survives_the_round_trip_top_down() {
    let view = view_of(&scene_with("TopDown", "0.0, 0.0", "1.0"));
    for x in -3..4 {
        for y in -3..4 {
            let world = Vec2Fx::from_ints(x * 16 + 8, y * 16 + 8);
            let canvas = to_canvas(view, CANVAS, RESOLUTION, world);
            let back = to_world(view, CANVAS, RESOLUTION, canvas);
            assert_eq!(cell(back), cell(world), "({x}, {y}) moved cell");
        }
    }
}

#[test]
fn a_cell_survives_the_round_trip_in_dimetric() {
    // The one that matters: the 2:1 shear is where a hand-written inverse goes
    // wrong, and it is the projection the engine is named after.
    let view = view_of(&scene_with("Isometric", "0.0, 0.0", "1.0"));
    for x in -3..4 {
        for y in -3..4 {
            let world = Vec2Fx::from_ints(x * 16 + 8, y * 16 + 8);
            let canvas = to_canvas(view, CANVAS, RESOLUTION, world);
            let back = to_world(view, CANVAS, RESOLUTION, canvas);
            assert_eq!(cell(back), cell(world), "({x}, {y}) moved cell");
        }
    }
}

#[test]
fn the_round_trip_holds_with_the_camera_somewhere_else() {
    // "Clicks land one cell off at certain camera positions" is the exact bug
    // this is guarding, so the camera has to be somewhere awkward.
    let view = view_of(&scene_with("Isometric", "137.0, -89.0", "1.0"));
    for x in -2..3 {
        for y in -2..3 {
            let world = Vec2Fx::from_ints(137 + x * 16, -89 + y * 16);
            let back = to_world(
                view,
                CANVAS,
                RESOLUTION,
                to_canvas(view, CANVAS, RESOLUTION, world),
            );
            assert_eq!(cell(back), cell(world), "({x}, {y}) moved cell");
        }
    }
}

#[test]
fn the_round_trip_holds_under_zoom() {
    for zoom in ["1.0", "2.0", "0.5"] {
        let view = view_of(&scene_with("Isometric", "0.0, 0.0", zoom));
        for x in -2..3 {
            let world = Vec2Fx::from_ints(x * 16 + 8, 8);
            let back = to_world(
                view,
                CANVAS,
                RESOLUTION,
                to_canvas(view, CANVAS, RESOLUTION, world),
            );
            assert_eq!(cell(back), cell(world), "zoom {zoom}, x {x}");
        }
    }
}

#[test]
fn the_dimetric_shear_is_the_one_the_renderer_draws() {
    // Both directions come off `dimetric_core::Projection`, which the renderer
    // also uses — that shared source is the whole point of moving it there.
    // Asserted against the shear written out by hand so a change to either
    // side shows up here.
    let p = Projection::Isometric;
    let world = Vec2Fx::from_ints(10, 4);
    assert_eq!(p.project(world), Vec2Fx::from_ints(6, 7));
    assert_eq!(p.unproject(Vec2Fx::from_ints(6, 7)), world);
}

#[test]
fn a_scene_with_no_camera_looks_at_the_origin() {
    // What the renderer does, so a script picking off a cameraless scene gets
    // the same answer as the picture.
    let out = dimetric_scene::parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n",
        "bare.dim",
        &KindRegistry::with_builtins(),
    );
    let view = view_of(&out.doc.unwrap().scene);
    assert_eq!(view, View::default());
    assert_eq!(
        to_world(view, CANVAS, RESOLUTION, Vec2Fx::from_ints(160, 90)),
        Vec2Fx::ZERO
    );
}

#[test]
fn the_schema_already_refuses_a_zero_zoom() {
    // Worth knowing which guard is load-bearing. A scene cannot express it at
    // all — `zoom` has a range and zero is outside it — so the clamp in
    // `view_of` is belt-and-braces for a value arriving some other way, such
    // as a tween passing through zero.
    let out = dimetric_scene::parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Camera2D\"\nname = \"Eye\"\n\
         current = true\nzoom = 0.0\n",
        "zero.dim",
        &KindRegistry::with_builtins(),
    );
    assert!(out.diagnostics.has_errors());
    assert!(out.diagnostics.to_string().contains("DIM0202"));
}

#[test]
fn a_view_with_a_bad_zoom_does_not_divide_by_zero() {
    // The belt-and-braces path, reached directly since a scene cannot.
    let view = View {
        zoom: Fx::ZERO,
        ..View::default()
    };
    let _ = to_world(view, CANVAS, RESOLUTION, Vec2Fx::from_ints(1, 1));
}

#[test]
fn picking_depends_on_the_render_resolution() {
    // Which is why the resolution is in the replay contract rather than being
    // a render setting. A wider viewport shows more world at the same zoom, so
    // the same pointer lands somewhere else.
    let view = view_of(&scene_with("TopDown", "0.0, 0.0", "1.0"));
    let narrow = to_world(view, CANVAS, (480, 270), Vec2Fx::from_ints(0, 90));
    let wide = to_world(view, CANVAS, (960, 540), Vec2Fx::from_ints(0, 90));
    assert_ne!(
        narrow, wide,
        "if these matched, the resolution would not need to be in the contract"
    );
}
