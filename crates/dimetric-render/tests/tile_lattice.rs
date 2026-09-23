//! A `TileLayer` can draw the 2:1 tile a dimetric board is made of.
//!
//! `Projection::Isometric` is `screen = (x - y, (x + y) / 2)`. For a grid with
//! step `W x H`, the neighbour at `+1 cx` lands at screen `(W, W/2)` and the
//! one at `+1 cy` at `(-H, H/2)`. Diamonds of width `TW` and height `TH`
//! tessellate when neighbours sit at `(TW/2, TH/2)` and `(-TW/2, TH/2)`.
//! Solving both gives `W = H` — a square step — and `TW = 2*TH` — a 2:1
//! sprite. That is the standard dimetric arrangement, and it is exactly what
//! this projection is for.
//!
//! `cell` used to be three things at once: the slice out of the sheet, the
//! size drawn, and the step between neighbours. A square step therefore forced
//! a square sprite, so the arrangement above could not be written down.
//! `cell = [32, 16]` gapped every other neighbour and `cell = [16, 16]` sliced
//! half of each tile, and there was no third value to try.

use dimetric_core::NodeUid;
use dimetric_render::atlas::Source;
use dimetric_render::{extract, Atlas, Camera, Projection};
use dimetric_scene::{Node, Scene, Value};

/// A sheet holding two 32x16 tiles side by side.
fn atlas() -> Atlas {
    Atlas::pack(
        vec![
            Source {
                name: "tiles/floor".into(),
                width: 64,
                height: 16,
                pixels: [0x40u8, 0x80, 0xC0, 0xFF].repeat(64 * 16),
            },
            dimetric_render::atlas::placeholder(8),
        ],
        256,
    )
}

fn uid(s: &str) -> NodeUid {
    NodeUid::parse(s).unwrap()
}

/// A layer with the given `cell` and optional `tile_size`, holding tiles at
/// `(0,0)`, `(1,0)` and `(0,1)`.
fn board(cell: [i32; 2], tile_size: Option<[i32; 2]>) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid("n_root0000"), "Node2D", "Root"), None)
        .unwrap();
    let mut layer = Node::new(uid("n_floor000"), "TileLayer", "Floor");
    layer.props.insert(
        "tileset".into(),
        Value::Ref(dimetric_scene::Reference::Asset("tiles/floor".into())),
    );
    layer.props.insert("cell".into(), Value::Vec2i(cell));
    if let Some(size) = tile_size {
        layer.props.insert("tile_size".into(), Value::Vec2i(size));
    }
    let layer_uid = layer.uid;
    scene.insert(layer, Some(root)).unwrap();
    for (x, y) in [(0, 0), (1, 0), (0, 1)] {
        dimetric_scene::chunk::set_tile_in(&mut scene.chunks, layer_uid, x, y, 1).unwrap();
    }
    scene.update_world_transforms();
    scene
}

fn camera() -> Camera {
    Camera {
        projection: Projection::Isometric,
        ..Camera::new((64, 64))
    }
}

/// Where the `+1 cx` and `+1 cy` neighbours land relative to the tile at the
/// origin, on screen, in pixels.
///
/// The offsets are what the lattice is about; the absolute position depends on
/// where the cells sit in the world and says nothing.
fn neighbour_offsets(scene: &Scene) -> ((i32, i32), (i32, i32)) {
    let c = screen_centres(scene);
    assert_eq!(c.len(), 3, "three tiles: {c:?}");
    // Sorted by x: the +cy neighbour is left, the origin is between, and the
    // +cx neighbour is right — under a projection that shears x by `-y`.
    let (left, mid, right) = (c[0], c[1], c[2]);
    (
        (right.0 - mid.0, right.1 - mid.1),
        (left.0 - mid.0, left.1 - mid.1),
    )
}

/// Where each drawn tile's centre lands on screen, in pixels, sorted.
fn screen_centres(scene: &Scene) -> Vec<(i32, i32)> {
    let frame = extract(scene, &atlas(), &camera(), None);
    let mut out: Vec<(i32, i32)> = frame
        .sprites
        .iter()
        .map(|item| {
            let p = Projection::Isometric.project(item.pos);
            (p.x.round_int(), p.y.round_int())
        })
        .collect();
    out.sort();
    out
}

/// The pixel width and height each tile is drawn at.
fn drawn_sizes(scene: &Scene) -> Vec<(i32, i32)> {
    let frame = extract(scene, &atlas(), &camera(), None);
    frame
        .sprites
        .iter()
        .map(|item| (item.size.x.round_int(), item.size.y.round_int()))
        .collect()
}

#[test]
fn a_square_step_with_a_two_to_one_sprite_tessellates() {
    // The arrangement the projection is built for: neighbours exactly half a
    // tile across and half a tile down from each other, which is what makes
    // the diamonds meet edge to edge instead of gapping or overlapping.
    let scene = board([16, 16], Some([32, 16]));
    assert_eq!(drawn_sizes(&scene), vec![(32, 16); 3], "the sprite is 2:1");

    // Half a tile across and half a tile down, in both directions: tile width
    // 32, height 16, so (16, 8) and (-16, 8).
    assert_eq!(
        neighbour_offsets(&scene),
        ((16, 8), (-16, 8)),
        "the diamonds do not meet edge to edge"
    );
}

#[test]
fn the_sheet_is_sliced_at_the_sprites_size_not_the_steps() {
    // The other half of the old conflation: with `cell = [16, 16]` the engine
    // cut 16x16 out of a 32x16 tile and drew half of each, so the floor
    // collapsed. The sheet is 64 wide and the tiles are 32, so tile 1 is the
    // left half of it and its UVs must span exactly that.
    let scene = board([16, 16], Some([32, 16]));
    let frame = extract(&scene, &atlas(), &camera(), None);
    let uv = frame.sprites[0].uv;
    let width = uv[2] - uv[0];
    let sheet = atlas().region("tiles/floor").expect("packed").uv;
    let sheet_width = sheet[2] - sheet[0];
    assert!(
        (width - sheet_width / 2.0).abs() < 1e-5,
        "sliced {width} of {sheet_width}, expected half"
    );
}

#[test]
fn a_layer_that_says_nothing_draws_exactly_as_it_did() {
    // Top-down projects are unaffected, and so is any layer already working:
    // absent `tile_size` means `cell`, which is what `cell` meant before.
    let implied = board([16, 16], None);
    let spelled = board([16, 16], Some([16, 16]));
    assert_eq!(drawn_sizes(&implied), drawn_sizes(&spelled));
    assert_eq!(screen_centres(&implied), screen_centres(&spelled));
    assert_eq!(drawn_sizes(&implied), vec![(16, 16); 3]);
}

#[test]
fn the_step_stays_the_step_when_the_sprite_grows() {
    // `tile_size` must not move anything: a bigger sprite covers more of its
    // neighbours, it does not push them apart. The centres are the same as
    // the square-sprite layer's.
    let big = board([16, 16], Some([32, 16]));
    let plain = board([16, 16], None);
    assert_eq!(
        screen_centres(&big),
        screen_centres(&plain),
        "the sprite's size leaked into the grid's step"
    );
}

#[test]
fn a_nonsense_tile_size_falls_back_to_the_cell() {
    let scene = board([16, 16], Some([0, -4]));
    assert_eq!(drawn_sizes(&scene), vec![(16, 16); 3]);
}

#[test]
fn the_old_two_to_one_cell_is_still_the_old_two_to_one_cell() {
    // Not a regression test so much as a record of why `tile_size` had to
    // exist: `cell = [32, 16]` still steps 32 across, which under Isometric
    // puts the +cx neighbour a whole tile-width away instead of half, and
    // that is the gapping the report measured. Nothing here changes it —
    // `cell` means the step, and it always did.
    let scene = board([32, 16], None);
    let (cx, _) = neighbour_offsets(&scene);
    assert_eq!(cx, (32, 16), "a 32-wide step still steps 32");
    assert_ne!(
        cx,
        (16, 8),
        "which is twice as far as a tessellating neighbour, hence the gaps"
    );
}
