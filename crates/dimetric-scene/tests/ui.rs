//! Control layout: anchors, offsets, nesting, and hit testing.
//!
//! Every number here is a whole pixel on a declared canvas rather than a
//! fraction of somebody's window. That is the property the tests are really
//! guarding: two machines with different monitors have to agree on where a
//! button is, or they disagree on whether a click hit it.

use dimetric_core::{Fx, Vec2Fx};
use dimetric_scene::ui::{hit, layout, Canvas};
use dimetric_scene::{KindRegistry, Scene};

fn scene_of(body: &str) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n{body}"
    );
    let out = dimetric_scene::parse(&text, "t.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn canvas() -> Canvas {
    Canvas {
        width: 320,
        height: 180,
    }
}

fn find(scene: &Scene, path: &str) -> dimetric_core::NodeId {
    scene.resolve_path(path).expect("node")
}

const PINNED: &str = r#"
[[node]]
id = "n_box00000"
kind = "Control"
name = "Box"
parent = "n_root0000"
offset_left = 10.0
offset_top = 20.0
offset_right = 60.0
offset_bottom = 50.0
"#;

#[test]
fn offsets_alone_pin_a_box_to_the_top_left() {
    let scene = scene_of(PINNED);
    let out = layout(&scene, canvas());
    let rect = out[&find(&scene, "/Root/Box")];
    assert_eq!(rect.pos, Vec2Fx::from_ints(10, 20));
    assert_eq!(rect.size, Vec2Fx::from_ints(50, 30));
}

#[test]
fn a_box_pinned_by_offsets_does_not_move_with_the_canvas() {
    // The whole point of anchors: without one, a control is fixed. With one it
    // follows. Both have to be true or the scheme is not doing anything.
    let scene = scene_of(PINNED);
    let small = layout(
        &scene,
        Canvas {
            width: 320,
            height: 180,
        },
    );
    let large = layout(
        &scene,
        Canvas {
            width: 1280,
            height: 720,
        },
    );
    let id = find(&scene, "/Root/Box");
    assert_eq!(small[&id], large[&id]);
}

#[test]
fn anchors_place_an_edge_as_a_fraction_of_the_parent() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_right000"
kind = "Control"
name = "Right"
parent = "n_root0000"
anchor_left = 1.0
anchor_right = 1.0
anchor_top = 0.0
anchor_bottom = 0.0
offset_left = -40.0
offset_right = -10.0
offset_bottom = 20.0
"#,
    );
    let out = layout(&scene, canvas());
    let rect = out[&find(&scene, "/Root/Right")];
    // Anchored to the right edge, then offset back: 320 - 40 = 280.
    assert_eq!(rect.pos, Vec2Fx::from_ints(280, 0));
    assert_eq!(rect.size, Vec2Fx::from_ints(30, 20));
}

#[test]
fn stretching_follows_the_canvas() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_bar00000"
kind = "Control"
name = "Bar"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 0.0
offset_left = 8.0
offset_right = -8.0
offset_bottom = 16.0
"#,
    );
    let id = find(&scene, "/Root/Bar");
    let small = layout(
        &scene,
        Canvas {
            width: 320,
            height: 180,
        },
    )[&id];
    let large = layout(
        &scene,
        Canvas {
            width: 640,
            height: 360,
        },
    )[&id];
    assert_eq!(small.size, Vec2Fx::from_ints(304, 16));
    assert_eq!(large.size, Vec2Fx::from_ints(624, 16));
}

#[test]
fn a_child_is_laid_out_against_its_parent_not_the_canvas() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_panel000"
kind = "Control"
name = "Panel"
parent = "n_root0000"
offset_left = 100.0
offset_top = 50.0
offset_right = 200.0
offset_bottom = 150.0

[[node]]
id = "n_inner000"
kind = "Control"
name = "Inner"
parent = "n_panel000"
anchor_right = 1.0
anchor_bottom = 1.0
offset_left = 10.0
offset_top = 10.0
offset_right = -10.0
offset_bottom = -10.0
"#,
    );
    let out = layout(&scene, canvas());
    let inner = out[&find(&scene, "/Root/Panel/Inner")];
    assert_eq!(inner.pos, Vec2Fx::from_ints(110, 60));
    assert_eq!(inner.size, Vec2Fx::from_ints(80, 80));
}

#[test]
fn an_inside_out_box_collapses_rather_than_drawing_backwards() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_bad00000"
kind = "Control"
name = "Bad"
parent = "n_root0000"
offset_left = 100.0
offset_right = 40.0
offset_top = 100.0
offset_bottom = 40.0
"#,
    );
    let rect = layout(&scene, canvas())[&find(&scene, "/Root/Bad")];
    assert_eq!(rect.size, Vec2Fx::ZERO, "a negative size draws inside out");
}

const TWO_BUTTONS: &str = r#"
[[node]]
id = "n_back0000"
kind = "Panel"
name = "Backdrop"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 1.0
catches_input = false

[[node]]
id = "n_btna0000"
kind = "Control"
name = "A"
parent = "n_root0000"
offset_left = 10.0
offset_top = 10.0
offset_right = 60.0
offset_bottom = 30.0

[[node]]
id = "n_btnb0000"
kind = "Control"
name = "B"
parent = "n_root0000"
offset_left = 40.0
offset_top = 10.0
offset_right = 90.0
offset_bottom = 30.0
"#;

#[test]
fn a_hit_finds_the_control_under_the_point() {
    let scene = scene_of(TWO_BUTTONS);
    let out = layout(&scene, canvas());
    let at = |x: i32, y: i32| hit(&scene, &out, Vec2Fx::from_ints(x, y));

    assert_eq!(at(20, 20), Some(find(&scene, "/Root/A")));
    assert_eq!(at(80, 20), Some(find(&scene, "/Root/B")));
    assert_eq!(at(200, 100), None, "nothing is there but the backdrop");
}

#[test]
fn the_later_sibling_wins_an_overlap() {
    // Same order they draw in: what is on top is what gets clicked.
    let scene = scene_of(TWO_BUTTONS);
    let out = layout(&scene, canvas());
    assert_eq!(
        hit(&scene, &out, Vec2Fx::from_ints(50, 20)),
        Some(find(&scene, "/Root/B"))
    );
}

#[test]
fn a_control_that_does_not_catch_input_is_scenery() {
    // The backdrop covers the whole canvas. If it caught input no button
    // underneath it would ever be clickable.
    let scene = scene_of(TWO_BUTTONS);
    let out = layout(&scene, canvas());
    assert!(out.contains_key(&find(&scene, "/Root/Backdrop")));
    assert_eq!(hit(&scene, &out, Vec2Fx::from_ints(300, 170)), None);
}

#[test]
fn a_hidden_control_is_not_hit() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_hidden00"
kind = "Control"
name = "Hidden"
parent = "n_root0000"
visible = false
offset_right = 100.0
offset_bottom = 100.0
"#,
    );
    let out = layout(&scene, canvas());
    assert_eq!(hit(&scene, &out, Vec2Fx::from_ints(10, 10)), None);
}

#[test]
fn layout_is_the_same_every_time() {
    let scene = scene_of(TWO_BUTTONS);
    let once = layout(&scene, canvas());
    for _ in 0..32 {
        assert_eq!(layout(&scene, canvas()), once);
    }
}

#[test]
fn a_fractional_anchor_lands_on_an_exact_pixel() {
    // Half of 320 is 160 and there is no rounding to disagree about. Fixed
    // point is what makes that a guarantee rather than a coincidence.
    let scene = scene_of(
        r#"
[[node]]
id = "n_half0000"
kind = "Control"
name = "Half"
parent = "n_root0000"
anchor_left = 0.5
anchor_right = 1.0
anchor_bottom = 0.5
"#,
    );
    let rect = layout(&scene, canvas())[&find(&scene, "/Root/Half")];
    assert_eq!(rect.pos.x, Fx::from_int(160));
    assert_eq!(rect.size, Vec2Fx::from_ints(160, 90));
}

// -- Containers -----------------------------------------------------------

#[test]
fn a_vbox_stacks_its_children_and_fills_the_width() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_menu0000"
kind = "VBox"
name = "Menu"
parent = "n_root0000"
offset_left = 100.0
offset_top = 50.0
offset_right = 220.0
offset_bottom = 170.0
spacing = 4.0
padding = 6.0

[[node]]
id = "n_one00000"
kind = "Panel"
name = "One"
parent = "n_menu0000"
offset_bottom = 20.0

[[node]]
id = "n_two00000"
kind = "Panel"
name = "Two"
parent = "n_menu0000"
offset_bottom = 30.0

[[node]]
id = "n_three000"
kind = "Panel"
name = "Three"
parent = "n_menu0000"
offset_bottom = 20.0
"#,
    );
    let out = layout(&scene, canvas());
    let one = out[&find(&scene, "/Root/Menu/One")];
    let two = out[&find(&scene, "/Root/Menu/Two")];
    let three = out[&find(&scene, "/Root/Menu/Three")];

    // Inside the padding, filling the width: 120 wide less 6 either side.
    assert_eq!(one.pos, Vec2Fx::from_ints(106, 56));
    assert_eq!(one.size, Vec2Fx::from_ints(108, 20));

    // Each child starts after the last one and the spacing.
    assert_eq!(two.pos.y, Fx::from_int(56 + 20 + 4));
    assert_eq!(two.size.y, Fx::from_int(30));
    assert_eq!(three.pos.y, Fx::from_int(56 + 20 + 4 + 30 + 4));

    // All the same width: that is what being in a box buys you.
    assert_eq!(one.size.x, two.size.x);
    assert_eq!(two.size.x, three.size.x);
}

#[test]
fn an_hbox_stacks_across_and_fills_the_height() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_row00000"
kind = "HBox"
name = "Row"
parent = "n_root0000"
offset_right = 200.0
offset_bottom = 40.0
spacing = 5.0

[[node]]
id = "n_a0000000"
kind = "Panel"
name = "A"
parent = "n_row00000"
offset_right = 30.0

[[node]]
id = "n_b0000000"
kind = "Panel"
name = "B"
parent = "n_row00000"
offset_right = 50.0
"#,
    );
    let out = layout(&scene, canvas());
    let a = out[&find(&scene, "/Root/Row/A")];
    let b = out[&find(&scene, "/Root/Row/B")];

    assert_eq!(a.pos, Vec2Fx::ZERO);
    assert_eq!(a.size, Vec2Fx::from_ints(30, 40));
    assert_eq!(b.pos.x, Fx::from_int(35));
    assert_eq!(b.size, Vec2Fx::from_ints(50, 40));
}

#[test]
fn a_child_of_a_box_ignores_its_own_anchors() {
    // Putting a control in a box is handing the box its position. A child that
    // still honoured its anchors would sit wherever it liked and overlap its
    // siblings, which is not a container.
    let body = |anchors: &str| {
        format!(
            r#"
[[node]]
id = "n_col00000"
kind = "VBox"
name = "Col"
parent = "n_root0000"
offset_right = 100.0
offset_bottom = 100.0

[[node]]
id = "n_kid00000"
kind = "Panel"
name = "Kid"
parent = "n_col00000"
offset_bottom = 20.0
{anchors}
"#
        )
    };
    let plain = scene_of(&body(""));
    let anchored = scene_of(&body("anchor_left = 0.5\nanchor_top = 0.75"));

    let a = layout(&plain, canvas())[&find(&plain, "/Root/Col/Kid")];
    let b = layout(&anchored, canvas())[&find(&anchored, "/Root/Col/Kid")];
    assert_eq!(a, b);
}

#[test]
fn a_box_nested_in_a_box_stacks_within_its_slot() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_outer000"
kind = "VBox"
name = "Outer"
parent = "n_root0000"
offset_right = 200.0
offset_bottom = 200.0

[[node]]
id = "n_head0000"
kind = "Panel"
name = "Head"
parent = "n_outer000"
offset_bottom = 10.0

[[node]]
id = "n_inner000"
kind = "HBox"
name = "Inner"
parent = "n_outer000"
offset_bottom = 40.0

[[node]]
id = "n_left0000"
kind = "Panel"
name = "Left"
parent = "n_inner000"
offset_right = 60.0
"#,
    );
    let out = layout(&scene, canvas());
    let inner = out[&find(&scene, "/Root/Outer/Inner")];
    let left = out[&find(&scene, "/Root/Outer/Inner/Left")];

    // The inner box takes its slot under the header.
    assert_eq!(inner.pos.y, Fx::from_int(10));
    assert_eq!(inner.size, Vec2Fx::from_ints(200, 40));
    // And its own child stacks inside that, not inside the outer box.
    assert_eq!(left.pos, Vec2Fx::from_ints(0, 10));
    assert_eq!(left.size, Vec2Fx::from_ints(60, 40));
}

#[test]
fn an_empty_box_lays_out_without_complaint() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_empty000"
kind = "VBox"
name = "Empty"
parent = "n_root0000"
offset_right = 50.0
offset_bottom = 50.0
"#,
    );
    let out = layout(&scene, canvas());
    assert_eq!(
        out[&find(&scene, "/Root/Empty")].size,
        Vec2Fx::from_ints(50, 50)
    );
}

#[test]
fn a_button_is_hit_like_any_other_control() {
    let scene = scene_of(
        r#"
[[node]]
id = "n_btn00000"
kind = "Button"
name = "Go"
parent = "n_root0000"
offset_left = 10.0
offset_top = 10.0
offset_right = 60.0
offset_bottom = 30.0
"#,
    );
    let out = layout(&scene, canvas());
    assert_eq!(
        hit(&scene, &out, Vec2Fx::from_ints(30, 20)),
        Some(find(&scene, "/Root/Go"))
    );
}
