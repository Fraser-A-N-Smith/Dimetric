//! What the last blit does to a frame that is not on a whole scale.
//!
//! Everything inside the frame is drawn with nearest sampling, and should be:
//! that is where pixel art lives. The *present* pass used to share that
//! sampler, and at a fractional scale nearest cannot reproduce a frame — it can
//! only drop source rows or double them. At 0.9 it drops one row and one column
//! in ten.
//!
//! On a sprite that loses a pixel here and there. On a letter it loses a
//! stroke: a game's run menu in a 1728×972 window read *Aim en action*, because
//! the bowl of the `a` sat on one of the rows that went, and *Act* lost the bar
//! of its `t`. Which strokes go moves with the window size, so it is not
//! something a font or a layout can be designed around.
//!
//! The frame here is twenty pixels tall with exactly one row of ink in it, which
//! is the defect reduced to the smallest thing that still shows it. At 0.9 two
//! of those twenty rows are never sampled, so for two choices of ink row the
//! finished picture is *blank*.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{
    extract, headless_instance, Atlas, Camera, Capture, PresentFilter, RenderSettings, Renderer,
};
use dimetric_scene::{Color, Node, Scene, Value};

/// The internal frame's size, and the sprite's.
const SIDE: u32 = 20;
/// The output the fractional case is presented into: 0.9 of the frame.
const SMALL: u32 = 18;

const INK: [u8; 4] = [0xE8, 0xC4, 0x8A, 0xFF];
const PAPER: [u8; 4] = [0x14, 0x14, 0x1C, 0xFF];

/// A frame-sized texture that is `PAPER` everywhere except one row of `INK`.
///
/// One row rather than a stripe pattern, because the question is whether a row
/// survives at all and a pattern lets a surviving neighbour stand in for it.
fn atlas_with_ink_on(row: u32) -> Atlas {
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        let colour = match y == row {
            true => INK,
            false => PAPER,
        };
        for _ in 0..SIDE {
            pixels.extend_from_slice(&colour);
        }
    }
    Atlas::pack(
        vec![
            Source {
                name: "sprites/page".into(),
                width: SIDE,
                height: SIDE,
                pixels,
            },
            dimetric_render::atlas::placeholder(8),
        ],
        256,
    )
}

fn settings(filter: PresentFilter) -> RenderSettings {
    RenderSettings {
        internal_resolution: (SIDE, SIDE),
        integer_upscale: true,
        pixel_snap: true,
        ambient: Color::WHITE,
        present_filter: filter,
    }
}

/// One sprite covering the whole frame.
fn page() -> Scene {
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
    scene.update_world_transforms();
    scene
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).expect("a node id")
}

/// Render the frame with ink on `row` into an `output`-sized target, or `None`
/// when this machine has no adapter.
fn render(row: u32, filter: PresentFilter, output: (u32, u32)) -> Option<Vec<u8>> {
    let atlas = atlas_with_ink_on(row);
    let instance = headless_instance();
    let mut renderer = match Renderer::new(&instance, None, &atlas, settings(filter)) {
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
    let capture = Capture::new(&renderer, output);
    let mut camera = Camera::new((SIDE, SIDE));
    camera.zoom = 1.0;
    camera.center = Vec2Fx::ZERO;
    let frame = extract(&page(), &atlas, &camera, None);
    Some(capture.render(&mut renderer, &frame).expect("a frame"))
}

/// How close a pixel is to the ink's colour, ignoring alpha.
fn inkiness(pixel: &[u8]) -> i32 {
    (0..3)
        .map(|c| (pixel[c] as i32 - PAPER[c] as i32).abs())
        .sum()
}

/// How much of the ink survived: the largest departure from the paper colour
/// anywhere in the picture.
fn surviving_ink(pixels: &[u8]) -> i32 {
    pixels
        .chunks_exact(4)
        .map(inkiness)
        .max()
        .unwrap_or_default()
}

/// The ink's full strength, for scale.
fn full_ink() -> i32 {
    inkiness(&INK)
}

#[test]
fn the_frame_is_exact_at_a_whole_scale() {
    // The premise everything below rests on: one row of ink in, one row of ink
    // out, in the right place. If the sprite did not land on whole pixels the
    // rest of this would be measuring the wrong thing.
    let Some(pixels) = render(4, PresentFilter::Auto, (SIDE, SIDE)) else {
        return;
    };
    let rows: Vec<u32> = (0..SIDE)
        .filter(|y| {
            (0..SIDE).any(|x| {
                let i = ((y * SIDE + x) * 4) as usize;
                inkiness(&pixels[i..i + 4]) > full_ink() / 2
            })
        })
        .collect();
    assert_eq!(rows.len(), 1, "ink on {rows:?}");
}

#[test]
fn nearest_sampling_loses_whole_rows_at_a_fractional_scale() {
    // The defect, demonstrated rather than asserted: of the twenty rows a frame
    // has, at 0.9 there are two that nearest never reads. Ink on one of those
    // is a picture with nothing in it.
    let mut lost = Vec::new();
    for row in 0..SIDE {
        let Some(pixels) = render(row, PresentFilter::Nearest, (SMALL, SMALL)) else {
            return;
        };
        if surviving_ink(&pixels) == 0 {
            lost.push(row);
        }
    }
    assert!(
        !lost.is_empty(),
        "nearest kept every row at 0.9, and this test is stale"
    );
    eprintln!("nearest loses rows {lost:?} of {SIDE} at 0.9");
}

#[test]
fn every_row_survives_a_fractional_scale_by_default() {
    // The fix. Not "most rows", and not "a row is dimmer": every one of the
    // twenty leaves a mark, because a missing stroke is a word a player cannot
    // read and a soft one is a word they can.
    for row in 0..SIDE {
        let Some(pixels) = render(row, PresentFilter::Auto, (SMALL, SMALL)) else {
            return;
        };
        let kept = surviving_ink(&pixels);
        assert!(
            kept > full_ink() / 4,
            "row {row} of {SIDE} came through at {kept} of {}",
            full_ink()
        );
    }
}

#[test]
fn a_whole_scale_is_untouched_by_the_default() {
    // Only the fractional case changes. At 1x and 2x the default and nearest
    // have to agree exactly, or this round would have softened every pixel-art
    // game that was already right.
    for output in [(SIDE, SIDE), (SIDE * 2, SIDE * 2), (SIDE * 3, SIDE * 3)] {
        let Some(auto) = render(4, PresentFilter::Auto, output) else {
            return;
        };
        let Some(nearest) = render(4, PresentFilter::Nearest, output) else {
            return;
        };
        assert_eq!(auto, nearest, "{output:?} differed");
    }
}

#[test]
fn a_project_that_asks_for_nearest_gets_it() {
    // A real preference for some pixel art: a frame with rows missing over a
    // soft one. Asking for it has to actually mean it, or the setting is a
    // comment.
    let Some(auto) = render(4, PresentFilter::Auto, (SMALL, SMALL)) else {
        return;
    };
    let Some(nearest) = render(4, PresentFilter::Nearest, (SMALL, SMALL)) else {
        return;
    };
    assert_ne!(auto, nearest, "the setting changed nothing");
    assert_eq!(surviving_ink(&nearest), 0, "row 4 is one nearest drops");
}

#[test]
fn a_project_that_asks_for_linear_gets_it_even_at_a_whole_scale() {
    // For art that was never on a pixel grid. The frame is blurred at 2x, which
    // is what was asked for.
    let Some(linear) = render(4, PresentFilter::Linear, (SIDE * 2, SIDE * 2)) else {
        return;
    };
    let Some(nearest) = render(4, PresentFilter::Nearest, (SIDE * 2, SIDE * 2)) else {
        return;
    };
    assert_ne!(linear, nearest);
}
