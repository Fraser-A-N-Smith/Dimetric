//! An animated tile is drawn from the tick, and the map never changes.
//!
//! `fn tiles` used to read a cell's id, subtract one, and slice the sheet. That
//! is every tile a still forever: water on a played floor could not ripple, and
//! the only way to make it was to step the map each tick — which would put a
//! decoration into the simulation, where a rollback would have to undo it and a
//! recorded run's hash would depend on the art.
//!
//! So the cycle is declared on the tileset and asked at draw time. These tests
//! are the two halves of that: the slice drawn does move with the tick, and the
//! scene it was drawn from does not move at all.

use dimetric_assets::meta::TileAnimation;
use dimetric_assets::tile::Animations;
use dimetric_core::{NodeUid, Tick};
use dimetric_render::atlas::Source;
use dimetric_render::{extract_at, Atlas, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::{Node, Scene, Value};

/// Ticks one frame is held at the timing these tests use.
///
/// 100ms at 60Hz. Spelt out rather than assumed, because every expectation
/// below is a multiple of it.
const HELD: u64 = 6;

/// A 64x16 sheet: four 16x16 slices, so tile ids 1 to 4.
fn sheet() -> Vec<Source> {
    vec![
        Source {
            name: "tilesets/water".into(),
            width: 64,
            height: 16,
            pixels: [0x40u8, 0x80, 0xC0, 0xFF].repeat(64 * 16),
        },
        dimetric_render::atlas::placeholder(8),
    ]
}

/// An atlas whose tile 1 cycles through slices 1, 2 and 4.
///
/// Three frames rather than two, and skipping slice 3, so a wrap is
/// distinguishable from a toggle and an index is distinguishable from an id.
fn animated() -> Atlas {
    let declared = [TileAnimation {
        id: 1,
        frames: vec![1, 2, 4],
        frame_ms: Some(100),
    }];
    let mut tiles = std::collections::BTreeMap::new();
    tiles.insert(
        "tilesets/water".to_string(),
        Animations::bake(&declared, 100, 60),
    );
    Atlas::pack(sheet(), 256).with_tile_animations(tiles)
}

/// The same atlas with nothing declared.
fn still() -> Atlas {
    Atlas::pack(sheet(), 256)
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// A one-cell floor holding `tile`.
fn floor(tile: u16) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .unwrap();
    let mut layer = Node::new(uid("n_floor000"), "TileLayer", "Floor");
    layer.props.insert(
        "tileset".into(),
        Value::Ref(dimetric_scene::Reference::Asset("tilesets/water".into())),
    );
    layer.props.insert("cell".into(), Value::Vec2i([16, 16]));
    let layer_uid = layer.uid;
    scene.insert(layer, Some(root)).unwrap();
    dimetric_scene::chunk::set_tile_in(&mut scene.chunks, layer_uid, 0, 0, tile).unwrap();
    scene.update_world_transforms();
    scene
}

/// Which slice the single drawn tile is taking, as a column index into the
/// sheet.
///
/// Read back from the texture coordinates rather than from any bookkeeping,
/// because the texture coordinates are what reaches the GPU.
fn slice_at(atlas: &Atlas, scene: &Scene, tick: u64) -> u32 {
    let camera = Camera::new((64, 64));
    let frame = extract_at(scene, atlas, &camera, None, Canvas::default(), Tick(tick));
    assert_eq!(frame.sprites.len(), 1, "one cell, one sprite");
    // The sheet is 64 wide and sits at the atlas origin after packing, so a
    // slice's left edge is its column times 16 texels.
    let u_min = frame.sprites[0].uv[0];
    (u_min * atlas.width as f32).round() as u32 / 16
}

#[test]
fn a_tile_with_no_cycle_is_still_a_still() {
    // Nothing declared, and nothing moves: the behaviour every project that
    // says nothing keeps.
    let atlas = still();
    let scene = floor(1);
    for tick in [0, 1, HELD, 1_000] {
        assert_eq!(slice_at(&atlas, &scene, tick), 0, "tick {tick}");
    }
}

#[test]
fn a_declared_tile_moves_with_the_tick() {
    let atlas = animated();
    let scene = floor(1);
    // Frames 1, 2 and 4 are columns 0, 1 and 3.
    for (tick, column) in [
        (0, 0),
        (HELD - 1, 0),
        (HELD, 1),
        (2 * HELD - 1, 1),
        (2 * HELD, 3),
        (3 * HELD - 1, 3),
    ] {
        assert_eq!(slice_at(&atlas, &scene, tick), column, "tick {tick}");
    }
}

#[test]
fn the_cycle_comes_back_round() {
    let atlas = animated();
    let scene = floor(1);
    for pass in 0..4u64 {
        let base = pass * 3 * HELD;
        assert_eq!(slice_at(&atlas, &scene, base), 0, "pass {pass}");
        assert_eq!(slice_at(&atlas, &scene, base + HELD), 1, "pass {pass}");
        assert_eq!(slice_at(&atlas, &scene, base + 2 * HELD), 3, "pass {pass}");
    }
}

#[test]
fn an_undeclared_tile_on_an_animated_sheet_is_left_alone() {
    // One sheet, one tile declared. The wall beside the water does not ripple.
    let atlas = animated();
    let scene = floor(3);
    for tick in [0, HELD, 2 * HELD, 97] {
        assert_eq!(slice_at(&atlas, &scene, tick), 2, "tick {tick}");
    }
}

#[test]
fn the_tick_moves_the_slice_and_nothing_else() {
    // The whole point, stated as a test: extraction reads the scene and writes
    // nothing back. If animation ever became a map edit, this is what notices
    // — and it notices before a replay fixture does.
    let atlas = animated();
    let scene = floor(1);
    let camera = Camera::new((64, 64));
    let before = scene.chunks.clone();
    let first = extract_at(&scene, &atlas, &camera, None, Canvas::default(), Tick(0));
    let later = extract_at(
        &scene,
        &atlas,
        &camera,
        None,
        Canvas::default(),
        Tick(2 * HELD),
    );
    assert_eq!(scene.chunks, before, "the map is what it was");
    assert_ne!(first.sprites[0].uv, later.sprites[0].uv, "the slice moved");
    assert_eq!(
        first.sprites[0].pos, later.sprites[0].pos,
        "the cell did not"
    );
    assert_eq!(
        first.sprites[0].size, later.sprites[0].size,
        "nor did its size"
    );
}

#[test]
fn the_shorter_doors_draw_the_first_frame() {
    // `extract` and `extract_with_canvas` are a still of a scene, which is
    // what an editor viewport and most of this crate's own tests want.
    let atlas = animated();
    let scene = floor(1);
    let camera = Camera::new((64, 64));
    let still = dimetric_render::extract(&scene, &atlas, &camera, None);
    let at_zero = extract_at(&scene, &atlas, &camera, None, Canvas::default(), Tick::ZERO);
    assert_eq!(still.sprites[0].uv, at_zero.sprites[0].uv);
}

#[test]
fn a_cycle_is_asked_for_every_cell_that_holds_the_tile() {
    // Not once per layer: two cells holding the same animated id are the same
    // slice, and two holding different ids are not.
    let atlas = animated();
    let mut scene = floor(1);
    dimetric_scene::chunk::set_tile_in(&mut scene.chunks, uid("n_floor000"), 1, 0, 1).unwrap();
    dimetric_scene::chunk::set_tile_in(&mut scene.chunks, uid("n_floor000"), 2, 0, 3).unwrap();
    scene.update_world_transforms();

    let camera = Camera::new((64, 64));
    let frame = extract_at(
        &scene,
        &atlas,
        &camera,
        None,
        Canvas::default(),
        Tick(2 * HELD),
    );
    let mut columns: Vec<u32> = frame
        .sprites
        .iter()
        .map(|s| (s.uv[0] * atlas.width as f32).round() as u32 / 16)
        .collect();
    columns.sort();
    // The two animated cells are both on frame 4 (column 3); the wall is
    // column 2 and did not move.
    assert_eq!(columns, vec![2, 3, 3]);
}
