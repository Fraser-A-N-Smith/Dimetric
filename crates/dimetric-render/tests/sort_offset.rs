//! A sprite can be drawn at one point and sorted at another.
//!
//! Walls on a board stand up: the ground diamond fills the bottom of the
//! sprite and the block rises above it. A figure behind a wall should be hidden
//! by it and one in front should cover it, which is what depth sorting is for —
//! the engine already sorts a `TileLayer`'s tiles against sprites in the same
//! layer by the depth of their centres.
//!
//! What stopped it: a figure is taller than its cell, so it is drawn lifted for
//! its feet to land on the floor, and `offset` fed the depth as well. Lift a
//! figure six units and it sorts six units *behind* its own floor tile, which
//! then draws over its feet. The two ways out were both a way of lying to the
//! sorter — pad every sheet with transparent rows until the centre falls at the
//! feet, or put the figures on a layer above all tiles, which fixes the walls
//! in front and breaks the walls behind.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera};
use dimetric_scene::{Node, Scene, Value};

/// A 16-pixel cell, and a figure 28 pixels tall.
///
/// The game's numbers. A 28-tall sprite centred on a 16-tall cell has to come
/// up six units for its feet to land on the floor: `(28 - 16) / 2`.
const CELL: i32 = 16;
const LIFT: i32 = -6;

/// A sheet holding one 16x16 floor tile and one 16x28 figure.
fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "tiles/floor".into(),
                width: 16,
                height: 16,
                pixels: [0x40u8, 0x80, 0xC0, 0xFF].repeat(16 * 16),
            },
            Source {
                name: "sprites/figure".into(),
                width: 16,
                height: 28,
                pixels: [0xC0u8, 0x40, 0x40, 0xFF].repeat(16 * 28),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        256,
    )
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// Three floor cells in a column, and a figure standing on the middle one.
///
/// `sort_offset` is what the test varies. Everything else is what the game
/// ships: the figure lifted so its feet land on its cell, on the same layer as
/// the floor so the two sort against each other at all.
fn board(sort_offset: Option<[i32; 2]>) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .unwrap();

    let mut layer = Node::new(uid("n_floor000"), "TileLayer", "Floor");
    layer.props.insert(
        "tileset".into(),
        Value::Ref(dimetric_scene::Reference::Asset("tiles/floor".into())),
    );
    layer
        .props
        .insert("cell".into(), Value::Vec2i([CELL, CELL]));
    let layer_uid = layer.uid;
    scene.insert(layer, Some(root)).unwrap();
    for y in 0..3 {
        dimetric_scene::chunk::set_tile_in(&mut scene.chunks, layer_uid, 0, y, 1).unwrap();
    }

    let mut figure = Node::new(uid("n_figure00"), "Sprite2D", "Figure");
    figure.props.insert(
        "texture".into(),
        Value::Ref(dimetric_scene::Reference::Asset("sprites/figure".into())),
    );
    // The centre of the middle cell: half a cell in, one whole cell down.
    figure.transform.pos = Vec2Fx::from_ints(CELL / 2, CELL + CELL / 2);
    figure
        .props
        .insert("offset".into(), Value::Vec2(Vec2Fx::from_ints(0, LIFT)));
    if let Some(offset) = sort_offset {
        figure.props.insert(
            "sort_offset".into(),
            Value::Vec2(Vec2Fx::from_ints(offset[0], offset[1])),
        );
    }
    scene.insert(figure, Some(root)).unwrap();
    scene.update_world_transforms();
    scene
}

/// The drawn order of the four sprites, named.
///
/// Read out of the sort keys the extraction produced, sorted the way the
/// batcher sorts them, so this is the order they actually reach the GPU in.
fn order(scene: &Scene) -> Vec<String> {
    let frame = extract(scene, &atlas(), &Camera::new((64, 128)), None);
    let mut items: Vec<_> = frame.sprites.iter().collect();
    items.sort_by_key(|item| item.key);
    items
        .iter()
        .map(|item| match item.node == uid("n_figure00") {
            true => "figure".to_string(),
            // A tile's y in cells, from the centre the extraction gave it.
            false => format!("floor{}", (item.pos.y.round_int() - CELL / 2) / CELL),
        })
        .collect()
}

#[test]
fn a_lifted_figure_used_to_sort_behind_its_own_floor() {
    // The defect, reproduced. The figure stands on `floor1` and is drawn six
    // units up, so it sorts at depth 18 where its own tile is at 24 — before
    // its own floor, which then draws over its feet.
    assert_eq!(
        order(&board(None)),
        ["floor0", "figure", "floor1", "floor2"],
        "the figure sorts before the tile it is standing on"
    );
}

#[test]
fn sorting_at_the_feet_puts_the_figure_on_its_own_floor() {
    // Six units back to undo the lift, and one more so the figure sorts
    // strictly past its own cell's centre rather than tying with it.
    assert_eq!(
        order(&board(Some([0, -LIFT + 1]))),
        ["floor0", "floor1", "figure", "floor2"],
        "the figure covers the floor it stands on and not the one in front"
    );
}

#[test]
fn the_tile_in_front_still_draws_over_the_figure() {
    // The whole point: a wall in the cell in front covers the figure, and the
    // one it stands on does not. `floor2` is that cell.
    let drawn = order(&board(Some([0, -LIFT + 1])));
    let figure = drawn.iter().position(|n| n == "figure").unwrap();
    let in_front = drawn.iter().position(|n| n == "floor2").unwrap();
    let behind = drawn.iter().position(|n| n == "floor0").unwrap();
    assert!(behind < figure, "the cell behind is covered: {drawn:?}");
    assert!(figure < in_front, "the cell in front covers: {drawn:?}");
}

#[test]
fn a_sort_offset_moves_nothing_that_is_drawn() {
    // Added to the depth position and to nothing else. If it ever reached the
    // draw position the figure would slide across the board.
    let plain = extract(&board(None), &atlas(), &Camera::new((64, 128)), None);
    let sorted = extract(
        &board(Some([0, 64])),
        &atlas(),
        &Camera::new((64, 128)),
        None,
    );
    let pick = |frame: &dimetric_render::Frame| {
        frame
            .sprites
            .iter()
            .find(|item| item.node == uid("n_figure00"))
            .map(|item| (item.pos, item.size, item.uv))
            .expect("the figure is drawn")
    };
    assert_eq!(pick(&plain), pick(&sorted), "only the sort key may move");
}

#[test]
fn no_sort_offset_is_exactly_the_old_behaviour() {
    // The default has to be a no-op, because every scene already written
    // depends on it. An absent property and an explicit zero are the same
    // thing.
    let absent = extract(&board(None), &atlas(), &Camera::new((64, 128)), None);
    let zero = extract(
        &board(Some([0, 0])),
        &atlas(),
        &Camera::new((64, 128)),
        None,
    );
    let keys = |frame: &dimetric_render::Frame| {
        let mut keys: Vec<_> = frame.sprites.iter().map(|item| item.key).collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&absent), keys(&zero));
}

#[test]
fn a_sort_offset_works_under_the_isometric_shear_too() {
    // Depth is `x + y` under Isometric rather than `y`, and `sort_offset` is
    // in world units, so the same nudge has to carry through the projection
    // rather than being a screen-space fudge.
    let camera = Camera {
        projection: dimetric_render::Projection::Isometric,
        ..Camera::new((64, 128))
    };
    let drawn = |scene: &Scene| {
        let frame = extract(scene, &atlas(), &camera, None);
        let mut items: Vec<_> = frame.sprites.iter().collect();
        items.sort_by_key(|item| item.key);
        items.iter().map(|item| item.node).collect::<Vec<_>>()
    };
    let plain = drawn(&board(None));
    let lifted = drawn(&board(Some([0, -LIFT + 1])));
    assert_ne!(plain, lifted, "the nudge changed the order under Isometric");
    // Its own cell is `floor1`; under `x + y` the figure at (8, 24) with the
    // nudge sits past that cell's centre and before the next one's.
    let figure = lifted.iter().position(|n| *n == uid("n_figure00")).unwrap();
    assert_eq!(figure, 2, "between its own cell and the one in front");
}
