//! A world sprite can keep its colour in a dark room.
//!
//! The composite multiplies the whole world texture by the light buffer, which
//! is right for the floor, the walls and the figures and wrong for everything a
//! player *reads*: a health bar over a monster's head, a damage number, the
//! cursor, the cells an ability can reach, a spell effect that ought to glow
//! rather than dim. Those are world drawing — they sit on the board and move
//! with the camera — so they cannot be UI controls without the game
//! re-projecting every one of them into screen space each frame, and the UI
//! pass is the only thing the light already skips.
//!
//! The implementation is a mask: a second pass over the same instances in the
//! same sort order, writing unlit coverage into the light buffer's alpha, and a
//! composite that mixes per pixel between "the light decides" and "leave it
//! alone". The order is what makes it correct rather than merely cheap, and
//! `a_lit_sprite_in_front_still_dims` is the test that holds it to that.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, PresentFilter, RenderSettings, Renderer,
};
use dimetric_scene::{Color, Node, Scene, Value};

const SIDE: u32 = 16;
/// Dark enough that the difference between lit and unlit is unmistakable.
const TOMB: Color = Color {
    r: 0x20,
    g: 0x20,
    b: 0x28,
    a: 0xFF,
};
/// An opaque mid grey, so a multiply shows in both directions.
const PAINT: [u8; 4] = [0x80, 0x80, 0x80, 0xFF];

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/page".into(),
                width: SIDE,
                height: SIDE,
                pixels: PAINT.repeat((SIDE * SIDE) as usize),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        256,
    )
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).expect("a node id")
}

fn settings() -> RenderSettings {
    RenderSettings {
        internal_resolution: (SIDE, SIDE),
        integer_upscale: true,
        pixel_snap: true,
        ambient: Color::WHITE,
        present_filter: PresentFilter::Nearest,
    }
}

/// A full-frame sprite, optionally marked unlit, optionally with a second
/// sprite of the same size drawn over it on a higher layer.
fn page(lit: bool, over: Option<bool>) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .expect("root");
    let add = |id: &str, name: &str, layer: i32, lit: bool| {
        let mut sprite = Node::new(uid(id), "Sprite2D", name);
        sprite.props.insert(
            "texture".into(),
            Value::Ref(dimetric_scene::Reference::Asset("sprites/page".into())),
        );
        sprite.layer = layer;
        if !lit {
            sprite.props.insert("lit".into(), Value::Bool(false));
        }
        sprite
    };
    let under = add("n_page0000", "Page", 0, lit);
    scene.insert(under, Some(root)).expect("sprite");
    if let Some(over_lit) = over {
        let on_top = add("n_page0001", "Over", 1, over_lit);
        scene.insert(on_top, Some(root)).expect("sprite");
    }
    scene.update_world_transforms();
    scene
}

/// Mean brightness of the finished picture, or `None` with no adapter.
fn brightness(scene: &Scene, ambient: Option<Color>) -> Option<f64> {
    let atlas = atlas();
    let instance = headless_instance();
    let mut renderer = match Renderer::new(&instance, None, &atlas, settings()) {
        Ok(renderer) => renderer,
        Err(e) => {
            assert!(
                std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                "DIMETRIC_REQUIRE_GPU is set but no adapter was available: {e}"
            );
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let capture = Capture::new(&renderer, (SIDE, SIDE));
    let mut camera = Camera::new((SIDE, SIDE));
    camera.center = Vec2Fx::ZERO;
    camera.ambient = ambient;
    let frame = extract(scene, &atlas, &camera, None);
    let pixels = capture.render(&mut renderer, &frame).expect("a frame");
    let total: u64 = pixels
        .chunks_exact(4)
        .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
        .sum();
    Some(total as f64 / (pixels.len() / 4 * 3) as f64)
}

#[test]
fn an_unlit_sprite_keeps_its_colour_in_a_dark_room() {
    let Some(noon) = brightness(&page(true, None), None) else {
        return;
    };
    let dimmed = brightness(&page(true, None), Some(TOMB)).expect("adapter");
    let kept = brightness(&page(false, None), Some(TOMB)).expect("adapter");
    assert!(dimmed < noon * 0.5, "a lit sprite dims: {dimmed} of {noon}");
    assert!(
        (kept - noon).abs() < 1.0,
        "an unlit one does not: {kept} against {noon} unlit"
    );
}

#[test]
fn lit_is_the_default_so_nothing_already_written_changes() {
    let Some(absent) = brightness(&page(true, None), Some(TOMB)) else {
        return;
    };
    // Explicitly `lit = true` has to be the same as saying nothing.
    let mut scene = page(true, None);
    let id = scene.resolve_path("/Root/Page").expect("the sprite");
    scene
        .get_mut(id)
        .unwrap()
        .props
        .insert("lit".into(), Value::Bool(true));
    let explicit = brightness(&scene, Some(TOMB)).expect("adapter");
    assert!((absent - explicit).abs() < 1.0, "{absent} vs {explicit}");
}

#[test]
fn a_lit_sprite_in_front_still_dims() {
    // The property that makes the mask correct rather than merely cheap. An
    // unlit range marker on a low layer with a lit figure drawn over it: the
    // figure has to dim, because the mask is written in the same order as the
    // colour and the figure's zero coverage lands on top.
    //
    // Both sprites cover the whole frame, so if the exemption leaked the
    // picture would be at full brightness everywhere.
    let Some(noon) = brightness(&page(true, None), None) else {
        return;
    };
    let over_lit = brightness(&page(false, Some(true)), Some(TOMB)).expect("adapter");
    assert!(
        over_lit < noon * 0.5,
        "the lit sprite in front has to dim: {over_lit} of {noon}"
    );
    // And the other way: an unlit sprite drawn over a lit one is exempt.
    let over_unlit = brightness(&page(true, Some(false)), Some(TOMB)).expect("adapter");
    assert!(
        (over_unlit - noon).abs() < 1.0,
        "the unlit sprite in front keeps its colour: {over_unlit} of {noon}"
    );
}

#[test]
fn an_unlit_sprite_still_sorts_with_everything_else() {
    // Only the multiply skips it. If `lit` ever reached the sort key or the
    // batching, an overlay would jump a layer.
    let atlas = atlas();
    let lit = extract(&page(true, None), &atlas, &Camera::new((SIDE, SIDE)), None);
    let unlit = extract(&page(false, None), &atlas, &Camera::new((SIDE, SIDE)), None);
    assert_eq!(
        lit.sprites.iter().map(|i| i.key).collect::<Vec<_>>(),
        unlit.sprites.iter().map(|i| i.key).collect::<Vec<_>>(),
        "the sort key must not know about it"
    );
    assert_eq!(
        lit.batches.len(),
        unlit.batches.len(),
        "nor may the batching split on it"
    );
    assert!(lit.sprites.iter().all(|i| i.lit));
    assert!(unlit.sprites.iter().all(|i| !i.lit));
}

#[test]
fn an_unlit_sprite_in_a_bright_room_is_drawn_the_same_as_a_lit_one() {
    // The mask only matters where there is a light to skip. With nothing
    // dimmed, marking a sprite unlit has to be invisible — otherwise the flag
    // would be a tint.
    let Some(lit) = brightness(&page(true, None), None) else {
        return;
    };
    let unlit = brightness(&page(false, None), None).expect("adapter");
    assert!((lit - unlit).abs() < 1.0, "{lit} vs {unlit}");
}

#[test]
fn the_flag_never_reaches_what_is_drawn_where() {
    // Presentation. Two scenes differing only in `lit` extract to the same
    // geometry, the same tints and the same lights.
    let atlas = atlas();
    let camera = Camera::new((SIDE, SIDE));
    let lit = extract(&page(true, None), &atlas, &camera, None);
    let unlit = extract(&page(false, None), &atlas, &camera, None);
    let shape = |f: &dimetric_render::Frame| {
        f.sprites
            .iter()
            .map(|i| (i.pos, i.size, i.uv, i.modulate, i.blend))
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&lit), shape(&unlit));
}
