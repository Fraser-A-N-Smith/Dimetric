//! Every node's world position, read in one pass.
//!
//! `world_of` walks *up* from a node, composing once per ancestor, and its own
//! doc comment tells a caller touching many nodes to use
//! `update_world_transforms` plus `Node::world` instead. A renderer holds only
//! a `&Scene`, so it could do neither: the cache needs `&mut` and is stale on a
//! tree that has never been stepped, and the recursive version was measured
//! costing more per frame than the state clone it was meant to replace.
//!
//! So there is a third door, and the only thing that matters about it is that
//! it agrees with the first one exactly — a renderer that interpolated from
//! almost-right positions would move every sprite by a sub-pixel amount and no
//! golden would ever settle. These tests are that agreement, over the cases
//! where composition is more than addition: a rotated parent, a scaled parent,
//! and a chain of both.

use dimetric_core::{Angle, Fx, Vec2Fx};
use dimetric_scene::{KindRegistry, Scene};

fn parse(text: &str) -> Scene {
    let out = dimetric_scene::parse(text, "w.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// Assert the bulk read agrees with `world_of` for every node in the tree.
///
/// The oracle is deliberately the function this one exists to avoid calling per
/// node: if the two ever disagree, the picture has moved.
fn agrees(scene: &Scene) {
    let mut bulk = Vec::new();
    scene.world_positions(&mut bulk);
    let by_uid: std::collections::BTreeMap<_, _> = bulk.iter().copied().collect();

    let mut seen = 0;
    for id in scene.walk() {
        let node = scene.get(id).expect("a node");
        let expected = scene.world_of(id).map(|t| t.pos).expect("a transform");
        assert_eq!(
            by_uid.get(&node.uid).copied(),
            Some(expected),
            "{} disagreed",
            node.name
        );
        seen += 1;
    }
    assert_eq!(bulk.len(), seen, "one entry per node, no more and no fewer");
}

#[test]
fn a_plain_chain_agrees() {
    agrees(&parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         pos = [5.0, 7.0]\n\
         \n[[node]]\nid = \"n_aaaa0000\"\nkind = \"Node2D\"\nname = \"A\"\n\
         parent = \"n_root0000\"\npos = [10.0, 0.0]\n\
         \n[[node]]\nid = \"n_bbbb0000\"\nkind = \"Node2D\"\nname = \"B\"\n\
         parent = \"n_aaaa0000\"\npos = [0.0, 10.0]\n\
         \n[[node]]\nid = \"n_cccc0000\"\nkind = \"Node2D\"\nname = \"C\"\n\
         parent = \"n_bbbb0000\"\npos = [-3.0, -4.0]\n",
    ));
}

#[test]
fn a_rotated_parent_agrees() {
    // The case that makes composition more than addition: a child's world
    // position is its local offset turned by every rotation above it. An
    // implementation that only added positions would pass every other test
    // here and fail this one.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_turn0000\"\nkind = \"Node2D\"\nname = \"Turn\"\n\
         parent = \"n_root0000\"\npos = [20.0, 20.0]\nrot = 90.0\n\
         \n[[node]]\nid = \"n_arm00000\"\nkind = \"Node2D\"\nname = \"Arm\"\n\
         parent = \"n_turn0000\"\npos = [10.0, 0.0]\n\
         \n[[node]]\nid = \"n_hand0000\"\nkind = \"Node2D\"\nname = \"Hand\"\n\
         parent = \"n_arm00000\"\npos = [0.0, 6.0]\nrot = 45.0\n",
    );
    agrees(&scene);

    // And it really is turned, so the test above is not comparing two copies
    // of the same mistake: ten units along a parent turned a quarter is ten
    // units down from it, not ten along.
    let arm = scene.resolve_path("/World/Turn/Arm").expect("the arm");
    let at = scene.world_of(arm).expect("a transform").pos;
    assert_ne!(at, Vec2Fx::from_ints(30, 20), "it was not turned at all");
    assert!(
        (at.y - Fx::from_int(30)).abs() < Fx::ONE,
        "a quarter turn puts it below the shoulder, at {at:?}"
    );
    let _ = Angle::ZERO;
}

#[test]
fn a_scaled_parent_agrees() {
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_big00000\"\nkind = \"Node2D\"\nname = \"Big\"\n\
         parent = \"n_root0000\"\npos = [4.0, 4.0]\nscale = [3.0, 2.0]\n\
         \n[[node]]\nid = \"n_pip00000\"\nkind = \"Node2D\"\nname = \"Pip\"\n\
         parent = \"n_big00000\"\npos = [5.0, 5.0]\n",
    );
    agrees(&scene);
    let pip = scene.resolve_path("/World/Big/Pip").expect("the pip");
    assert_eq!(
        scene.world_of(pip).expect("a transform").pos,
        Vec2Fx::from_ints(19, 14),
        "4 + 5*3 across and 4 + 5*2 down"
    );
}

#[test]
fn a_wide_tree_agrees_and_is_complete() {
    // Many siblings under one parent, which is the shape a tile layer or a
    // crowd of actors has -- and the shape where the traversal's sibling
    // handling could drop or duplicate a node.
    let mut text = String::from(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         pos = [1.0, 2.0]\n",
    );
    for i in 0..64 {
        text.push_str(&format!(
            "\n[[node]]\nid = \"n_{i:08}\"\nkind = \"Node2D\"\nname = \"N{i}\"\n\
             parent = \"n_root0000\"\npos = [{i}.0, {}.0]\n",
            i * 2
        ));
    }
    let scene = parse(&text);
    agrees(&scene);
    let mut bulk = Vec::new();
    scene.world_positions(&mut bulk);
    assert_eq!(bulk.len(), 65, "the root and sixty-four children");
}

#[test]
fn an_empty_scene_reads_as_nothing() {
    let scene = Scene::new();
    let mut bulk = vec![(
        dimetric_core::NodeUid::parse("n_stale000").expect("an id"),
        Vec2Fx::from_ints(9, 9),
    )];
    scene.world_positions(&mut bulk);
    assert!(bulk.is_empty(), "the buffer is cleared, not appended to");
}

#[test]
fn the_buffer_is_refilled_rather_than_grown() {
    // A caller keeps one buffer across ticks to avoid allocating, so a stale
    // entry surviving a refill would tween a node from somewhere it had left.
    let one = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_aaaa0000\"\nkind = \"Node2D\"\nname = \"A\"\n\
         parent = \"n_root0000\"\npos = [1.0, 1.0]\n",
    );
    let two = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n",
    );
    let mut bulk = Vec::new();
    one.world_positions(&mut bulk);
    assert_eq!(bulk.len(), 2);
    two.world_positions(&mut bulk);
    assert_eq!(bulk.len(), 1, "the smaller tree does not keep the larger's");
}

#[test]
fn it_does_not_read_the_cache_and_so_works_on_a_tree_never_stepped() {
    // The reason this exists rather than `Node::world`. A freshly parsed tree
    // has never had `update_world_transforms` run on it, so the cache says
    // whatever it was initialised to -- and a frame drawn between tick zero and
    // tick one reads exactly this.
    let scene = parse(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         pos = [100.0, 50.0]\n\
         \n[[node]]\nid = \"n_aaaa0000\"\nkind = \"Node2D\"\nname = \"A\"\n\
         parent = \"n_root0000\"\npos = [7.0, 3.0]\n",
    );
    let mut bulk = Vec::new();
    scene.world_positions(&mut bulk);
    let by_uid: std::collections::BTreeMap<_, _> = bulk.into_iter().collect();
    let a = dimetric_core::NodeUid::parse("n_aaaa0000").expect("an id");
    assert_eq!(
        by_uid.get(&a).copied(),
        Some(Vec2Fx::from_ints(107, 53)),
        "composed from the file, with no propagation pass having run"
    );
}
