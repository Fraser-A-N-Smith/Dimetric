//! `TextureRect`: a control that draws a texture.
//!
//! The UI walk emitted two things — filled quads for `Panel` and `Button`, and
//! glyphs for a `Label` under a control — so an icon, a portrait or a rune on
//! the card holding it had to be a world-space `Sprite2D`, read at a world
//! position under the camera's projection and zoom. That is not where a control
//! is. Every button in a game built on this was a word instead.

use dimetric_core::{Fx, NodeUid};
use dimetric_render::atlas::Source;
use dimetric_render::{extract_with_canvas, Atlas, Camera, Projection};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{Node, Scene, Value};

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// Two 8x8 icons side by side, so a region picks one.
fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "ui/icons".into(),
                width: 16,
                height: 8,
                pixels: (0..16 * 8)
                    .flat_map(|i| {
                        let left = (i % 16) < 8;
                        match left {
                            true => [0xE0u8, 0x40, 0x40, 0xFF],
                            false => [0x40, 0x40, 0xE0, 0xFF],
                        }
                    })
                    .collect(),
            },
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
            dimetric_render::atlas::placeholder(8),
        ],
        128,
    )
}

fn canvas() -> Canvas {
    Canvas {
        width: 128,
        height: 64,
    }
}

/// One child of the card: its id, its kind, and the properties it carries.
type Child<'a> = (&'a str, &'a str, &'a [(&'a str, Value)]);

/// A panel with the given children appended to it.
fn ui_scene(children: &[Child<'_>]) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node", "Root"), None)
        .unwrap();
    let mut panel = Node::new(uid("n_panel000"), "Panel", "Card");
    panel.base = "Control".into();
    for (key, value) in [
        ("offset_left", 10i32),
        ("offset_top", 10),
        ("offset_right", 90),
        ("offset_bottom", 50),
    ] {
        panel
            .props
            .insert(key.into(), Value::Scalar(Fx::from_int(value)));
    }
    let panel = scene.insert(panel, Some(root)).unwrap();
    for (n, (id, kind, props)) in children.iter().enumerate() {
        let mut child = Node::new(uid(id), *kind, format!("Child{n}"));
        child.base = "Control".into();
        for (key, value) in props.iter() {
            child.props.insert((*key).into(), value.clone());
        }
        scene.insert(child, Some(panel)).unwrap();
    }
    scene.update_world_transforms();
    scene
}

fn frame(scene: &Scene) -> dimetric_render::Frame {
    let camera = Camera {
        projection: Projection::Isometric,
        ..Camera::new((128, 64))
    };
    extract_with_canvas(scene, &atlas(), &camera, None, canvas())
}

fn texture_ref(name: &str) -> Value {
    Value::Ref(dimetric_scene::Reference::Asset(name.into()))
}

/// Offsets that put a child at a known rectangle inside the card.
fn at(left: i32, top: i32, right: i32, bottom: i32) -> Vec<(&'static str, Value)> {
    vec![
        ("offset_left", Value::Scalar(Fx::from_int(left))),
        ("offset_top", Value::Scalar(Fx::from_int(top))),
        ("offset_right", Value::Scalar(Fx::from_int(right))),
        ("offset_bottom", Value::Scalar(Fx::from_int(bottom))),
    ]
}

#[test]
fn a_texture_rect_draws_in_the_ui_layer_at_its_control_rectangle() {
    let mut props = at(4, 4, 20, 20);
    props.push(("texture", texture_ref("ui/icons")));
    let props: Vec<(&str, Value)> = props;
    let scene = ui_scene(&[("n_icon0000", "TextureRect", &props)]);
    let frame = frame(&scene);

    assert!(
        frame.sprites.is_empty(),
        "a control must not also be drawn in the world: {:?}",
        frame.sprites.len()
    );
    assert_eq!(frame.ui.len(), 2, "the card and the icon");
    let icon = frame
        .ui
        .iter()
        .find(|i| i.node == uid("n_icon0000"))
        .expect("the icon is drawn");
    // Offsets are from the parent's top-left, so 10 + 4 across and down, over a
    // 16x16 box.
    assert_eq!((icon.size.x.round_int(), icon.size.y.round_int()), (16, 16));
    assert_eq!(
        (icon.pos.x.round_int(), icon.pos.y.round_int()),
        (14 + 8, 14 + 8),
        "the icon is not centred in its control rectangle"
    );
}

#[test]
fn it_draws_in_tree_order_with_the_panels_around_it() {
    // The hit test says later siblings win, and the draw order has to agree or
    // something invisible is still clickable.
    let mut backdrop = at(0, 0, 80, 40);
    backdrop.push(("texture", texture_ref("ui/icons")));
    let front = at(4, 4, 20, 20);
    let scene = ui_scene(&[
        ("n_back0000", "TextureRect", &backdrop),
        ("n_front000", "Panel", &front),
    ]);
    let frame = frame(&scene);
    let order: Vec<NodeUid> = frame.ui.iter().map(|i| i.node).collect();
    let back = order.iter().position(|u| *u == uid("n_back0000"));
    let front_at = order.iter().position(|u| *u == uid("n_front000"));
    assert!(back < front_at, "declared first must draw first: {order:?}");
}

#[test]
fn a_region_picks_part_of_the_sheet() {
    let mut props = at(4, 4, 20, 20);
    props.push(("texture", texture_ref("ui/icons")));
    props.push((
        "region",
        Value::Rect(dimetric_core::Rect::new(
            dimetric_core::Vec2Fx::from_ints(8, 0),
            dimetric_core::Vec2Fx::from_ints(8, 8),
        )),
    ));
    let scene = ui_scene(&[("n_icon0000", "TextureRect", &props)]);
    let icon = frame(&scene)
        .ui
        .into_iter()
        .find(|i| i.node == uid("n_icon0000"))
        .expect("drawn");
    // The right half of a 16-wide sheet: u starts halfway along the packed
    // region rather than at its left edge.
    let whole = atlas().region("ui/icons").expect("packed").uv;
    assert!(
        icon.uv[0] > whole[0],
        "the region did not move the slice: {:?} vs {:?}",
        icon.uv,
        whole
    );
}

#[test]
fn a_tint_applies() {
    let mut props = at(4, 4, 20, 20);
    props.push(("texture", texture_ref("ui/icons")));
    props.push((
        "modulate",
        Value::Color(dimetric_scene::Color::rgba(255, 143, 74, 200)),
    ));
    let scene = ui_scene(&[("n_icon0000", "TextureRect", &props)]);
    let icon = frame(&scene)
        .ui
        .into_iter()
        .find(|i| i.node == uid("n_icon0000"))
        .expect("drawn");
    assert_eq!(icon.modulate, [255, 143, 74, 200]);
}

#[test]
fn a_texture_rect_with_no_texture_draws_nothing_rather_than_panicking() {
    let props = at(4, 4, 20, 20);
    let scene = ui_scene(&[("n_icon0000", "TextureRect", &props)]);
    let frame = frame(&scene);
    assert_eq!(frame.ui.len(), 1, "only the card");
}

#[test]
fn a_missing_texture_draws_the_placeholder_rather_than_nothing() {
    // Same rule the world sprite follows: a magenta checkerboard says more than
    // a gap does.
    let mut props = at(4, 4, 20, 20);
    props.push(("texture", texture_ref("ui/not-imported")));
    let scene = ui_scene(&[("n_icon0000", "TextureRect", &props)]);
    let frame = frame(&scene);
    assert_eq!(frame.ui.len(), 2, "the card and a placeholder");
}
