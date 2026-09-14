//! Rendering tests that put pixels through a real device.
//!
//! These skip themselves when no graphics adapter is available, so
//! `cargo test --workspace` works on any machine. CI installs a software
//! adapter and sets `DIMETRIC_REQUIRE_GPU`, which turns the skip into a
//! failure — otherwise a runner that lost its driver would go green while
//! testing nothing.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, Projection, RenderSettings, Renderer,
};
use dimetric_scene::{Color, Node, Scene, Value};

/// A 16x16 atlas holding one flat red square under the name `sprites/block`.
fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/block".into(),
                width: 16,
                height: 16,
                pixels: [0xC8u8, 0x28, 0x28, 0xFF].repeat(16 * 16),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        256,
    )
}

fn settings() -> RenderSettings {
    RenderSettings {
        internal_resolution: (64, 64),
        integer_upscale: true,
        pixel_snap: true,
        ambient: Color::WHITE,
    }
}

/// A scene with one 16x16 sprite at the origin.
fn one_sprite() -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .unwrap();
    let mut sprite = Node::new(uid("n_block001"), "Sprite2D", "Block");
    sprite.props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/block".into())),
    );
    scene.insert(sprite, Some(root)).unwrap();
    scene.update_world_transforms();
    scene
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// A renderer, or `None` when this machine has no adapter.
fn renderer(atlas: &Atlas, settings: RenderSettings) -> Option<Renderer> {
    let instance = headless_instance();
    match Renderer::new(&instance, None, atlas, settings) {
        Ok(renderer) => {
            eprintln!("adapter: {}", renderer.adapter_info.name);
            Some(renderer)
        }
        Err(e) => {
            assert!(
                std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                "DIMETRIC_REQUIRE_GPU is set but no adapter was available: {e}"
            );
            eprintln!("skipping: {e}");
            None
        }
    }
}

/// Pixel at `(x, y)` as RGBA.
fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

#[test]
fn a_sprite_lands_where_the_camera_says_it_should() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let scene = one_sprite();
    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let frame = extract(&scene, &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 1, "one sprite should be extracted");
    assert_eq!(frame.draw_calls(), 1, "and it should be one draw call");

    let pixels = capture.render(&mut renderer, &frame).unwrap();

    // The sprite is 16x16 at the origin, the camera is centred on the origin,
    // so it occupies the middle 16 pixels of a 64x64 frame.
    let centre = at(&pixels, 64, 32, 32);
    assert_eq!(
        &centre[..3],
        &[0xC8, 0x28, 0x28],
        "centre should be the sprite"
    );
    let corner = at(&pixels, 64, 2, 2);
    assert_eq!(corner[3], 255, "the frame should be opaque");
    assert_ne!(
        &corner[..3],
        &[0xC8, 0x28, 0x28],
        "the corner should not be"
    );

    // Count the sprite's pixels: 16x16 at zoom 1 in a 64x64 frame.
    let drawn = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] == [0xC8, 0x28, 0x28])
        .count();
    assert_eq!(
        drawn,
        16 * 16,
        "the sprite should cover exactly its own area"
    );
}

#[test]
fn moving_a_node_moves_its_sprite() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let mut scene = one_sprite();
    let block = scene.resolve_path("/Root/Block").unwrap();
    scene.set_position(block, Vec2Fx::from_ints(16, 0));
    scene.update_world_transforms();

    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let pixels = capture
        .render(&mut renderer, &extract(&scene, &atlas, &camera, None))
        .unwrap();

    let is_sprite = |x: u32, y: u32| at(&pixels, 64, x, y)[..3] == [0xC8, 0x28, 0x28];
    assert!(is_sprite(48, 32), "the sprite should have moved right");
    assert!(!is_sprite(32, 32), "and left the centre");
}

#[test]
fn interpolation_draws_between_two_states_without_touching_either() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let previous = one_sprite();
    let mut current = one_sprite();
    let block = current.resolve_path("/Root/Block").unwrap();
    current.set_position(block, Vec2Fx::from_ints(16, 0));
    current.update_world_transforms();

    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let frame = extract(
        &current,
        &atlas,
        &camera,
        Some(dimetric_render::Interpolation {
            previous: &previous,
            alpha: 0.5,
        }),
    );

    // Halfway between 0 and 16 is 8.
    assert_eq!(frame.sprites[0].pos.x, Fx::from_int(8));

    let pixels = capture.render(&mut renderer, &frame).unwrap();
    assert_eq!(
        at(&pixels, 64, 40, 32)[..3],
        [0xC8, 0x28, 0x28],
        "the sprite should be drawn halfway"
    );

    // I7: the states it read are unchanged.
    assert_eq!(
        current.get(block).unwrap().transform.pos,
        Vec2Fx::from_ints(16, 0)
    );
    let before = previous.resolve_path("/Root/Block").unwrap();
    assert_eq!(previous.get(before).unwrap().transform.pos, Vec2Fx::ZERO);
}

#[test]
fn an_invisible_node_hides_its_children_too() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let mut scene = one_sprite();
    let root = scene.root().unwrap();
    scene.node_mut_no_transform(root).unwrap().visible = false;

    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let frame = extract(&scene, &atlas, &camera, None);
    assert!(
        frame.sprites.is_empty(),
        "nothing under a hidden node draws"
    );

    let pixels = capture.render(&mut renderer, &frame).unwrap();
    assert!(
        !pixels.chunks_exact(4).any(|p| p[..3] == [0xC8, 0x28, 0x28]),
        "and nothing reaches the frame"
    );
}

#[test]
fn a_missing_texture_draws_the_placeholder_rather_than_nothing() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let mut scene = one_sprite();
    let block = scene.resolve_path("/Root/Block").unwrap();
    scene.node_mut_no_transform(block).unwrap().props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/nope".into())),
    );

    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let frame = extract(&scene, &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 1, "a missing texture still draws");

    let pixels = capture.render(&mut renderer, &frame).unwrap();
    // The placeholder's magenta, so a broken reference is obvious in a
    // screenshot instead of being an absence nobody notices.
    assert!(
        pixels.chunks_exact(4).any(|p| p[..3] == [0xE0, 0x3F, 0xB0]),
        "the placeholder checkerboard should be visible"
    );
}

#[test]
fn both_projections_render_and_disagree() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));

    let mut scene = one_sprite();
    let block = scene.resolve_path("/Root/Block").unwrap();
    scene.set_position(block, Vec2Fx::from_ints(0, 16));
    scene.update_world_transforms();

    let render_with = |renderer: &mut Renderer, projection| {
        let mut camera = Camera::new((64, 64));
        camera.zoom = 1.0;
        camera.projection = projection;
        capture
            .render(renderer, &extract(&scene, &atlas, &camera, None))
            .unwrap()
    };

    let top_down = render_with(&mut renderer, Projection::TopDown);
    let isometric = render_with(&mut renderer, Projection::Isometric);
    assert_ne!(
        top_down, isometric,
        "the shear must actually reach the GPU, not be silently dropped"
    );

    // The sprite sits at world (0, 16) with the camera at the origin.
    //
    // Top-down puts it straight below the centre: screen (0, 16), pixel
    // (32, 48). The shear maps it to (x - y, (x + y) / 2) = (-16, 8), which is
    // pixel (16, 40) — down, and half as far down as it went left.
    //
    // Only the centre is asserted. The quad itself is drawn upright under both
    // projections, because the projection moves a sprite's position and not its
    // shape, so checking a corner would be checking the quad and not the
    // transform.
    let sprite_at = |pixels: &[u8], x: u32, y: u32| at(pixels, 64, x, y)[..3] == [0xC8, 0x28, 0x28];
    assert!(sprite_at(&top_down, 32, 48), "top-down: straight down");
    assert!(sprite_at(&isometric, 16, 40), "isometric: down and left");
}

#[test]
fn rendering_the_same_frame_twice_gives_the_same_pixels() {
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));
    let scene = one_sprite();
    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    let frame = extract(&scene, &atlas, &camera, None);

    let first = capture.render(&mut renderer, &frame).unwrap();
    let second = capture.render(&mut renderer, &frame).unwrap();
    assert_eq!(first, second, "one adapter must be repeatable");
}

#[test]
fn the_shear_moves_a_sprite_without_deforming_it() {
    // Isometric artwork is already drawn in projection, so the engine projects
    // where a sprite is and leaves what it looks like alone. Shearing the quad
    // as well turns every character into a parallelogram — which is what the
    // first version of this renderer did, and it is obvious in a screenshot
    // and invisible to a test that only checks the sprite is somewhere.
    let atlas = atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));
    let scene = one_sprite();

    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;
    camera.projection = Projection::Isometric;
    let pixels = capture
        .render(&mut renderer, &extract(&scene, &atlas, &camera, None))
        .unwrap();

    // A 16x16 sprite at the origin, drawn upright, covers exactly 256 pixels.
    // A sheared one covers the same area — the shear has determinant 1 — so
    // area proves nothing and the bounding box is what tells them apart.
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for y in 0..64 {
        for x in 0..64 {
            if at(&pixels, 64, x, y)[..3] == [0xC8, 0x28, 0x28] {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    let (x0, y0, x1, y1) = bounds.expect("the sprite should be drawn");
    assert_eq!(
        (x1 - x0 + 1, y1 - y0 + 1),
        (16, 16),
        "an upright 16x16 quad, not a 32x16 parallelogram"
    );
}
