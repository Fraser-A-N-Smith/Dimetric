//! A scene decides the light it is drawn under, and a dim scene dims.
//!
//! `Light2D` has been drawn since M6 and the pass that draws it only ran when
//! `RenderSettings::ambient` was not white — which nothing a game ships could
//! set. `dim frame capture --ambient` could, `project.toml` could not, a scene
//! could not, and a script could not, so in `dim-play` the ambient was always
//! white and every light a game placed added nothing at all.
//!
//! The second half was quieter and worse. The composite only applied the
//! ambient when the frame *also* carried at least one light, so a region at
//! dusk with no torches in it was drawn as if it were noon: the pass that
//! applies the ambient was skipped along with the pass that accumulates lights,
//! and the two are not the same question.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, PresentFilter, RenderSettings, Renderer,
};
use dimetric_scene::{Color, Node, Scene, Value};

/// The frame's size, and the sprite's.
const SIDE: u32 = 16;
/// A mid grey, so a multiply by anything is visible in both directions.
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

fn settings(ambient: Color) -> RenderSettings {
    RenderSettings {
        internal_resolution: (SIDE, SIDE),
        integer_upscale: true,
        pixel_snap: true,
        ambient,
        present_filter: PresentFilter::Nearest,
    }
}

/// One sprite covering the whole frame, and optionally a light over it.
fn page(light: bool) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .expect("root");
    let mut sprite = Node::new(uid("n_page0000"), "Sprite2D", "Page");
    sprite.props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/page".into())),
    );
    scene.insert(sprite, Some(root)).expect("sprite");
    if light {
        let mut lamp = Node::new(uid("n_lamp0000"), "Light2D", "Lamp");
        lamp.props.insert(
            "radius".into(),
            Value::Scalar(dimetric_core::Fx::from_int(64)),
        );
        lamp.props.insert(
            "energy".into(),
            Value::Scalar(dimetric_core::Fx::from_int(1)),
        );
        lamp.props
            .insert("color".into(), Value::Color(Color::WHITE));
        scene.insert(lamp, Some(root)).expect("lamp");
    }
    scene.update_world_transforms();
    scene
}

/// Mean brightness of the finished picture, 0 to 255, or `None` with no adapter.
///
/// `project` is the project-wide ambient and `scene` is what the scene's camera
/// declares, so the two together are the precedence question.
fn brightness(project: Color, scene: Option<Color>, light: bool) -> Option<f64> {
    let atlas = atlas();
    let instance = headless_instance();
    let mut renderer = match Renderer::new(&instance, None, &atlas, settings(project)) {
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
    camera.ambient = scene;
    let frame = extract(&page(light), &atlas, &camera, None);
    let pixels = capture.render(&mut renderer, &frame).expect("a frame");
    let total: u64 = pixels
        .chunks_exact(4)
        .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
        .sum();
    Some(total as f64 / (pixels.len() / 4 * 3) as f64)
}

#[test]
fn a_scene_that_says_nothing_is_unlit() {
    let Some(plain) = brightness(Color::WHITE, None, false) else {
        return;
    };
    assert!(
        plain > 100.0,
        "a mid grey page under no light at all: {plain}"
    );
}

#[test]
fn a_dim_scene_dims_even_with_no_light_in_it() {
    // The quiet half of the defect. A region at dusk with no torches was
    // composited as if it were noon.
    let Some(noon) = brightness(Color::WHITE, None, false) else {
        return;
    };
    let dusk = brightness(
        Color::WHITE,
        Some(Color::rgba(0x30, 0x30, 0x40, 0xFF)),
        false,
    )
    .expect("the adapter was there a moment ago");
    assert!(
        dusk < noon * 0.6,
        "dusk {dusk} should be well under noon {noon}"
    );
}

#[test]
fn a_light_brightens_what_the_ambient_dimmed() {
    let Some(dark) = brightness(
        Color::WHITE,
        Some(Color::rgba(0x20, 0x20, 0x28, 0xFF)),
        false,
    ) else {
        return;
    };
    let lamp = brightness(
        Color::WHITE,
        Some(Color::rgba(0x20, 0x20, 0x28, 0xFF)),
        true,
    )
    .expect("the adapter was there a moment ago");
    assert!(
        lamp > dark,
        "the lamp has to add something: {lamp} vs {dark}"
    );
}

#[test]
fn the_scenes_own_ambient_wins_over_the_projects() {
    // A region's light is a fact about the region: nine scenes, nine ambients,
    // and a scene swap brings the new one with it.
    let Some(bright_project) = brightness(Color::WHITE, None, false) else {
        return;
    };
    let dim_project =
        brightness(Color::rgba(0x30, 0x30, 0x40, 0xFF), None, false).expect("adapter");
    assert!(
        dim_project < bright_project,
        "the project's own ambient applies when a scene says nothing"
    );

    // The scene overrides it in both directions, which is the test that says
    // the camera is read rather than merged.
    let scene_lifts = brightness(
        Color::rgba(0x30, 0x30, 0x40, 0xFF),
        Some(Color::WHITE),
        false,
    )
    .expect("adapter");
    assert!(
        (scene_lifts - bright_project).abs() < 1.0,
        "a scene declaring white is unlit however dim the project is: \
         {scene_lifts} vs {bright_project}"
    );
}

#[test]
fn the_ambient_never_reaches_what_is_drawn() {
    // Presentation, like the zoom beside it. Extraction is what the simulation
    // can affect, and the ambient is not in it: two cameras differing only in
    // ambient must extract the same frame.
    let atlas = atlas();
    let scene = page(true);
    let mut lit = Camera::new((SIDE, SIDE));
    lit.ambient = Some(Color::rgba(0x10, 0x10, 0x18, 0xFF));
    let plain = extract(&scene, &atlas, &Camera::new((SIDE, SIDE)), None);
    let dimmed = extract(&scene, &atlas, &lit, None);
    let keys = |f: &dimetric_render::Frame| {
        f.sprites
            .iter()
            .map(|i| (i.key, i.pos, i.size, i.modulate))
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&plain), keys(&dimmed), "only the composite may differ");
    assert_eq!(plain.lights.len(), dimmed.lights.len());
}
