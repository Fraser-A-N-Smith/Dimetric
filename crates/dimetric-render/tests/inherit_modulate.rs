//! A child sprite can wear its parent's tint.
//!
//! Visibility is inherited and tint was not. `extract::sprite` read `modulate`
//! off each node alone, so every hit flash, fade-in and death fade a view
//! tweened onto a figure left its child sprites at full strength: a dying
//! adventurer faded out from under a cloak that did not.
//!
//! The workaround was mirroring every one of those writes onto each child with
//! the colours multiplied in script — a copy of this rule, written in Lua, in
//! every tween.
//!
//! Opt-in, because making `modulate` inherit by default would change how every
//! scene already written looks. The Godot split (an inheriting `modulate` and a
//! standalone `self_modulate`) was offered and declined: it buys nothing this
//! does not, and it moves every existing scene to buy it. The first test here
//! is the one that holds that promise.

use dimetric_core::{Fx, NodeUid};
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{Color, KindRegistry, Scene};

const SIDE: u32 = 8;

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/body".into(),
                width: SIDE,
                height: SIDE,
                pixels: [0xFF; 4].repeat((SIDE * SIDE) as usize),
            },
            dimetric_render::atlas::solid(Color::WHITE),
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
    let out = dimetric_scene::parse(text, "t.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut scene = out.doc.unwrap().scene;
    scene.update_world_transforms();
    scene
}

/// A body sprite with a trim child, each with whatever extra lines are given.
fn figure(body: &str, trim: &str) -> Scene {
    parse(&format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_body0000\"\nkind = \"Sprite2D\"\nname = \"Body\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/body\"\n{body}\
         \n[[node]]\nid = \"n_trim0000\"\nkind = \"Sprite2D\"\nname = \"Trim\"\n\
         parent = \"n_body0000\"\ntexture = \"asset:sprites/body\"\n{trim}"
    ))
}

/// The colour a named node is actually drawn with.
fn drawn(scene: &Scene, uid: &str) -> [u8; 4] {
    let want = NodeUid::parse(uid).expect("a node id");
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(scene, &atlas, &camera, None, canvas());
    frame
        .sprites
        .iter()
        .chain(frame.ui.iter())
        .find(|i| i.node == want)
        .unwrap_or_else(|| panic!("{uid} was not drawn"))
        .modulate
}

#[test]
fn a_scene_that_does_not_ask_is_unchanged() {
    // The promise that made this opt-in rather than the Godot split. A tinted
    // parent must leave an un-asking child exactly as it was.
    for trim in ["", "inherit_modulate = false\n"] {
        let scene = figure("modulate = \"#ff8000ff\"\n", trim);
        assert_eq!(drawn(&scene, "n_body0000"), [0xFF, 0x80, 0x00, 0xFF]);
        assert_eq!(
            drawn(&scene, "n_trim0000"),
            [0xFF, 0xFF, 0xFF, 0xFF],
            "{trim:?}: the child stands alone"
        );
    }
}

#[test]
fn an_inheriting_child_wears_the_parents_colour() {
    // The cape in the colour of the run's first essence: greys over the body,
    // tinted to the essence by the parent it hangs on.
    let scene = figure("modulate = \"#ff8000ff\"\n", "inherit_modulate = true\n");
    assert_eq!(
        drawn(&scene, "n_trim0000"),
        [0xFF, 0x80, 0x00, 0xFF],
        "white times the parent is the parent"
    );
}

#[test]
fn the_childs_own_colour_still_counts() {
    // It is a multiply, not a replacement: a cape painted half-strength grey
    // under an orange body comes out half-strength orange.
    let scene = figure(
        "modulate = \"#ff8000ff\"\n",
        "inherit_modulate = true\nmodulate = \"#808080ff\"\n",
    );
    let [r, g, b, a] = drawn(&scene, "n_trim0000");
    // 0xFF * 0x80 / 255 = 128, 0x80 * 0x80 / 255 = 64, 0x00 * anything = 0.
    assert_eq!([r, g, b, a], [0x80, 0x40, 0x00, 0xFF]);
}

#[test]
fn a_fade_on_the_parent_fades_the_child_with_it() {
    // The defect, stated as the thing it broke. A death fade is alpha on the
    // body; before this the cloak stayed opaque over a vanishing figure.
    let scene = figure("modulate = \"#ffffff40\"\n", "inherit_modulate = true\n");
    assert_eq!(
        drawn(&scene, "n_trim0000")[3],
        0x40,
        "the child's alpha follows the parent's"
    );
}

#[test]
fn white_is_exactly_the_identity() {
    // Why the multiply truncates rather than rounds: `255 * v / 255 == v` for
    // every v, so an untinted parent leaves a child bit-identical. A multiply
    // that was nearly the identity would shift every inheriting sprite by a
    // channel step and no golden would ever settle.
    for shade in ["#00000000", "#123456ff", "#ffffffff", "#7f7f7f7f"] {
        let scene = figure(
            "",
            &format!("inherit_modulate = true\nmodulate = \"{shade}\"\n"),
        );
        let alone = figure("", &format!("modulate = \"{shade}\"\n"));
        assert_eq!(
            drawn(&scene, "n_trim0000"),
            drawn(&alone, "n_trim0000"),
            "{shade} under a white parent"
        );
    }
}

#[test]
fn the_chain_carries_through_an_inheriting_parent() {
    // Three deep, each multiplying the one above: a trim on a pauldron on a
    // body. The middle has to pass its product down, not just its own colour.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_body0000\"\nkind = \"Sprite2D\"\nname = \"Body\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/body\"\n\
         modulate = \"#80ffffff\"\n\
         \n[[node]]\nid = \"n_pald0000\"\nkind = \"Sprite2D\"\nname = \"Pauldron\"\n\
         parent = \"n_body0000\"\ntexture = \"asset:sprites/body\"\n\
         inherit_modulate = true\nmodulate = \"#ff80ffff\"\n\
         \n[[node]]\nid = \"n_trim0000\"\nkind = \"Sprite2D\"\nname = \"Trim\"\n\
         parent = \"n_pald0000\"\ntexture = \"asset:sprites/body\"\n\
         inherit_modulate = true\nmodulate = \"#ffff80ff\"\n",
    );
    assert_eq!(drawn(&scene, "n_pald0000"), [0x80, 0x80, 0xFF, 0xFF]);
    assert_eq!(
        drawn(&scene, "n_trim0000"),
        [0x80, 0x80, 0x80, 0xFF],
        "all three multiplied together"
    );
}

#[test]
fn a_parent_that_does_not_inherit_stops_the_chain() {
    // The documented rule, pinned. The value a child multiplies by is exactly
    // what its parent looks like -- so a middle node standing alone passes
    // only its own colour down, and nothing above it leaks through. That is
    // the only version of this rule that can be stated in one sentence, and it
    // is why the flag is per node.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_body0000\"\nkind = \"Sprite2D\"\nname = \"Body\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/body\"\n\
         modulate = \"#ff0000ff\"\n\
         \n[[node]]\nid = \"n_pald0000\"\nkind = \"Sprite2D\"\nname = \"Pauldron\"\n\
         parent = \"n_body0000\"\ntexture = \"asset:sprites/body\"\n\
         modulate = \"#00ff00ff\"\n\
         \n[[node]]\nid = \"n_trim0000\"\nkind = \"Sprite2D\"\nname = \"Trim\"\n\
         parent = \"n_pald0000\"\ntexture = \"asset:sprites/body\"\n\
         inherit_modulate = true\n",
    );
    assert_eq!(
        drawn(&scene, "n_trim0000"),
        [0x00, 0xFF, 0x00, 0xFF],
        "the pauldron's green, with none of the body's red"
    );
}

#[test]
fn a_texture_rect_inherits_from_its_panel() {
    // The UI half. An icon on a card fades with the card, which is the same
    // defect in the other walk -- and the reason the chain is resolved once
    // for the whole tree rather than per pass.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"Panel\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\nmodulate = \"#ffffff80\"\n\
         offset_right = 40.0\noffset_bottom = 40.0\n\
         \n[[node]]\nid = \"n_icon0000\"\nkind = \"TextureRect\"\nname = \"Icon\"\n\
         parent = \"n_card0000\"\ntexture = \"asset:sprites/body\"\n\
         inherit_modulate = true\noffset_right = 16.0\noffset_bottom = 16.0\n",
    );
    assert_eq!(
        drawn(&scene, "n_icon0000")[3],
        0x80,
        "the icon fades with the card it sits on"
    );
}

#[test]
fn an_inherited_tint_reaches_a_sliced_frame() {
    // Nine quads from one node, so every one of them has to carry the tint --
    // a frame where one cell faded and eight did not would be worse than none
    // fading.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_card0000\"\nkind = \"Panel\"\nname = \"Card\"\n\
         parent = \"n_root0000\"\nmodulate = \"#ff8000ff\"\n\
         offset_right = 60.0\noffset_bottom = 60.0\n\
         \n[[node]]\nid = \"n_icon0000\"\nkind = \"TextureRect\"\nname = \"Icon\"\n\
         parent = \"n_card0000\"\ntexture = \"asset:sprites/body\"\n\
         inherit_modulate = true\nslice = [2.0, 2.0, 2.0, 2.0]\n\
         offset_right = 40.0\noffset_bottom = 40.0\n",
    );
    let want = NodeUid::parse("n_icon0000").expect("a node id");
    let atlas = atlas();
    let camera = Camera::new((320, 180));
    let frame = extract::extract_with_canvas(&scene, &atlas, &camera, None, canvas());
    let cells: Vec<[u8; 4]> = frame
        .ui
        .iter()
        .filter(|i| i.node == want)
        .map(|i| i.modulate)
        .collect();
    assert_eq!(cells.len(), 9, "a sliced frame");
    for (index, cell) in cells.iter().enumerate() {
        assert_eq!(*cell, [0xFF, 0x80, 0x00, 0xFF], "cell {index}");
    }
}

#[test]
fn an_inherited_tint_never_reaches_the_simulation() {
    // I7, pinned. The chain is resolved in extraction and nothing is written
    // back, so the child's own `modulate` in state is still whatever the scene
    // said -- which is what keeps this render-only and the hash still.
    let scene = figure(
        "modulate = \"#ff8000ff\"\n",
        "inherit_modulate = true\nmodulate = \"#808080ff\"\n",
    );
    let before = scene
        .by_uid(NodeUid::parse("n_trim0000").unwrap())
        .and_then(|id| scene.get(id))
        .and_then(|n| n.get("modulate"))
        .and_then(dimetric_scene::Value::as_color)
        .expect("the trim's own colour");
    assert_eq!(drawn(&scene, "n_trim0000"), [0x80, 0x40, 0x00, 0xFF]);
    let after = scene
        .by_uid(NodeUid::parse("n_trim0000").unwrap())
        .and_then(|id| scene.get(id))
        .and_then(|n| n.get("modulate"))
        .and_then(dimetric_scene::Value::as_color)
        .expect("the trim's own colour");
    assert_eq!(before, after, "extraction wrote into the scene");
    assert_eq!(
        (after.r, after.g, after.b),
        (0x80, 0x80, 0x80),
        "still the grey the scene authored"
    );
    let _ = Fx::ONE;
}
