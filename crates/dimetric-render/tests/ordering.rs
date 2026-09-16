//! Projection and draw ordering.

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_render::ProjectionRender as _;
use dimetric_render::{build, Blend, Camera, DrawItem, Projection, SortKey};

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

#[test]
fn top_down_is_the_identity() {
    let p = Projection::TopDown;
    assert_eq!(p.to_screen(Vec2Fx::from_ints(12, -7)), (12.0, -7.0));
    assert_eq!(p.matrix(), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn isometric_is_a_two_to_one_shear() {
    let p = Projection::Isometric;
    // One tile east is half a tile down and a tile right.
    assert_eq!(p.to_screen(Vec2Fx::from_ints(1, 0)), (1.0, 0.5));
    assert_eq!(p.to_screen(Vec2Fx::from_ints(0, 1)), (-1.0, 0.5));
    assert_eq!(p.to_screen(Vec2Fx::ZERO), (0.0, 0.0));
}

#[test]
fn projections_invert_themselves() {
    for p in [Projection::TopDown, Projection::Isometric] {
        for world in [(0.0f32, 0.0f32), (12.0, -7.0), (-3.5, 40.25)] {
            let screen = p.to_screen(Vec2Fx::new(
                Fx::from_f64_lossy(world.0 as f64),
                Fx::from_f64_lossy(world.1 as f64),
            ));
            let back = p.to_world(screen);
            assert!(
                (back.0 - world.0).abs() < 1e-4 && (back.1 - world.1).abs() < 1e-4,
                "{p:?} {world:?} -> {screen:?} -> {back:?}"
            );
        }
    }
}

#[test]
fn a_camera_round_trips_a_click_back_to_the_world() {
    for projection in [Projection::TopDown, Projection::Isometric] {
        let mut camera = Camera::new((320, 180));
        camera.projection = projection;
        camera.center = Vec2Fx::from_ints(100, 50);
        camera.zoom = 2.0;
        let world = Vec2Fx::from_ints(112, 64);
        let screen = camera.world_to_screen(world);
        let back = camera.screen_to_world(screen);
        assert!(
            (back.0 - 112.0).abs() < 0.01 && (back.1 - 64.0).abs() < 0.01,
            "{projection:?}: {screen:?} -> {back:?}"
        );
    }
}

#[test]
fn interpolation_stays_between_the_two_states() {
    let a = Vec2Fx::from_ints(0, 0);
    let b = Vec2Fx::from_ints(10, 20);
    assert_eq!(Camera::interpolate(a, b, 0.0), (0.0, 0.0));
    assert_eq!(Camera::interpolate(a, b, 1.0), (10.0, 20.0));
    assert_eq!(Camera::interpolate(a, b, 0.5), (5.0, 10.0));
    // Out-of-range alpha is clamped rather than extrapolated: a frame arriving
    // late should not draw the player past where they will be.
    assert_eq!(Camera::interpolate(a, b, 4.0), (10.0, 20.0));
}

#[test]
fn layer_beats_depth_and_depth_beats_the_batch_group() {
    let low_layer = SortKey::new(0, Fx::from_int(1000), 0, uid("n_aaaaaaaa"));
    let high_layer = SortKey::new(1, Fx::from_int(-1000), 9, uid("n_aaaaaaaa"));
    assert!(low_layer < high_layer, "layer must dominate");

    let near = SortKey::new(0, Fx::from_int(10), 9, uid("n_aaaaaaaa"));
    let far = SortKey::new(0, Fx::from_int(200), 0, uid("n_aaaaaaaa"));
    assert!(near < far, "depth must beat the batch group");
}

#[test]
fn negative_depth_sorts_before_positive_depth() {
    let above = SortKey::new(0, Fx::from_int(-500), 0, uid("n_aaaaaaaa"));
    let below = SortKey::new(0, Fx::from_int(500), 0, uid("n_aaaaaaaa"));
    assert!(above < below, "negative depth must not wrap past positive");
}

#[test]
fn identical_sprites_still_have_a_defined_order() {
    // Two sprites at the same place on the same layer must not swap between
    // runs; a golden-image test would catch it and nobody could explain it.
    let a = SortKey::new(0, Fx::ZERO, 0, uid("n_aaaaaaaa"));
    let b = SortKey::new(0, Fx::ZERO, 0, uid("n_bbbbbbbb"));
    assert_ne!(a, b);
}

#[test]
fn sub_unit_movement_does_not_change_sort_position() {
    // Depth is quantised to whole units, so two nearly-coincident sprites do
    // not flicker past each other as one drifts by a fraction of a pixel.
    let a = SortKey::new(0, Fx::from_int(10), 0, uid("n_aaaaaaaa"));
    let b = SortKey::new(0, Fx::parse_exact("10.25").unwrap(), 0, uid("n_aaaaaaaa"));
    assert_eq!(a, b);
}

fn item(layer: i32, depth: i32, atlas: u16, blend: Blend, id: &str) -> DrawItem {
    DrawItem {
        key: SortKey::new(
            layer,
            Fx::from_int(depth),
            dimetric_render::batch_group(atlas, 0, blend),
            uid(id),
        ),
        atlas,
        blend,
        shader: 0,
        pos: Vec2Fx::ZERO,
        size: Vec2Fx::ONE,
        rotation: dimetric_core::Angle::ZERO,
        uv: [0.0, 0.0, 1.0, 1.0],
        modulate: [255; 4],
        node: uid(id),
    }
}

#[test]
fn sprites_sharing_an_atlas_and_blend_mode_become_one_draw() {
    let mut items = vec![
        item(0, 10, 1, Blend::Alpha, "n_aaaaaaaa"),
        item(0, 20, 1, Blend::Alpha, "n_bbbbbbbb"),
        item(0, 30, 1, Blend::Alpha, "n_cccccccc"),
    ];
    let batches = build(&mut items);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].count, 3);
}

#[test]
fn batching_never_reorders_what_the_sort_decided() {
    // An additive sprite between two alpha ones has to split the batch. Merging
    // it away would draw something on top of what it should be behind.
    let mut items = vec![
        item(0, 30, 1, Blend::Alpha, "n_cccccccc"),
        item(0, 10, 1, Blend::Alpha, "n_aaaaaaaa"),
        item(0, 20, 1, Blend::Additive, "n_bbbbbbbb"),
    ];
    let batches = build(&mut items);
    assert_eq!(batches.len(), 3, "the additive sprite splits the run");
    let order: Vec<u64> = items.iter().map(|i| i.key.0).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "items must end up in sort order");
}

#[test]
fn batching_is_reproducible() {
    let make = || {
        vec![
            item(1, 5, 2, Blend::Alpha, "n_ddddddd1"),
            item(0, 30, 1, Blend::Alpha, "n_cccccccc"),
            item(0, 10, 1, Blend::Additive, "n_aaaaaaaa"),
            item(0, 10, 1, Blend::Alpha, "n_bbbbbbbb"),
        ]
    };
    let (mut a, mut b) = (make(), make());
    assert_eq!(build(&mut a), build(&mut b));
    assert_eq!(
        a.iter().map(|i| i.node).collect::<Vec<_>>(),
        b.iter().map(|i| i.node).collect::<Vec<_>>()
    );
}

#[test]
fn the_matrix_agrees_with_the_mapping_it_claims_to_encode() {
    // `matrix()` is what a GPU applies; `to_screen()` is what sorting and
    // picking use. Nothing checked they were the same transform, and the M0
    // spike found them disagreeing by a transpose — which stayed plausible on
    // screen, because the transpose of a 2:1 shear is another shear with the
    // same determinant, so the quad had the right area and the wrong shape.
    for projection in [Projection::TopDown, Projection::Isometric] {
        let [a, b, c, d] = projection.matrix();
        for (x, y) in [(1, 0), (0, 1), (3, -7), (-12, 5)] {
            let expected = projection.to_screen(Vec2Fx::from_ints(x, y));
            let (x, y) = (x as f32, y as f32);
            // Row-major: the first row produces the screen x, the second y.
            let applied = (a * x + b * y, c * x + d * y);
            assert_eq!(
                applied, expected,
                "{projection:?} matrix disagrees with to_screen at ({x}, {y})"
            );
        }
    }
}

#[test]
fn the_view_projection_puts_the_camera_centre_in_the_middle_of_the_screen() {
    // Applies the matrix the GPU is given, rather than checking its components.
    // The M0 spike shipped a transposed matrix that every component-level test
    // would have accepted, so this asserts the thing that actually matters:
    // where a world point lands in clip space.
    let apply = |m: [[f32; 4]; 4], x: f32, y: f32| {
        (
            m[0][0] * x + m[1][0] * y + m[3][0],
            m[0][1] * x + m[1][1] * y + m[3][1],
        )
    };

    for projection in [Projection::TopDown, Projection::Isometric] {
        let mut camera = Camera::new((320, 180));
        camera.projection = projection;
        camera.center = Vec2Fx::from_ints(40, 24);
        let m = camera.view_projection();

        let (x, y) = apply(m, 40.0, 24.0);
        assert!(
            x.abs() < 1e-5 && y.abs() < 1e-5,
            "{projection:?}: the camera centre should map to clip origin, got ({x}, {y})"
        );
    }
}

#[test]
fn the_view_projection_shears_and_does_not_transpose() {
    // Under the 2:1 shear, one unit of world +x must move the point right and
    // *down* the screen. A transposed matrix moves it right and up, which is
    // still a plausible-looking diamond and completely wrong.
    let mut camera = Camera::new((320, 180));
    camera.projection = Projection::Isometric;
    camera.zoom = 1.0;
    let m = camera.view_projection();

    let at = |x: f32, y: f32| (m[0][0] * x + m[1][0] * y, m[0][1] * x + m[1][1] * y);
    let (px, py) = at(100.0, 0.0);
    assert!(px > 0.0, "world +x should move right, got {px}");
    assert!(py < 0.0, "world +x should move down the screen, got {py}");

    let (qx, qy) = at(0.0, 100.0);
    assert!(qx < 0.0, "world +y should move left, got {qx}");
    assert!(qy < 0.0, "world +y should move down the screen, got {qy}");
}

#[test]
fn sprites_at_one_depth_are_grouped_by_what_the_batcher_splits_on() {
    // The stress scene is projectiles and enemies at every depth, and before
    // this the two blend modes alternated all the way down: 440 sprites in 27
    // draw calls. Two sprites at the same quantised depth are in no meaningful
    // order, so putting the ones that can be drawn together next to each other
    // costs nothing and saves a third of the draws.
    let mut items = vec![
        item(0, 10, 0, Blend::Alpha, "n_aaaaaaaa"),
        item(0, 10, 0, Blend::Additive, "n_bbbbbbbb"),
        item(0, 10, 0, Blend::Alpha, "n_cccccccc"),
        item(0, 10, 0, Blend::Additive, "n_dddddddd"),
    ];
    let batches = build(&mut items);
    assert_eq!(batches.len(), 2, "one batch per blend, not four");
}

#[test]
fn the_sort_group_is_exactly_what_the_batcher_splits_on() {
    // If these ever disagree the sort produces a tidy order and the same number
    // of draw calls, which is an optimisation that looks like it works.
    let a = item(0, 10, 0, Blend::Alpha, "n_aaaaaaaa");
    let b = item(0, 10, 1, Blend::Alpha, "n_bbbbbbbb");
    let c = item(0, 10, 0, Blend::Additive, "n_cccccccc");
    assert_ne!(a.key.group(), b.key.group(), "a different atlas splits");
    assert_ne!(a.key.group(), c.key.group(), "a different blend splits");

    let d = item(0, 10, 0, Blend::Alpha, "n_dddddddd");
    assert_eq!(a.key.group(), d.key.group(), "the same pair does not");
}

#[test]
fn depth_still_wins_over_the_group() {
    // Grouping is a tie-break and nothing more. An additive sprite in front of
    // an alpha one is drawn in front of it, however many draws that costs.
    let mut items = vec![
        item(0, 30, 0, Blend::Alpha, "n_cccccccc"),
        item(0, 20, 0, Blend::Additive, "n_bbbbbbbb"),
        item(0, 10, 0, Blend::Alpha, "n_aaaaaaaa"),
    ];
    let batches = build(&mut items);
    assert_eq!(batches.len(), 3);
    assert_eq!(items[0].node, uid("n_aaaaaaaa"));
    assert_eq!(items[2].node, uid("n_cccccccc"));
}
