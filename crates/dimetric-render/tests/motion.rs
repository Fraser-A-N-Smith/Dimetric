//! Interpolating from a position per node, rather than from a copy of the state.
//!
//! A host kept a whole `SimState` clone every tick purely so the next frame
//! could interpolate. Cloning the state clones every node's properties and
//! every script variable, so the cost of *drawing* rose with everything a game
//! keeps: a combat log filling over a floor made the fortieth round of a floor
//! more expensive to draw than the first, for a frame that reads one value per
//! node.
//!
//! The one value is the node's previous world position. These tests hold the
//! new store to the old answer: whatever the tree looks like, interpolating
//! from a `Motion` has to land exactly where interpolating from the cloned
//! scene did, because the picture must not move.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera, Interpolation, Motion};
use dimetric_scene::{KindRegistry, Scene};

const SIDE: u32 = 8;

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/block".into(),
                width: SIDE,
                height: SIDE,
                pixels: [0xFF; 4].repeat((SIDE * SIDE) as usize),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        64,
    )
}

/// A parent at `parent` with a child offset locally, so a world position is a
/// composition rather than a copy of a local one.
fn nested(parent: (i32, i32), child: (i32, i32)) -> Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         \n[[node]]\nid = \"n_body0000\"\nkind = \"Sprite2D\"\nname = \"Body\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/block\"\n\
         pos = [{}.0, {}.0]\n\
         \n[[node]]\nid = \"n_trim0000\"\nkind = \"Sprite2D\"\nname = \"Trim\"\n\
         parent = \"n_body0000\"\ntexture = \"asset:sprites/block\"\n\
         pos = [{}.0, {}.0]\n",
        parent.0, parent.1, child.0, child.1
    );
    let out = dimetric_scene::parse(&text, "m.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut scene = out.doc.unwrap().scene;
    scene.update_world_transforms();
    scene
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).expect("a node id")
}

/// Where a node is drawn, given a previous `Motion` and an alpha.
fn drawn_at(current: &Scene, previous: &Motion, alpha: f32, node: NodeUid) -> Vec2Fx {
    let atlas = atlas();
    let camera = Camera::new((64, 64));
    extract(
        current,
        &atlas,
        &camera,
        Some(Interpolation { previous, alpha }),
    )
    .sprites
    .iter()
    .find(|i| i.node == node)
    .unwrap_or_else(|| panic!("{node:?} was not drawn"))
    .pos
}

/// What the old code computed: the previous *scene's* world position, blended.
///
/// Kept here deliberately as the oracle. It is the expression
/// `extract::interpolated` used to evaluate against a cloned `SimState`, so if
/// the new store ever disagrees with it the picture has moved.
fn the_old_way(previous: &Scene, current: &Scene, alpha: f32, node: NodeUid) -> Vec2Fx {
    let before = previous
        .by_uid(node)
        .and_then(|id| previous.world_of(id))
        .map(|t| t.pos)
        .expect("the node was there before");
    let now = current
        .by_uid(node)
        .and_then(|id| current.world_of(id))
        .map(|t| t.pos)
        .expect("the node is there now");
    before.lerp(now, Fx::from_f64_lossy(alpha as f64))
}

#[test]
fn a_motion_interpolates_exactly_where_a_cloned_state_did() {
    // The invariant this whole change rests on. Several alphas, a parent that
    // moved and a child that moved with it, against the expression the old
    // code evaluated.
    let previous_scene = nested((0, 0), (3, 5));
    let current = nested((16, 8), (3, 5));
    let mut previous = Motion::new();
    previous.refresh(&previous_scene);

    for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
        for node in ["n_body0000", "n_trim0000"] {
            let node = uid(node);
            assert_eq!(
                drawn_at(&current, &previous, alpha, node),
                the_old_way(&previous_scene, &current, alpha, node),
                "alpha {alpha} for {node:?}"
            );
        }
    }
}

#[test]
fn a_childs_recorded_position_is_its_world_one() {
    // The composition is what makes this more than a copy of `pos`: a child
    // three units along inside a parent at sixteen was at nineteen, and that is
    // what a frame has to blend from.
    let scene = nested((16, 8), (3, 5));
    let mut motion = Motion::new();
    motion.refresh(&scene);
    assert_eq!(
        motion.get(uid("n_body0000")),
        Some(Vec2Fx::from_ints(16, 8))
    );
    assert_eq!(
        motion.get(uid("n_trim0000")),
        Some(Vec2Fx::from_ints(19, 13))
    );
}

#[test]
fn a_node_that_was_not_there_is_drawn_where_it_is() {
    // Sliding a newly spawned node in from a position it never occupied looks
    // worse than popping it in, so an unrecorded uid is not interpolated.
    let current = nested((16, 8), (3, 5));
    let previous = Motion::new();
    assert!(previous.is_empty());
    // An empty store still answers, and answers nothing.
    assert_eq!(previous.get(uid("n_body0000")), None);
    assert_eq!(
        drawn_at(&current, &previous, 0.5, uid("n_body0000")),
        Vec2Fx::from_ints(16, 8),
        "drawn where it actually is"
    );
}

#[test]
fn refreshing_replaces_rather_than_accumulates() {
    // It is rebuilt wholesale every tick and reuses its allocation, so a stale
    // entry from two ticks ago must not survive -- that would tween a node from
    // somewhere it had already left.
    let mut motion = Motion::new();
    motion.refresh(&nested((0, 0), (0, 0)));
    assert_eq!(motion.get(uid("n_body0000")), Some(Vec2Fx::ZERO));
    motion.refresh(&nested((40, 40), (0, 0)));
    assert_eq!(
        motion.get(uid("n_body0000")),
        Some(Vec2Fx::from_ints(40, 40)),
        "the newest refresh is the answer"
    );

    // And a tree that shrank does not keep what it lost.
    let empty = dimetric_scene::Scene::new();
    motion.refresh(&empty);
    assert!(motion.is_empty());
    assert_eq!(motion.get(uid("n_body0000")), None);
}

#[test]
fn clearing_forgets_a_tree_that_went_away() {
    // What a scene swap does. A uid from the old tree could be live in the new
    // one, which would tween a node from somewhere it never was.
    let mut motion = Motion::new();
    motion.refresh(&nested((7, 7), (0, 0)));
    assert!(!motion.is_empty());
    motion.clear();
    assert!(motion.is_empty());
    assert_eq!(motion.get(uid("n_body0000")), None);
}

#[test]
fn every_node_in_the_tree_is_recorded_not_only_the_drawn_ones() {
    // A `Node2D` draws nothing and is still somebody's parent, and a uid is
    // looked up by whatever is drawn -- so the store covers the tree rather
    // than the draw list.
    let scene = nested((1, 2), (3, 4));
    let mut motion = Motion::new();
    motion.refresh(&scene);
    assert_eq!(motion.get(uid("n_root0000")), Some(Vec2Fx::ZERO));
    for id in scene.walk() {
        let node = scene.get(id).expect("a node");
        assert_eq!(
            motion.get(node.uid),
            scene.world_of(id).map(|t| t.pos),
            "{} was not recorded as its world position",
            node.name
        );
    }
}
