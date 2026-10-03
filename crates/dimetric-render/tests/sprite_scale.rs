//! `scale` is drawn.
//!
//! It is a key every node has, `tween.rs` writes it into the transform, and
//! `extract.rs` never read it: the quad was the texture region's size, always.
//! So a scale tween ran, was hashed, was snapshotted, and changed nothing on
//! screen — a health bar could not shrink, and nothing could squash on a hit.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera, Projection};
use dimetric_scene::{Node, Scene, Value};

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/block".into(),
                width: 16,
                height: 8,
                pixels: [0xC8u8, 0x28, 0x28, 0xFF].repeat(16 * 8),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        128,
    )
}

/// One 16x8 sprite, optionally under a scaled parent.
fn scene(own: Option<[i32; 2]>, parent_scale: Option<[i32; 2]>) -> Scene {
    let mut scene = Scene::new();
    let mut root = Node::new(uid("n_root0000"), "Node2D", "Root");
    if let Some([x, y]) = parent_scale {
        root.transform.scale = Vec2Fx::from_ints(x, y);
    }
    let root = scene.insert(root, None).unwrap();
    let mut sprite = Node::new(uid("n_block001"), "Sprite2D", "Block");
    sprite.props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/block".into())),
    );
    if let Some([x, y]) = own {
        sprite.transform.scale = Vec2Fx::from_ints(x, y);
    }
    scene.insert(sprite, Some(root)).unwrap();
    scene.update_world_transforms();
    scene
}

/// The quad's size, in pixels.
fn drawn(scene: &Scene) -> (i32, i32) {
    let camera = Camera {
        projection: Projection::TopDown,
        ..Camera::new((64, 64))
    };
    let frame = extract(scene, &atlas(), &camera, None);
    assert_eq!(frame.sprites.len(), 1);
    let s = frame.sprites[0].size;
    (s.x.round_int(), s.y.round_int())
}

#[test]
fn an_unscaled_sprite_is_the_size_of_its_region() {
    assert_eq!(drawn(&scene(None, None)), (16, 8));
}

#[test]
fn a_scaled_sprite_draws_scaled() {
    assert_eq!(drawn(&scene(Some([2, 1]), None)), (32, 8));
    assert_eq!(drawn(&scene(Some([3, 2]), None)), (48, 16));
}

#[test]
fn the_scale_is_the_world_scale_so_a_parent_scales_its_children() {
    // Which is what makes it worth living on the transform rather than on the
    // sprite: scaling a container scales what is in it.
    assert_eq!(drawn(&scene(None, Some([2, 2]))), (32, 16));
    assert_eq!(drawn(&scene(Some([2, 1]), Some([2, 2]))), (64, 16));
}

#[test]
fn a_negative_component_mirrors_the_quad() {
    // The shader builds the quad from `(corner - 0.5) * size`, so a negative
    // size mirrors it. That makes `flip_h` sugar for `scale = [-1, 1]`.
    let (w, h) = drawn(&scene(Some([-1, 1]), None));
    assert_eq!((w, h), (-16, 8));
}

#[test]
fn a_fractional_scale_lands_where_fixed_point_puts_it() {
    // No float anywhere: `Vec2Fx::mul_components` is the same multiply the
    // simulation does, so a tween's halfway point draws the same everywhere.
    let mut s = scene(None, None);
    let id = s.resolve_path("/Root/Block").expect("block");
    if let Some(node) = s.node_mut_no_transform(id) {
        node.transform.scale = Vec2Fx::new(dimetric_core::Fx::ONE / 2, dimetric_core::Fx::ONE / 4);
    }
    s.update_world_transforms();
    assert_eq!(drawn(&s), (8, 2));
}

#[test]
fn a_tile_layer_is_not_scaled_by_this() {
    // Deliberate: the sprite size and the step between cells are different
    // numbers, and scaling one without the other breaks the lattice. A scaled
    // board wants a deliberate answer, not this one.
    let mut scene = Scene::new();
    let mut root = Node::new(uid("n_root0000"), "Node2D", "Root");
    root.transform.scale = Vec2Fx::from_ints(2, 2);
    let root = scene.insert(root, None).unwrap();
    let mut layer = Node::new(uid("n_floor000"), "TileLayer", "Floor");
    layer.props.insert(
        "tileset".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/block".into())),
    );
    layer.props.insert("cell".into(), Value::Vec2i([16, 8]));
    let layer_uid = layer.uid;
    scene.insert(layer, Some(root)).unwrap();
    dimetric_scene::chunk::set_tile_in(&mut scene.chunks, layer_uid, 0, 0, 1).unwrap();
    scene.update_world_transforms();
    assert_eq!(drawn(&scene), (16, 8), "a tile followed the node's scale");
}
