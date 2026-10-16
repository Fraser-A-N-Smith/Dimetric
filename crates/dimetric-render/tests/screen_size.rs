//! Text on the board can be drawn at the size the font was made to be read at.
//!
//! A world `Label` is drawn at the board's zoom. That is right for something
//! that belongs to the scene — a sign painted on a wall — and wrong for
//! everything a player *reads*: a damage number at zoom 3 is three times the
//! size of the same number in the HUD, and at zoom 0.5 it is illegible. There
//! was no `scale` and no way to ask for the font's own size.
//!
//! `Label.in_world` was declined once, for putting text on the ground plane;
//! this is the opposite request and a much smaller one. The origin still
//! projects like any world position — so the text stays with the thing it
//! labels — and only the glyphs stop growing.
//!
//! Implemented by dividing the quad and the advance by the zoom in extraction,
//! because the shader multiplies both by a `pixel_scale` that carries the zoom:
//! the two cancel. That keeps the whole decision on the CPU where it can be
//! measured without a graphics adapter, which is what the assertions below do.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::{Node, Scene, Value};

fn atlas() -> Atlas {
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

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).expect("a node id")
}

/// One world label at the origin, with `screen_size` set or not.
fn scene(screen_size: Option<bool>) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "World"), None)
        .expect("root");
    let mut label = Node::new(uid("n_text0000"), "Label", "Damage");
    label
        .props
        .insert("text".into(), Value::Str("160".to_string()));
    if let Some(on) = screen_size {
        label.props.insert("screen_size".into(), Value::Bool(on));
    }
    scene.insert(label, Some(root)).expect("label");
    scene
}

fn camera_at(zoom: f32) -> Camera {
    let mut camera = Camera::new((320, 180));
    camera.zoom = zoom;
    camera
}

/// Every glyph's quad size and screen offset, in world units as the shader
/// receives them.
fn glyphs(scene: &Scene, zoom: f32) -> Vec<(Vec2Fx, Vec2Fx, Vec2Fx)> {
    let atlas = atlas();
    let camera = camera_at(zoom);
    extract::extract(scene, &atlas, &camera, None)
        .sprites
        .iter()
        .map(|i| (i.pos, i.screen_offset, i.size))
        .collect()
}

/// What the shader actually puts on screen for a quad: its extent times the
/// pixel scale, which carries the zoom. In clip-space units, so a value that
/// does not move with the zoom is a glyph that does not grow.
fn on_screen(extent: Fx, zoom: f32) -> f32 {
    extent.to_f64() as f32 * 2.0 * zoom / 320.0
}

#[test]
fn without_the_flag_a_world_label_still_scales_with_the_zoom() {
    // The control, and what every scene written before this did.
    for screen_size in [None, Some(false)] {
        let scene = scene(screen_size);
        let one = glyphs(&scene, 1.0);
        let three = glyphs(&scene, 3.0);
        assert_eq!(one.len(), 3, "three inked digits");
        assert_eq!(
            one, three,
            "the quads are the same world size at any zoom, so the zoom magnifies them"
        );
        // Which means what lands on screen is three times as big.
        let a = on_screen(one[0].2.x, 1.0);
        let b = on_screen(three[0].2.x, 3.0);
        assert!(
            (b - a * 3.0).abs() < 1e-6,
            "{screen_size:?}: {b} should be three times {a}"
        );
    }
}

#[test]
fn with_the_flag_the_glyphs_are_the_same_size_at_any_zoom() {
    let flagged = scene(Some(true));
    let sizes: Vec<f32> = [0.5, 1.0, 2.0, 3.0]
        .iter()
        .map(|zoom| {
            let g = glyphs(&flagged, *zoom);
            assert_eq!(g.len(), 3, "three inked digits at zoom {zoom}");
            on_screen(g[0].2.x, *zoom)
        })
        .collect();
    let first = sizes[0];
    for (zoom, got) in [0.5, 1.0, 2.0, 3.0].iter().zip(&sizes) {
        assert!(
            (got - first).abs() < 1e-5,
            "zoom {zoom} drew the glyph at {got}, not {first}"
        );
    }

    // And it is the font's own size, not some other constant: at zoom 1 the
    // unflagged label is already at the font's pixel size, so the two agree.
    let plain = glyphs(&scene(None), 1.0);
    assert!(
        (on_screen(plain[0].2.x, 1.0) - first).abs() < 1e-5,
        "the flagged size should be the font's own"
    );
}

#[test]
fn the_advance_between_glyphs_is_unmagnified_too() {
    // Otherwise "160" is three glyphs of the right size spread across a line
    // three times too wide, which is worse than three big digits.
    let scene = scene(Some(true));
    let one = glyphs(&scene, 1.0);
    let three = glyphs(&scene, 3.0);
    for (index, (a, b)) in one.iter().zip(&three).enumerate() {
        let on_a = on_screen(a.1.x, 1.0);
        let on_b = on_screen(b.1.x, 3.0);
        assert!(
            (on_a - on_b).abs() < 1e-5,
            "glyph {index}: advance {on_a} became {on_b}"
        );
    }
}

#[test]
fn the_origin_still_projects_like_any_world_position() {
    // The whole point of it being a world label. If the origin stopped
    // projecting, the text would no longer sit on the thing it names.
    let flagged = glyphs(&scene(Some(true)), 3.0);
    let plain = glyphs(&scene(None), 3.0);
    for (a, b) in flagged.iter().zip(&plain) {
        assert_eq!(
            a.0, b.0,
            "the position the shader projects is untouched by the flag"
        );
    }
}

#[test]
fn a_label_in_the_ui_tree_is_unaffected() {
    // The canvas projection has no zoom in it, so a UI label is already at the
    // font's own size and the flag has nothing to do. Asserted rather than
    // assumed, because the same `label` function serves both passes.
    // Parsed rather than hand-built: `ui::layout` lays out nodes whose *base*
    // kind is `Control`, and only the parser resolves `based_on` against the
    // kind registry. A hand-built `Panel` is not a control as far as layout is
    // concerned, and its caption would never reach the UI pass at all.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Control\"\nname = \"Root\"\n\
         anchor_right = 1.0\nanchor_bottom = 1.0\n\
         \n[[node]]\nid = \"n_pane0000\"\nkind = \"Panel\"\nname = \"Pane\"\n\
         parent = \"n_root0000\"\n\
         offset_left = 0.0\noffset_top = 0.0\noffset_right = 100.0\noffset_bottom = 40.0\n\
         \n[[node]]\nid = \"n_text0000\"\nkind = \"Label\"\nname = \"Caption\"\n\
         parent = \"n_pane0000\"\ntext = \"160\"\nscreen_size = true\n";
    let out = dimetric_scene::parse(
        text,
        "u.dim",
        &dimetric_scene::KindRegistry::with_builtins(),
    );
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let scene = out.doc.unwrap().scene;

    let atlas = atlas();
    let frame_a = extract::extract(&scene, &atlas, &camera_at(1.0), None);
    let frame_b = extract::extract(&scene, &atlas, &camera_at(4.0), None);
    let caption = |f: &extract::Frame| {
        f.ui.iter()
            .filter(|i| i.node == uid("n_text0000"))
            .map(|i| (i.pos, i.screen_offset, i.size))
            .collect::<Vec<_>>()
    };
    let a = caption(&frame_a);
    assert_eq!(a.len(), 3, "three inked digits");
    assert_eq!(
        a,
        caption(&frame_b),
        "a UI label does not move with the camera's zoom, flagged or not"
    );
}
