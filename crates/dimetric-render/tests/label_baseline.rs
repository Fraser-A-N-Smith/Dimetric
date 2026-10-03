//! A world-space `Label` lays its glyphs along a line, under either projection.
//!
//! `Projection::Isometric` is `screen = (x - y, (x + y) / 2)`. A label used to
//! put each glyph at `anchor + advance` as a **world** position, so the advance
//! went through that shear and every glyph landed one step down and to the
//! right of the last. "160" came out as three digits on a descending diagonal,
//! and every number a game wants over its board was unreadable.
//!
//! A label's anchor is world geometry and should project. Its glyph advance is
//! typography and should not — exactly the split a `Sprite2D` already makes,
//! where the origin projects and the quad is drawn axis-aligned. The shader
//! has always said so in a comment: "the centre goes through the projection;
//! the quad is added in screen space afterwards."

use dimetric_core::NodeUid;
use dimetric_render::{extract, Atlas, Camera, Projection};
use dimetric_scene::{Node, Scene, Value};

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// An atlas holding only the engine's own font.
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

/// A scene with one label at the world origin.
fn scene(text: &str) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node", "Root"), None)
        .unwrap();
    let mut label = Node::new(uid("n_lbltext0"), "Label", "Probe");
    label
        .props
        .insert("text".into(), Value::Str(text.to_string()));
    scene.insert(label, Some(root)).unwrap();
    scene.update_world_transforms();
    scene
}

fn camera(projection: Projection) -> Camera {
    Camera {
        projection,
        ..Camera::new((400, 120))
    }
}

/// Where each glyph lands on screen, in pixels, in layout order.
///
/// The same arithmetic the shader does: project the centre, add the quad's own
/// offset afterwards.
fn glyph_positions(scene: &Scene, projection: Projection) -> Vec<(i32, i32)> {
    let frame = extract(scene, &bare_atlas(), &camera(projection), None);
    frame
        .sprites
        .iter()
        .map(|item| {
            let p = projection.project(item.pos);
            (
                (p.x + item.screen_offset.x).round_int(),
                (p.y + item.screen_offset.y).round_int(),
            )
        })
        .collect()
}

#[test]
fn glyphs_share_a_baseline_under_isometric() {
    // The repro. Under the shear every glyph used to be lower than the last.
    let scene = scene("MMM160");
    let placed = glyph_positions(&scene, Projection::Isometric);
    assert!(placed.len() >= 6, "six inked glyphs: {placed:?}");
    let baseline = placed[0].1;
    assert!(
        placed.iter().all(|(_, y)| *y == baseline),
        "the glyphs descend instead of sharing a baseline: {placed:?}"
    );
}

#[test]
fn glyphs_advance_only_rightwards_under_isometric() {
    let scene = scene("MMM160");
    let placed = glyph_positions(&scene, Projection::Isometric);
    for pair in placed.windows(2) {
        assert!(
            pair[1].0 > pair[0].0,
            "a glyph did not advance to the right of the last: {placed:?}"
        );
    }
}

#[test]
fn the_two_projections_lay_the_same_text_out_the_same_way() {
    // Typography does not depend on the camera. The anchor may land somewhere
    // else; the shape of the run may not.
    let scene = scene("MMM160");
    let relative = |p: Projection| {
        let placed = glyph_positions(&scene, p);
        let first = placed[0];
        placed
            .iter()
            .map(|(x, y)| (x - first.0, y - first.1))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        relative(Projection::Isometric),
        relative(Projection::TopDown)
    );
}

#[test]
fn the_anchor_still_projects() {
    // Only the advance is exempt. A label at a world position has to move with
    // the board it labels, or it is not a world-space label at all.
    let mut shifted = scene("M");
    let id = shifted.resolve_path("/Root/Probe").expect("label");
    shifted.set_position(id, dimetric_core::Vec2Fx::from_ints(40, 0));
    shifted.update_world_transforms();

    let at_origin = glyph_positions(&scene("M"), Projection::Isometric);
    let moved = glyph_positions(&shifted, Projection::Isometric);
    // (40, 0) projects to (40, 20) under the shear, so the glyph moves on both
    // axes even though its advance does not.
    assert_eq!(moved[0].0 - at_origin[0].0, 40);
    assert_eq!(moved[0].1 - at_origin[0].1, 20);
}

#[test]
fn a_newline_still_starts_a_new_line() {
    let scene = scene("A\nB");
    let placed = glyph_positions(&scene, Projection::Isometric);
    assert_eq!(placed.len(), 2, "{placed:?}");
    assert_eq!(
        placed[0].0, placed[1].0,
        "the second line is not flush left"
    );
    assert!(placed[1].1 > placed[0].1, "the second line is not below");
}
