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
    // The solid white texel too: a panel is a sprite sampling one opaque
    // pixel, so an atlas without it draws no UI at all.
    Atlas::pack(
        vec![
            page,
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
        ],
        64,
    )
    .with_fonts(fonts)
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

/// One glyph five pixels wide, which is an odd number — like the engine's own
/// built-in font, and unlike the four-pixel block above.
///
/// The width being odd is the whole point. A quad is positioned by its centre
/// and the layout gives a corner, so the centre is the corner plus half the
/// width; half of five is not an integer, and an implementation that pretends
/// it is puts the quad's edges half a texel off the pixel grid.
fn odd_font() -> (dimetric_assets::Font, dimetric_assets::Image) {
    use dimetric_assets::font::Glyph;
    let mut glyphs = std::collections::BTreeMap::new();
    glyphs.insert(
        'A',
        Glyph {
            x: 0,
            y: 0,
            width: 5,
            height: 7,
            bearing_x: 0,
            bearing_y: -7,
            advance: 6,
        },
    );
    let font = dimetric_assets::Font {
        size: 7,
        line_height: 8,
        ascent: 7,
        descent: 0,
        glyphs,
    };
    let page = dimetric_assets::Image {
        name: "fonts/odd".to_string(),
        width: 10,
        height: 7,
        pixels: vec![255; 10 * 7 * 4],
    };
    (font, page)
}

#[test]
fn an_odd_width_glyph_still_lands_on_the_pixel_grid() {
    let (font, page) = odd_font();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert("fonts/odd".to_string(), font);
    let atlas = Atlas::pack(vec![page], 64).with_fonts(fonts);

    let source = r#"format = "dimetric"
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
font = "asset:fonts/odd"
text = "A"
pos = [0.0, 0.0]
"#;
    let out = dimetric_scene::parse(
        source,
        "t.dim",
        &dimetric_scene::KindRegistry::with_builtins(),
    );
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;

    let camera = Camera::new((64, 64));
    let frame = extract(&scene, &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 1);
    let quad = &frame.sprites[0];

    // The corner the layout asked for, recovered from the centre and the size.
    // Half a texel out here and every sample lands on a boundary between two
    // columns of the font page, so nearest filtering serves up pieces of the
    // neighbouring letter and a line of text comes out as a jumble.
    let left = quad.pos.x - quad.size.x / 2;
    let top = quad.pos.y - quad.size.y / 2;
    assert_eq!(
        left,
        dimetric_core::Fx::from_int(0),
        "a five-wide glyph left its quad off the pixel grid"
    );
    assert_eq!(
        top,
        dimetric_core::Fx::from_int(0),
        "and the same vertically"
    );
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

// -- UI -------------------------------------------------------------------

fn ui_scene(body: &str) -> dimetric_scene::Scene {
    let text = format!(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n{body}"
    );
    let out = dimetric_scene::parse(
        &text,
        "t.dim",
        &dimetric_scene::KindRegistry::with_builtins(),
    );
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

const CORNER_PANEL: &str = r##"
[[node]]
id = "n_panel000"
kind = "Panel"
name = "Panel"
parent = "n_root0000"
offset_right = 32.0
offset_bottom = 16.0
modulate = "#ff0000ff"
"##;

#[test]
fn a_panel_is_extracted_into_the_ui_layer_and_not_the_world() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(&ui_scene(CORNER_PANEL), &atlas, &camera, None);

    assert_eq!(frame.ui.len(), 1, "the panel should be a UI quad");
    assert!(
        frame.sprites.is_empty(),
        "and nothing should have gone into the world layer"
    );
    // Canvas pixels, so the rect is where the scene said and not where a
    // camera happens to be looking.
    assert_eq!(frame.ui[0].size, dimetric_core::Vec2Fx::from_ints(32, 16));
    assert_eq!(frame.ui[0].pos, dimetric_core::Vec2Fx::from_ints(16, 8));
}

#[test]
fn ui_does_not_move_when_the_camera_does() {
    // The property that makes it UI. A health bar that scrolled with the world
    // would be a sprite with extra steps.
    let atlas = label_atlas();
    let scene = ui_scene(CORNER_PANEL);

    let mut near = Camera::new((64, 64));
    near.center = dimetric_core::Vec2Fx::from_ints(0, 0);
    let mut far = Camera::new((64, 64));
    far.center = dimetric_core::Vec2Fx::from_ints(500, -300);

    let a = extract(&scene, &atlas, &near, None);
    let b = extract(&scene, &atlas, &far, None);
    assert_eq!(a.ui[0].pos, b.ui[0].pos);
}

#[test]
fn a_label_inside_a_control_draws_in_canvas_space_exactly_once() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r#"
[[node]]
id = "n_panel000"
kind = "Panel"
name = "Panel"
parent = "n_root0000"
offset_left = 20.0
offset_top = 10.0
offset_right = 90.0
offset_bottom = 40.0

[[node]]
id = "n_text0000"
kind = "Label"
name = "Text"
parent = "n_panel000"
font = "asset:fonts/block"
text = "AB"
"#,
        ),
        &atlas,
        &camera,
        None,
    );

    // One panel quad plus one quad per glyph, all in the UI layer.
    assert_eq!(frame.ui.len(), 3);
    assert!(
        frame.sprites.is_empty(),
        "a label in the UI tree must not also be drawn in the world"
    );
    // Laid out from the panel's top-left corner.
    let first_glyph = frame
        .ui
        .iter()
        .find(|i| i.size.x == dimetric_core::Fx::from_int(4))
        .unwrap();
    assert_eq!(first_glyph.pos.x, dimetric_core::Fx::from_int(22));
}

#[test]
fn a_label_outside_the_ui_tree_still_draws_in_the_world() {
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(&label_scene("AB", "Left"), &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 2);
    assert!(frame.ui.is_empty());
}

#[test]
fn ui_reaches_the_target_above_the_world_and_unlit() {
    let atlas = label_atlas();
    let Some(mut renderer) = renderer(&atlas, settings()) else {
        return;
    };
    let capture = Capture::new(&renderer, (64, 64));
    let camera = Camera::new((64, 64));

    let blank = capture
        .render(
            &mut renderer,
            &extract(&ui_scene(""), &atlas, &camera, None),
        )
        .expect("render");
    let panelled = capture
        .render(
            &mut renderer,
            &extract(&ui_scene(CORNER_PANEL), &atlas, &camera, None),
        )
        .expect("render");

    let changed = blank
        .chunks(4)
        .zip(panelled.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 0, "the panel drew nothing");

    // Opaque red, and red it must stay: the composite multiplies the world by
    // the light buffer, and a UI that went through that would dim in a dark
    // room. The panel is at the canvas top-left, so sample near the origin.
    let px = at(&panelled, 64, 2, 2);
    assert!(
        px[0] > 200 && px[1] < 60 && px[2] < 60,
        "expected opaque red, found {px:?}"
    );
}

#[test]
fn a_button_draws_the_colour_its_state_asks_for() {
    // The other end of the property the simulation writes: extraction reads it
    // and picks one of three colours. Nothing here knows a simulation exists,
    // which is the point of routing it through a node property.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let button = |state: i64| {
        format!(
            r##"
[[node]]
id = "n_btn00000"
kind = "Button"
name = "Go"
parent = "n_root0000"
offset_right = 40.0
offset_bottom = 20.0
modulate = "#112233ff"
modulate_hover = "#445566ff"
modulate_pressed = "#778899ff"
state = {state}
"##
        )
    };
    let colour_of = |state: i64| {
        let frame = extract(&ui_scene(&button(state)), &atlas, &camera, None);
        frame.ui[0].modulate
    };

    let idle = colour_of(0);
    let hover = colour_of(1);
    let pressed = colour_of(2);
    assert_ne!(idle, hover, "hover must not look like idle");
    assert_ne!(hover, pressed, "pressed must not look like hover");

    // And the actual values, so a mixed-up lookup shows as a wrong colour
    // rather than merely a different one.
    assert_eq!(idle[0], 0x11);
    assert_eq!(hover[0], 0x44);
    assert_eq!(pressed[0], 0x77);
}

#[test]
fn a_button_with_no_hover_colour_falls_back_to_its_fill() {
    // A scene that sets only `modulate` should get a button that works, not an
    // invisible one.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r##"
[[node]]
id = "n_btn00000"
kind = "Button"
name = "Go"
parent = "n_root0000"
offset_right = 40.0
offset_bottom = 20.0
modulate = "#112233ff"
state = 1
"##,
        ),
        &atlas,
        &camera,
        None,
    );
    // Lightened rather than the engine's own grey: a button that set one
    // colour should look like that colour, not like the default theme.
    let r = frame.ui[0].modulate[0];
    assert!(r > 0x11, "hover should lighten the fill, got {r:#04x}");
    assert!(r < 0x40, "and not replace it, got {r:#04x}");
}

// -- The built-in font ----------------------------------------------------

/// An atlas holding only what the engine itself provides: no project assets.
fn bare_atlas() -> Atlas {
    let (font, page) = dimetric_assets::builtin_font::builtin();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert(
        dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
        font,
    );
    Atlas::pack(
        vec![
            dimetric_render::atlas::Source {
                name: dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
                width: page.width,
                height: page.height,
                pixels: page.pixels,
            },
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
        ],
        512,
    )
    .with_fonts(fonts)
}

#[test]
fn a_label_with_no_font_draws_in_the_built_in_one() {
    // The point of having a built-in: a project with no font asset can still
    // put words on the screen.
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r#"
[[node]]
id = "n_text0000"
kind = "Label"
name = "Text"
parent = "n_root0000"
text = "OK"
"#,
        ),
        &bare_atlas(),
        &camera,
        None,
    );
    assert_eq!(frame.sprites.len(), 2, "two letters, two quads");
}

#[test]
fn a_label_naming_a_font_that_is_not_there_falls_back_rather_than_vanishing() {
    // Before there was a built-in the honest answer was to draw nothing. Now
    // that the engine always has a font, showing the words beats showing a
    // gap where a sentence should be.
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r#"
[[node]]
id = "n_text0000"
kind = "Label"
name = "Text"
parent = "n_root0000"
font = "asset:fonts/nothing-here"
text = "OK"
"#,
        ),
        &bare_atlas(),
        &camera,
        None,
    );
    assert_eq!(frame.sprites.len(), 2);
}

#[test]
fn a_project_font_still_wins_over_the_built_in() {
    // The fallback must not shadow a real asset.
    let (font, page) = dimetric_assets::builtin_font::builtin();
    let (block, block_page) = block_font();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert(
        dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
        font,
    );
    fonts.insert("fonts/block".to_string(), block);
    let atlas = Atlas::pack(
        vec![
            dimetric_render::atlas::Source {
                name: dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
                width: page.width,
                height: page.height,
                pixels: page.pixels,
            },
            block_page,
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
        ],
        512,
    )
    .with_fonts(fonts);

    let camera = Camera::new((64, 64));
    let frame = extract(&label_scene("AB", "Left"), &atlas, &camera, None);
    // The block font's glyphs are 4 wide; the built-in's are 5.
    assert_eq!(frame.sprites[0].size.x, dimetric_core::Fx::from_int(4));
}

#[test]
fn ui_draws_in_tree_order_so_the_eye_and_the_hit_test_agree() {
    // A regression, and the kind worth writing down. Every unit test passed
    // while the actual menu came out with two of its three buttons missing:
    // the items sorted by node uid, which put a full-canvas backdrop in the
    // middle of the list and painted over everything declared before it.
    //
    // Tree order is the only thing that may decide what draws on top, because
    // the hit test already says later siblings win. A backdrop that covered
    // the buttons it was declared *before* would be invisible to the eye and
    // still clickable, which is the worst of both.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r##"
[[node]]
id = "n_back0000"
kind = "Panel"
name = "Backdrop"
parent = "n_root0000"
anchor_right = 1.0
anchor_bottom = 1.0
modulate = "#101018ff"

[[node]]
id = "n_zzz00000"
kind = "Panel"
name = "First"
parent = "n_root0000"
offset_bottom = 16.0
offset_right = 40.0
modulate = "#ff0000ff"

[[node]]
id = "n_aaa00000"
kind = "Panel"
name = "Second"
parent = "n_root0000"
offset_top = 20.0
offset_bottom = 36.0
offset_right = 40.0
modulate = "#00ff00ff"
"##,
        ),
        &atlas,
        &camera,
        None,
    );

    // Declaration order, whatever the uids sort to. The ids above are chosen
    // so that uid order and tree order disagree.
    let names: Vec<[u8; 4]> = frame.ui.iter().map(|i| i.modulate).collect();
    assert_eq!(
        names,
        vec![
            [0x10, 0x10, 0x18, 0xff],
            [0xff, 0x00, 0x00, 0xff],
            [0x00, 0xff, 0x00, 0xff],
        ],
        "the backdrop must draw first, then each panel in the order declared"
    );
}

#[test]
fn a_caption_draws_on_top_of_the_button_it_sits_on() {
    // Falls out of the depth-first walk, but it is the whole reason a button
    // with a label child is usable, so it is asserted rather than assumed.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let frame = extract(
        &ui_scene(
            r#"
[[node]]
id = "n_btn00000"
kind = "Button"
name = "Go"
parent = "n_root0000"
offset_right = 60.0
offset_bottom = 20.0

[[node]]
id = "n_cap00000"
kind = "Label"
name = "Caption"
parent = "n_btn00000"
font = "asset:fonts/block"
text = "AB"
"#,
        ),
        &atlas,
        &camera,
        None,
    );
    assert_eq!(frame.ui.len(), 3, "a button and two glyphs");
    // The button is the wide one; the glyphs follow it.
    assert!(frame.ui[0].size.x > frame.ui[1].size.x);
}

#[test]
fn z_decides_the_order_of_two_sprites_at_one_position() {
    // The end-to-end proof that `z` is wired up, through real scene parsing
    // and real extraction. Two sprites at exactly the same world position, so
    // depth cannot decide and the uid tie-break would settle it; only `z` can
    // put the right one last, and last is what draws on top.
    //
    // This is the case the grid game has in every cell holding a zone effect,
    // a pickup and an actor, and until now it was arbitrary.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));

    let order = |low_z: i32, high_z: i32| -> Vec<String> {
        let body = format!(
            r##"
[[node]]
id = "n_zzzzzzzz"
kind = "Sprite2D"
name = "First"
parent = "n_root0000"
texture = "asset:fonts/block"
pos = [0.0, 0.0]
z = {low_z}

[[node]]
id = "n_aaaaaaaa"
kind = "Sprite2D"
name = "Second"
parent = "n_root0000"
texture = "asset:fonts/block"
pos = [0.0, 0.0]
z = {high_z}
"##
        );
        let scene = ui_scene(&body);
        let frame = extract(&scene, &atlas, &camera, None);
        frame
            .sprites
            .iter()
            .map(|item| {
                let id = scene
                    .walk()
                    .into_iter()
                    .find(|id| scene.get(*id).is_some_and(|n| n.uid == item.node))
                    .expect("node");
                scene.get(id).expect("node").name.clone()
            })
            .collect()
    };

    // The higher `z` is extracted last, so it draws on top.
    assert_eq!(order(1, 9), vec!["First", "Second"]);

    // Swap the numbers and the order swaps. The node ids are chosen so that
    // the uid tie-break would give the same answer both times if `z` were
    // still being ignored — a pass here cannot be the tie-break in disguise.
    assert_eq!(order(9, 1), vec!["Second", "First"]);
}

#[test]
fn sprites_with_different_z_still_batch_together() {
    // Worth knowing before 55 sheets depend on it. `z` sits above the batch
    // group in the sort key, so it *could* have interleaved runs — but the
    // batcher merges adjacent items that agree on atlas, blend and shader, and
    // sorting by z keeps them adjacent. Three sprites, three different z, one
    // draw call.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let mut body = String::new();
    for (i, z) in [0, 5, 20].iter().enumerate() {
        body.push_str(&format!(
            r##"
[[node]]
id = "n_s{i:07}"
kind = "Sprite2D"
name = "S{i}"
parent = "n_root0000"
texture = "asset:__solid"
pos = [{}.0, 0.0]
scale = [8.0, 8.0]
z = {z}
"##,
            i * 10
        ));
    }
    let frame = extract(&ui_scene(&body), &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 3);
    assert_eq!(
        frame.batches.len(),
        1,
        "different z must not split a batch: {:?}",
        frame.batches
    );
}

// -- Mirroring an animated sprite ------------------------------------------

#[test]
fn an_animated_sprite_can_be_mirrored() {
    // `Sprite2D` had `flip_h` and `flip_v` and `AnimatedSprite2D` had neither,
    // so a directional character needed four drawn facings where two would do.
    // Under the 2:1 shear a grid actor's four screen facings are two mirrored
    // pairs, which across a project's actor sheets is the largest single lever
    // on the art budget.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));

    let uv_of = |flip: &str| {
        let body = format!(
            r#"
[[node]]
id = "n_hero0000"
kind = "AnimatedSprite2D"
name = "Hero"
parent = "n_root0000"
frames = "asset:fonts/block"
animation = "idle"
{flip}
"#
        );
        let frame = extract(&ui_scene(&body), &atlas, &camera, None);
        assert_eq!(frame.sprites.len(), 1, "the sprite should extract");
        frame.sprites[0].uv
    };

    let plain = uv_of("");
    let flipped_h = uv_of("flip_h = true");
    let flipped_v = uv_of("flip_v = true");

    // A horizontal mirror swaps the u pair and leaves v alone.
    assert_eq!(flipped_h[0], plain[2]);
    assert_eq!(flipped_h[2], plain[0]);
    assert_eq!(flipped_h[1], plain[1]);
    assert_eq!(flipped_h[3], plain[3]);

    // And vertical the other way round.
    assert_eq!(flipped_v[1], plain[3]);
    assert_eq!(flipped_v[3], plain[1]);
    assert_eq!(flipped_v[0], plain[0]);
    assert_eq!(flipped_v[2], plain[2]);
}

#[test]
fn a_flipped_sprite_batches_with_an_unflipped_one() {
    // Asked for explicitly, and worth knowing before 55 sheets depend on it.
    // The flip swaps UVs on the draw item and never reaches `batch_group`,
    // which is atlas, shader and blend — so a row of actors facing both ways
    // is still one draw call.
    let atlas = label_atlas();
    let camera = Camera::new((64, 64));
    let mut body = String::new();
    for (i, flip) in ["", "flip_h = true", "", "flip_h = true"]
        .iter()
        .enumerate()
    {
        body.push_str(&format!(
            r#"
[[node]]
id = "n_h{i:07}"
kind = "AnimatedSprite2D"
name = "H{i}"
parent = "n_root0000"
frames = "asset:fonts/block"
animation = "idle"
pos = [{}.0, 0.0]
{flip}
"#,
            i * 12
        ));
    }
    let frame = extract(&ui_scene(&body), &atlas, &camera, None);
    assert_eq!(frame.sprites.len(), 4);
    assert_eq!(
        frame.batches.len(),
        1,
        "a flip must not split a batch: {:?}",
        frame.batches
    );
}
