//! Hiding a panel hides what is inside it, for the pointer as well as the eye.
//!
//! `visible` on a node is the node's own flag, and a parent's is never stored
//! on its children — so "can this be seen" is a question about the whole chain.
//! The renderer walked that chain and `ui::hit` read a control's own flag and
//! nothing else, so the two disagreed about a closed screen: its rows were
//! invisible and still on top. A click in the middle of the board landed on a
//! row nobody could see, `ui.hovered` named it, and the board ignored the
//! click.
//!
//! The workaround on offer was hiding every child by hand when a panel closes,
//! which is a second copy of the engine's own visibility rule, written in Lua,
//! in every panel.

use dimetric_core::{NodeId, Vec2Fx};
use dimetric_scene::ui::{hit, layout, Canvas};
use dimetric_scene::{KindRegistry, Scene};

/// A full-canvas `Screen` panel with one `Row4` button filling it.
///
/// The shape the defect was found in: a screen control with its rows under it,
/// closed by hiding the screen.
const SCREEN: &str = r#"
[[node]]
id = "n_screen00"
kind = "Panel"
name = "Screen"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 1.0

[[node]]
id = "n_row40000"
kind = "Button"
name = "Row4"
parent = "n_screen00"
anchor_right = 1.0
anchor_bottom = 1.0
"#;

fn scene() -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n{SCREEN}"
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

fn set_visible(scene: &mut Scene, path: &str, on: bool) {
    let id = id_of(scene, path);
    scene.get_mut(id).unwrap().visible = on;
}

/// What a click in the middle of the canvas lands on.
fn clicked(scene: &Scene) -> Option<NodeId> {
    let rects = layout(scene, canvas());
    hit(scene, &rects, Vec2Fx::from_ints(160, 90))
}

#[test]
fn an_open_screen_catches_the_click() {
    let scene = scene();
    assert_eq!(clicked(&scene), Some(id_of(&scene, "/Root/Screen/Row4")));
}

#[test]
fn a_closed_screen_lets_the_click_through() {
    // The defect. Only the screen is hidden; its row is untouched, exactly as
    // a game that closes a panel by hiding the panel leaves it. The root is a
    // full-canvas `Control` and catches input, so the click lands on it rather
    // than on nothing — what matters is that it is no longer the row.
    let mut scene = scene();
    set_visible(&mut scene, "/Root/Screen", false);
    assert_eq!(
        clicked(&scene),
        Some(id_of(&scene, "/Root")),
        "a row inside a hidden screen is still catching the pointer"
    );
}

#[test]
fn hiding_the_control_itself_still_works() {
    // The case that always worked, so the fix is an addition rather than a
    // replacement.
    let mut scene = scene();
    set_visible(&mut scene, "/Root/Screen/Row4", false);
    assert_eq!(clicked(&scene), Some(id_of(&scene, "/Root/Screen")));
}

#[test]
fn the_rule_is_the_whole_chain_not_just_the_parent() {
    // Hidden two levels up from the row, which a parent-only check would miss.
    let mut scene = scene();
    set_visible(&mut scene, "/Root", false);
    assert_eq!(clicked(&scene), None);
}

#[test]
fn reopening_the_screen_brings_its_rows_back() {
    // Nothing is latched: the row's own `visible` was never touched, so
    // showing the screen again is all it takes.
    let mut scene = scene();
    set_visible(&mut scene, "/Root/Screen", false);
    assert_ne!(clicked(&scene), Some(id_of(&scene, "/Root/Screen/Row4")));
    set_visible(&mut scene, "/Root/Screen", true);
    assert_eq!(clicked(&scene), Some(id_of(&scene, "/Root/Screen/Row4")));
}

#[test]
fn the_hit_test_never_returns_something_invisible() {
    // The property rather than examples of it: whatever `hit` returns has to
    // be something `Scene::is_visible` agrees is visible. The renderer asks
    // the same function, so this is also the statement that the two cannot
    // drift apart again.
    for root_on in [true, false] {
        for screen_on in [true, false] {
            for row_on in [true, false] {
                let mut scene = scene();
                set_visible(&mut scene, "/Root", root_on);
                set_visible(&mut scene, "/Root/Screen", screen_on);
                set_visible(&mut scene, "/Root/Screen/Row4", row_on);
                if let Some(id) = clicked(&scene) {
                    assert!(
                        scene.is_visible(id),
                        "hit {root_on}/{screen_on}/{row_on} returned something invisible"
                    );
                }
            }
        }
    }
}
