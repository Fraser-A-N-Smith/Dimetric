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

/// A three-frame strip: red, green, blue, side by side, 8 pixels each.
fn strip_atlas() -> Atlas {
    let mut pixels = Vec::with_capacity(24 * 8 * 4);
    for _ in 0..8 {
        for x in 0..24u32 {
            pixels.extend_from_slice(match x / 8 {
                0 => &[0xFF, 0x00, 0x00, 0xFF],
                1 => &[0x00, 0xFF, 0x00, 0xFF],
                _ => &[0x00, 0x00, 0xFF, 0xFF],
            });
        }
    }
    Atlas::pack_framed(
        vec![dimetric_assets::sheet::Framed {
            image: Source {
                name: "sprites/walk".into(),
                width: 24,
                height: 8,
                pixels,
            },
            frames: 3,
        }],
        256,
    )
}

/// An `AnimatedSprite2D` showing a given frame of the strip.
fn animated_scene(frame: i64) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(
            Node::new(NodeUid::parse("n_root0000").unwrap(), "Node2D", "Room"),
            None,
        )
        .expect("the root inserts");
    let mut node = Node::new(
        NodeUid::parse("n_walker01").unwrap(),
        "AnimatedSprite2D",
        "Walker",
    );
    node.set(
        "frames".to_string(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/walk".into())),
    );
    node.set("frame".to_string(), Value::Int(frame));
    scene.insert(node, Some(root)).expect("the sprite inserts");
    scene
}

#[test]
fn the_frame_index_picks_a_slice_of_the_strip() {
    // Without this the whole sheet draws as one sprite, and an animation that
    // advances correctly still looks like a contact sheet.
    let atlas = strip_atlas();
    let camera = Camera::new((64, 64));

    let first = extract(&animated_scene(0), &atlas, &camera, None);
    let second = extract(&animated_scene(1), &atlas, &camera, None);

    assert_eq!(first.sprites.len(), 1);
    assert_eq!(second.sprites.len(), 1);

    let uv_of = |frame: &dimetric_render::Frame| frame.sprites[0].uv;
    assert_ne!(
        uv_of(&first),
        uv_of(&second),
        "different frame, different uv"
    );

    // A third of the strip each, and the second starts where the first ends.
    let (a, b) = (uv_of(&first), uv_of(&second));
    let width = a[2] - a[0];
    assert!((b[0] - a[2]).abs() < 1e-5, "{a:?} then {b:?}");
    assert!((width * 3.0 - (atlas.frames("sprites/walk") as f32 * width)).abs() < 1e-5);
}

#[test]
fn a_sprite_slice_is_one_frame_wide_rather_than_the_whole_sheet() {
    let atlas = strip_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(&animated_scene(2), &atlas, &camera, None);
    // Eight pixels of a twenty-four pixel strip.
    assert_eq!(frame.sprites[0].size, Vec2Fx::from_ints(8, 8));
}

#[test]
fn a_frame_index_past_the_end_clamps_rather_than_sampling_nothing() {
    let atlas = strip_atlas();
    let camera = Camera::new((64, 64));
    let last = extract(&animated_scene(2), &atlas, &camera, None);
    let past = extract(&animated_scene(99), &atlas, &camera, None);
    assert_eq!(last.sprites[0].uv, past.sprites[0].uv);
}

#[test]
fn a_still_image_is_not_sliced() {
    let frame = extract(
        &{
            let mut scene = Scene::new();
            let root = scene
                .insert(
                    Node::new(NodeUid::parse("n_root0000").unwrap(), "Node2D", "Room"),
                    None,
                )
                .expect("the root inserts");
            let mut node = Node::new(NodeUid::parse("n_block001").unwrap(), "Sprite2D", "Block");
            node.set(
                "texture".to_string(),
                Value::Ref(dimetric_scene::Reference::Asset("sprites/block".into())),
            );
            scene.insert(node, Some(root)).expect("the sprite inserts");
            scene
        },
        &atlas(),
        &Camera::new((64, 64)),
        None,
    );
    assert_eq!(frame.sprites[0].size, Vec2Fx::from_ints(16, 16));
}

// -- Labels ---------------------------------------------------------------
//
// A synthetic font rather than a rasterised one. A real TTF would make these
// tests depend on which font happens to be installed, and the three CI
// platforms do not agree on that — but the thing worth testing here is the
// *path*, from baked metrics through layout to pixels on a target, and that
// path does not care where the bitmap came from.

/// Two glyphs, each a solid 4x4 block, side by side on an 8x4 page.
///
/// `A` is the left block and `B` the right, both advancing 5 — one pixel wider
/// than the ink, so a run of them has a visible gap and a placement bug cannot
/// hide behind touching blocks.
fn block_font() -> (dimetric_assets::Font, dimetric_assets::Image) {
    use dimetric_assets::font::Glyph;
    let block = |x: u32| Glyph {
        x,
        y: 0,
        width: 4,
        height: 4,
        bearing_x: 0,
        bearing_y: -4,
        advance: 5,
    };
    let mut glyphs = std::collections::BTreeMap::new();
    glyphs.insert('A', block(0));
    glyphs.insert('B', block(4));
    let font = dimetric_assets::Font {
        size: 4,
        line_height: 6,
        ascent: 4,
        descent: 0,
        glyphs,
    };
    let page = dimetric_assets::Image {
        name: "fonts/block".to_string(),
        width: 8,
        height: 4,
        pixels: vec![255; 8 * 4 * 4],
    };
    (font, page)
}

fn label_atlas() -> Atlas {
    let (font, page) = block_font();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert("fonts/block".to_string(), font);
    Atlas::pack(vec![page], 64).with_fonts(fonts)
}

fn label_scene(text: &str, align: &str) -> dimetric_scene::Scene {
    let source = format!(
        r#"format = "dimetric"
version = 1
[scene]
root = "n_root0000"
[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Root"
[[node]]
id = "n_label000"
kind = "Label"
name = "Text"
parent = "n_root0000"
font = "asset:fonts/block"
text = {text:?}
align = {align:?}
pos = [0.0, 0.0]
"#
    );
    let out = dimetric_scene::parse(
        &source,
        "t.dim",
        &dimetric_scene::KindRegistry::with_builtins(),
    );
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

#[test]
fn a_label_becomes_one_quad_per_inked_glyph_in_a_single_batch() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(&label_scene("AB", "Left"), &atlas, &camera, None);

    assert_eq!(frame.sprites.len(), 2, "one quad per glyph");
    assert_eq!(
        frame.draw_calls(),
        1,
        "a line of text shares a depth, so it batches as one draw"
    );

    // Five pixels apart: the advance, not the four-pixel bitmap width.
    let dx = frame.sprites[1].pos.x - frame.sprites[0].pos.x;
    assert_eq!(dx, dimetric_core::Fx::from_int(5));
}

#[test]
fn each_glyph_samples_its_own_part_of_the_page() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(&label_scene("AB", "Left"), &atlas, &camera, None);

    // Two glyphs, two different sub-rectangles. Identical UVs would mean every
    // letter drew the same glyph, which on a real font is the kind of bug that
    // looks like a rendering artefact rather than a lookup error.
    assert_ne!(frame.sprites[0].uv, frame.sprites[1].uv);
    assert!(frame.sprites[0].uv[2] <= frame.sprites[1].uv[0] + f32::EPSILON);
}

#[test]
fn a_label_with_no_text_or_no_font_draws_nothing() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    assert!(extract(&label_scene("", "Left"), &atlas, &camera, None)
        .sprites
        .is_empty());

    // A font the atlas has never heard of draws nothing rather than a
    // page-sized magenta placeholder.
    let missing = label_scene("AB", "Left");
    let empty = Atlas::pack(
        vec![dimetric_assets::Image {
            name: "other".into(),
            width: 1,
            height: 1,
            pixels: vec![255; 4],
        }],
        64,
    );
    assert!(extract(&missing, &empty, &camera, None).sprites.is_empty());
}

#[test]
fn text_reaches_the_target() {
    // The end of the path: metrics, layout, atlas lookup, quad, pixels.
    let atlas = label_atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));
    let mut camera = Camera::new((64, 64));
    camera.zoom = 1.0;

    let blank = capture
        .render(
            &mut renderer,
            &extract(&label_scene("", "Left"), &atlas, &camera, None),
        )
        .expect("render");
    let drawn = capture
        .render(
            &mut renderer,
            &extract(&label_scene("AB", "Left"), &atlas, &camera, None),
        )
        .expect("render");

    // Against the blank frame rather than against alpha: the composite writes
    // an opaque background, so every pixel is "inked" whether or not anything
    // was drawn on it. What text does is *change* pixels.
    let changed = blank
        .chunks(4)
        .zip(drawn.chunks(4))
        .filter(|(before, after)| before != after)
        .count();
    assert!(
        changed >= 32,
        "two 4x4 glyphs should change about 32 pixels, found {changed}"
    );
}
