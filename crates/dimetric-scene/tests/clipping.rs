//! A control can confine what is inside it, for the pointer as well as the eye.
//!
//! `docs/ENGINE-GAPS.md` declined scroll containers, grid containers and
//! nine-patch panels as layout convenience, and then said this about the one
//! piece worth reconsidering:
//!
//! > **Clipping** is a renderer capability a game genuinely cannot build from
//! > Lua: a scroll offset is already expressible today (a `VBox`'s `offset_top`
//! > is a control property, in state, and replays), so what is actually missing
//! > from a scrollable list is a scissor rect. That is a small, bounded thing
//! > with no layout policy in it. A `ScrollContainer` node is not.
//!
//! This is that. `clip_children = true` on any control, and its descendants are
//! drawn and clicked only inside its rectangle.
//!
//! The tests about the pointer are the ones that matter. A row scrolled out of
//! a list and still taking clicks is exactly the hidden-panel defect in a new
//! place, so the clip map has one door — `ui::clips` — that both the hit test
//! and the renderer read.

use dimetric_core::{NodeId, Vec2Fx};
use dimetric_scene::ui::{clips, hit, layout, Canvas};
use dimetric_scene::{KindRegistry, Scene, Value};

/// A 100x100 viewport at canvas y 40, with a 40-tall row scrolled 30 pixels
/// out of the top of it.
///
/// Offsets are relative to the parent, so the row's `-30.0` puts it at canvas
/// y 10..50 inside a viewport at y 40..140 — the shape a scrolled list has, and
/// expressible today without any container node, which is exactly why the gap
/// was a scissor rect and not a `ScrollContainer`.
const LIST: &str = r#"
[[node]]
id = "n_view0000"
kind = "Panel"
name = "View"
parent = "n_root0000"
offset_left = 0.0
offset_top = 40.0
offset_right = 100.0
offset_bottom = 140.0

[[node]]
id = "n_row00000"
kind = "Button"
name = "Row"
parent = "n_view0000"
offset_left = 0.0
offset_top = -30.0
offset_right = 100.0
offset_bottom = 10.0
"#;

fn scene() -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n{LIST}"
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

fn id_of(scene: &Scene, path: &str) -> NodeId {
    scene.resolve_path(path).unwrap_or_else(|| panic!("{path}"))
}

fn set(scene: &mut Scene, path: &str, key: &str, value: Value) {
    let id = id_of(scene, path);
    scene.get_mut(id).expect("the node").set(key, value);
}

/// What a click at `(x, y)` in canvas pixels lands on, by name.
fn clicked(scene: &Scene, x: i32, y: i32) -> String {
    let rects = layout(scene, canvas());
    hit(scene, &rects, Vec2Fx::from_ints(x, y))
        .and_then(|id| scene.get(id))
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "nothing".to_string())
}

#[test]
fn a_scene_that_asks_for_nothing_is_clipped_by_nothing() {
    // The property that keeps every scene already written working, and the
    // reason the map is a map rather than a rectangle per node.
    let scene = scene();
    let rects = layout(&scene, canvas());
    assert!(
        clips(&scene, &rects).is_empty(),
        "a UI that clips nothing pays nothing"
    );
}

#[test]
fn a_row_scrolled_out_of_a_list_is_not_clickable() {
    // The defect this exists to prevent, and the hidden-panel bug in a new
    // place: a row the player cannot see taking the click meant for what is
    // behind it.
    let mut scene = scene();
    set(&mut scene, "/Root/View", "clip_children", Value::Bool(true));

    // The row is at y 10..50 and the viewport at y 40..140, so only y 40..50
    // of it is inside.
    assert_eq!(
        clicked(&scene, 50, 45),
        "Row",
        "the part still inside the viewport is live"
    );
    assert_eq!(
        clicked(&scene, 50, 20),
        "Root",
        "where the row is and cannot be seen, the click falls through to what \
         is behind the list"
    );
    assert_eq!(
        clicked(&scene, 50, 100),
        "View",
        "and below the row is the viewport itself"
    );
}

#[test]
fn without_the_flag_the_row_still_takes_the_click() {
    // The control. Same geometry, no `clip_children`: the row overflows its
    // viewport and is clickable where it overflows, which is what every scene
    // written before clipping existed did and must go on doing.
    let scene = scene();
    assert_eq!(
        clicked(&scene, 50, 20),
        "Row",
        "unclipped, the row keeps its whole rectangle"
    );
    assert_eq!(clicked(&scene, 50, 45), "Row");
}

#[test]
fn nested_clips_intersect() {
    // A row inside a scrolled list inside a dialog is confined to the part of
    // the list that is inside the dialog. Taking only the nearest clip would
    // let the list's contents escape the dialog.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_dialog00\"\nkind = \"Panel\"\nname = \"Dialog\"\n\
         parent = \"n_root0000\"\nclip_children = true\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 60.0\noffset_bottom = 60.0\n\
         \n[[node]]\nid = \"n_list0000\"\nkind = \"Panel\"\nname = \"List\"\n\
         parent = \"n_dialog00\"\nclip_children = true\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 200.0\noffset_bottom = 200.0\n\
         \n[[node]]\nid = \"n_row00000\"\nkind = \"Button\"\nname = \"Row\"\n\
         parent = \"n_list0000\"\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 200.0\noffset_bottom = 200.0\n";
    let out = dimetric_scene::parse(text, "n.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;

    let rects = layout(&scene, canvas());
    let map = clips(&scene, &rects);
    let row = id_of(&scene, "/Root/Dialog/List/Row");
    let confined = map.get(&row).copied().expect("the row is clipped");
    assert_eq!(
        (confined.size.x, confined.size.y),
        (
            dimetric_core::Fx::from_int(60),
            dimetric_core::Fx::from_int(60)
        ),
        "the row is held to the dialog, not to the list that overflows it"
    );

    // The list itself is clipped by the dialog but not by its own flag: a
    // control that clipped itself to itself would be a no-op at best.
    let list = id_of(&scene, "/Root/Dialog/List");
    assert_eq!(
        map.get(&list).copied().map(|r| (r.size.x, r.size.y)),
        Some((
            dimetric_core::Fx::from_int(60),
            dimetric_core::Fx::from_int(60)
        ))
    );
    assert!(
        !map.contains_key(&id_of(&scene, "/Root/Dialog")),
        "nothing above the dialog clips it"
    );

    assert_eq!(clicked(&scene, 30, 30), "Row");
    assert_eq!(
        clicked(&scene, 100, 100),
        "Root",
        "outside the dialog, everything inside it is gone and the click falls \
         through to the root"
    );
}

#[test]
fn a_clip_that_overlaps_nothing_hides_everything_inside() {
    // Two clips that do not meet. The answer has to be "nothing is visible",
    // not "no clip applies", which is what a missing entry would mean.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_left0000\"\nkind = \"Panel\"\nname = \"Left\"\n\
         parent = \"n_root0000\"\nclip_children = true\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 20.0\noffset_bottom = 20.0\n\
         \n[[node]]\nid = \"n_rght0000\"\nkind = \"Panel\"\nname = \"Right\"\n\
         parent = \"n_left0000\"\nclip_children = true\n\
         offset_left = 100.0\noffset_top = 100.0\noffset_right = 140.0\noffset_bottom = 140.0\n\
         \n[[node]]\nid = \"n_row00000\"\nkind = \"Button\"\nname = \"Row\"\n\
         parent = \"n_rght0000\"\n\
         offset_left = 100.0\noffset_top = 100.0\noffset_right = 140.0\noffset_bottom = 140.0\n";
    let out = dimetric_scene::parse(text, "e.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;
    let rects = layout(&scene, canvas());
    let map = clips(&scene, &rects);
    let row = id_of(&scene, "/Root/Left/Right/Row");
    assert_eq!(
        map.get(&row).map(|r| r.is_empty()),
        Some(true),
        "an empty rectangle, so nothing inside is drawn or clicked"
    );
    assert_eq!(
        clicked(&scene, 110, 110),
        "Root",
        "`Right` is there and clipped to nothing, so it takes no click either"
    );
    assert_eq!(clicked(&scene, 10, 10), "Left");
}
