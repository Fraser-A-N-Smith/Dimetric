//! A light can be a pool on the floor instead of a circle on the screen.
//!
//! Every light was drawn as an axis-aligned screen-space quad, under a comment
//! saying why: "so a radial light stays a circle on screen instead of being
//! sheared into an ellipse". That is right for a light *in the air* — a glowing
//! projectile is a ball, and a ball looks like a circle from any angle — and
//! wrong for a torch on the floor. Under the 2:1 shear a screen circle reaches
//! twice as many tiles north as east, so a torch lights a diamond-shaped room
//! in a shape that has nothing to do with the room.
//!
//! `ground = true` measures the radius in world units and projects it with the
//! floor. The assertions below measure the lit region rather than describing
//! it: a ground light under `Isometric` has to come out twice as wide as it is
//! tall, because that is what the floor does to a circle, and a screen light
//! has to stay square whatever the projection.
//!
//! These need a graphics adapter — the whole point is what the shader does —
//! and skip without one unless `DIMETRIC_REQUIRE_GPU` is set.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, PresentFilter, Projection, RenderSettings,
    Renderer,
};
use dimetric_scene::{Color, Node, Scene, Value};

/// Big enough that an extent is measurable and small enough to stay quick.
const SIDE: u32 = 96;
/// Well inside the frame, so neither axis of the ellipse is clipped by it.
const RADIUS: i32 = 20;
/// An opaque mid grey, so the light has something to brighten.
const PAINT: [u8; 4] = [0x60, 0x60, 0x60, 0xFF];
/// Dark, so the lit region is unmistakable against the rest.
const NIGHT: Color = Color {
    r: 0x10,
    g: 0x10,
    b: 0x10,
    a: 0xFF,
};

fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "sprites/floor".into(),
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
        // Off: a snapped camera would move the lit region by up to a pixel
        // between the two projections and the comparison is about its shape.
        pixel_snap: false,
        ambient: NIGHT,
        present_filter: PresentFilter::Nearest,
    }
}

/// A floor filling the frame with one lamp at the origin.
fn room(ground: Option<bool>) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .expect("root");
    let mut floor = Node::new(uid("n_flor0000"), "Sprite2D", "Floor");
    floor.props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/floor".into())),
    );
    // Not lit by the light buffer would defeat the measurement entirely.
    scene.insert(floor, Some(root)).expect("floor");

    let mut lamp = Node::new(uid("n_lamp0000"), "Light2D", "Lamp");
    lamp.props
        .insert("radius".into(), Value::Scalar(Fx::from_int(RADIUS)));
    lamp.props
        .insert("energy".into(), Value::Scalar(Fx::from_int(2)));
    lamp.props
        .insert("color".into(), Value::Color(Color::WHITE));
    if let Some(on) = ground {
        lamp.props.insert("ground".into(), Value::Bool(on));
    }
    scene.insert(lamp, Some(root)).expect("lamp");
    scene.update_world_transforms();
    scene
}

/// The lit region's width and height in pixels, or `None` with no adapter.
///
/// "Lit" is any pixel brighter than the floor under the ambient alone, which
/// the night makes a wide margin: the floor is 0x60 times 0x10/0xFF and the lit
/// part is several times that.
fn lit_extent(ground: Option<bool>, projection: Projection) -> Option<(u32, u32)> {
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
    camera.projection = projection;
    let frame = extract(&room(ground), &atlas, &camera, None);
    let pixels = capture.render(&mut renderer, &frame).expect("a frame");

    // The unlit floor's own value, measured from a corner rather than computed,
    // so the threshold does not depend on how the composite rounds.
    let corner = pixels[0] as u32 + pixels[1] as u32 + pixels[2] as u32;
    let threshold = corner + 24;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (SIDE, 0u32, SIDE, 0u32);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let at = ((y * SIDE + x) * 4) as usize;
            let sum = pixels[at] as u32 + pixels[at + 1] as u32 + pixels[at + 2] as u32;
            if sum > threshold {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    assert!(max_x >= min_x && max_y >= min_y, "nothing was lit at all");
    Some((max_x - min_x + 1, max_y - min_y + 1))
}

#[test]
fn a_screen_light_stays_square_under_both_projections() {
    // The control, and the behaviour every scene written before this had. The
    // comment in `light.wgsl` claims the quad is axis-aligned so a radial light
    // stays a circle; this is that claim measured.
    for ground in [None, Some(false)] {
        let Some(top) = lit_extent(ground, Projection::TopDown) else {
            return;
        };
        let iso = lit_extent(ground, Projection::Isometric).expect("an adapter");
        for (name, (w, h)) in [("top-down", top), ("isometric", iso)] {
            let ratio = w as f64 / h as f64;
            assert!(
                (ratio - 1.0).abs() < 0.1,
                "{ground:?} {name}: {w}x{h} is not round"
            );
        }
        assert_eq!(top, iso, "{ground:?}: the projection does not reach it");
    }
}

#[test]
fn a_ground_light_is_a_two_to_one_ellipse_under_isometric() {
    // What the floor does to a circle. The 2:1 shear maps a world circle of
    // radius r to an ellipse 2*r*sqrt(2) wide and r*sqrt(2) tall, so the lit
    // region has to come out twice as wide as it is tall -- which is a pool of
    // light on a floor, and is what a torch in a diamond-shaped room looks
    // like.
    let Some((w, h)) = lit_extent(Some(true), Projection::Isometric) else {
        return;
    };
    let ratio = w as f64 / h as f64;
    assert!(
        (ratio - 2.0).abs() < 0.15,
        "{w}x{h} is a ratio of {ratio:.2}, not 2:1"
    );
}

#[test]
fn a_ground_light_under_top_down_is_the_same_circle_as_before() {
    // Top-down is the identity projection, so there is nothing for `ground` to
    // change. A difference here would mean the flag was doing something of its
    // own rather than deferring to the projection.
    let Some(plain) = lit_extent(Some(false), Projection::TopDown) else {
        return;
    };
    let grounded = lit_extent(Some(true), Projection::TopDown).expect("an adapter");
    assert_eq!(plain, grounded);
}

#[test]
fn a_ground_light_reaches_the_same_distance_in_world_units_every_way() {
    // The reason the shape matters. A screen circle of radius r reaches r
    // world units east and 2r north, because the floor is half as tall as it
    // is wide; a ground light reaches r in both.
    //
    // Measured as a *fraction* of the ideal extent rather than against it
    // directly. The falloff is `(1 - d)^2` and reaches zero at the radius, so
    // the outermost ring is too dim to clear any threshold and the measured
    // ellipse is always a little inside the mathematical one -- by the same
    // proportion on both axes, which is exactly the claim. Comparing the two
    // fractions says "the falloff is measured in world units equally in every
    // direction" without the answer depending on where the threshold sits.
    let Some((w, h)) = lit_extent(Some(true), Projection::Isometric) else {
        return;
    };
    // The projected extents of a world circle of radius r under
    // [1, -1, 0.5, 0.5]: 2*r*sqrt(2) across and r*sqrt(2) down.
    let across = 2.0 * RADIUS as f64 * std::f64::consts::SQRT_2;
    let down = across / 2.0;
    let fraction_x = w as f64 / across;
    let fraction_y = h as f64 / down;
    assert!(
        (fraction_x - fraction_y).abs() < 0.05,
        "the light reaches {:.3} of its radius across and {:.3} down ({w}x{h})",
        fraction_x,
        fraction_y
    );
    // And it really is most of the way out, so the fractions agreeing is not
    // two small numbers agreeing about nothing.
    assert!(
        fraction_x > 0.8 && fraction_x <= 1.0,
        "the lit region should fill most of the radius, not {fraction_x:.3}"
    );
}

#[test]
fn the_flag_reaches_the_light_item_and_defaults_to_off() {
    // The extraction half, which needs no adapter: whatever the shader does
    // with it, the flag has to arrive. Absent reads as false, which is what
    // every scene written before this means.
    let camera = Camera::new((SIDE, SIDE));
    let atlas = atlas();
    let ground_of = |ground: Option<bool>| {
        let frame = extract(&room(ground), &atlas, &camera, None);
        assert_eq!(frame.lights.len(), 1, "one lamp");
        frame.lights[0].ground
    };
    assert!(!ground_of(None), "absent is a screen light");
    assert!(!ground_of(Some(false)));
    assert!(ground_of(Some(true)));
}
