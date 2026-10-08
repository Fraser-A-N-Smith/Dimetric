//! A tileset's sidecar can say which tile ids move, and what they cycle through.
//!
//! A tileset is a sheet of slices and a map cell holds one id. That is enough
//! for a wall and not enough for water: water, a torch and a portal are one
//! tile to the map and several slices to the eye. Nothing in the PNG says that
//! slices 5, 33, 61 and 89 are one animated tile, which makes it exactly the
//! kind of thing the `.meta` is for — the same argument `[[clip]]` won.
//!
//! The alternative is to animate the map: step every animated cell to its next
//! id each tick. That would put a decoration in the simulation, where a
//! rollback would have to undo a ripple and the state hash of a recorded run
//! would depend on the art. These tests are about the shape that avoids it: a
//! declaration on the sheet, baked to ticks at import, asked at draw time.

use dimetric_assets::meta::{ImportSettings, MetaError, TileAnimation};
use dimetric_assets::tile::Animations;
use dimetric_core::{AssetId, Tick};

fn settings(body: &str) -> Result<ImportSettings, MetaError> {
    ImportSettings::parse(&format!("id = \"a_lu48nofm\"\n{body}"))
}

fn declared() -> Vec<TileAnimation> {
    vec![TileAnimation {
        id: 5,
        frames: vec![5, 33, 61, 89],
        frame_ms: Some(240),
    }]
}

#[test]
fn a_sheet_can_declare_which_tiles_move() {
    let parsed = settings(
        "
[[tile]]
id = 5
frames = [5, 33, 61, 89]
frame_ms = 240
",
    )
    .expect("a well-formed tile block parses");
    assert_eq!(parsed.tiles, declared());
}

#[test]
fn a_sheet_that_says_nothing_has_no_animated_tiles() {
    let parsed = settings("nearest = true\n").unwrap();
    assert!(
        parsed.tiles.is_empty(),
        "the old behaviour exactly: {:?}",
        parsed.tiles
    );
}

#[test]
fn what_was_written_is_what_is_read_back() {
    // `to_text` is hand-written, so the round trip is the only thing holding
    // it to the parser. A setting that survives a save and comes back changed
    // is how a sidecar starts disagreeing with the game.
    let mut written = ImportSettings::new(AssetId::parse("a_lu48nofm").unwrap());
    written.tiles = vec![
        TileAnimation {
            id: 5,
            frames: vec![5, 33, 61, 89],
            frame_ms: Some(240),
        },
        TileAnimation {
            id: 12,
            frames: vec![12, 13],
            frame_ms: None,
        },
    ];
    let text = written.to_text();
    let read = ImportSettings::parse(&text).expect("what we wrote parses");
    assert_eq!(read.tiles, written.tiles, "round trip of:\n{text}");
}

#[test]
fn a_tile_block_can_sit_beside_a_clip_block() {
    // Both are arrays of tables, and TOML lets every key after one belong to
    // it. If `to_text` ever emits a scalar below either, this is what notices.
    let parsed = settings(
        "
frames = 4
frame_ms = 100

[[clip]]
name = \"idle\"
from = 0
to = 3

[[tile]]
id = 1
frames = [1, 2]
",
    )
    .expect("a sheet may have both");
    assert_eq!(parsed.clips.len(), 1);
    assert_eq!(parsed.tiles.len(), 1);
    assert_eq!(parsed.frames, 4, "the strip count survived the tables");
}

#[test]
fn a_cycle_nothing_could_draw_is_refused() {
    // Refused rather than repaired: a tile animation that quietly fell back to
    // a still is a floor that does not move, which looks exactly like art
    // somebody has not finished yet.
    for (body, why) in [
        ("[[tile]]\nframes = [1, 2]\n", "no id"),
        (
            "[[tile]]\nid = 0\nframes = [1, 2]\n",
            "id 0 is the empty cell",
        ),
        ("[[tile]]\nid = 1\n", "no frames"),
        ("[[tile]]\nid = 1\nframes = []\n", "an empty cycle"),
        (
            "[[tile]]\nid = 1\nframes = [1, 0]\n",
            "frame 0 is not a tile",
        ),
        ("[[tile]]\nid = 1\nframes = [1, -2]\n", "a negative frame"),
        (
            "[[tile]]\nid = 1\nframes = [1]\n\n[[tile]]\nid = 1\nframes = [2]\n",
            "the same tile twice",
        ),
    ] {
        match settings(body) {
            Err(MetaError::BadTile(message)) => {
                assert!(!message.is_empty(), "{why}: the refusal says nothing")
            }
            other => panic!("{why} should be refused, got {other:?}"),
        }
    }
}

#[test]
fn milliseconds_become_ticks_at_import() {
    // The artist's timing is in milliseconds and the simulation counts ticks.
    // Converting on load would make a ripple's speed depend on whatever tick
    // rate that session happened to be configured with.
    let baked = Animations::bake(&declared(), 100, 60);
    let cycle = baked.cycle(5).expect("tile 5 animates");
    assert_eq!(cycle.ticks, 14, "240ms at 60Hz, rounded");
    assert_eq!(cycle.frames, vec![5, 33, 61, 89]);
    assert_eq!(cycle.duration_ticks(), 56);

    let faster = Animations::bake(&declared(), 100, 120);
    assert_eq!(
        faster.cycle(5).unwrap().ticks,
        29,
        "the same 240ms is more ticks at 120Hz"
    );
}

#[test]
fn a_block_without_its_own_timing_takes_the_sheets() {
    let baked = Animations::bake(
        &[TileAnimation {
            id: 7,
            frames: vec![7, 8],
            frame_ms: None,
        }],
        250,
        60,
    );
    assert_eq!(baked.cycle(7).unwrap().ticks, 15, "250ms at 60Hz");
}

#[test]
fn a_frame_is_never_held_for_zero_ticks() {
    // A zero-length frame advances infinitely fast, and the modulus below
    // would divide by zero. `ms_to_ticks` already clamps; this is the test
    // that says so out loud for tiles too.
    let baked = Animations::bake(
        &[TileAnimation {
            id: 1,
            frames: vec![1, 2],
            frame_ms: Some(1),
        }],
        100,
        60,
    );
    assert_eq!(baked.cycle(1).unwrap().ticks, 1);
}

#[test]
fn the_cycle_is_a_function_of_the_tick_and_nothing_else() {
    let baked = Animations::bake(&declared(), 100, 60);
    // Fourteen ticks a frame, four frames, so the whole cycle is 56 ticks.
    for (tick, expected) in [
        (0, 5),
        (13, 5),
        (14, 33),
        (27, 33),
        (28, 61),
        (42, 89),
        (55, 89),
        (56, 5),
        (56 + 14, 33),
    ] {
        assert_eq!(
            baked.frame_at(5, Tick(tick)),
            expected,
            "tile 5 at tick {tick}"
        );
    }
}

#[test]
fn a_tile_with_no_cycle_answers_with_itself() {
    // So the renderer can ask unconditionally, the same bargain the sheet
    // makes by reporting one frame for a still.
    let baked = Animations::bake(&declared(), 100, 60);
    for tick in [0u64, 1, 14, 1_000] {
        assert_eq!(baked.frame_at(4, Tick(tick)), 4);
        assert_eq!(baked.frame_at(33, Tick(tick)), 33, "a frame is not a tile");
    }
    assert_eq!(Animations::default().frame_at(9, Tick(12_345)), 9);
}

#[test]
fn a_run_long_enough_to_overflow_a_tick_counter_still_lands_in_range() {
    // The modulus is taken before the index for this reason. A u64 of ticks is
    // more than any run, but the arithmetic should not be the thing that says
    // so.
    let baked = Animations::bake(&declared(), 100, 60);
    for tick in [u64::MAX, u64::MAX - 1, u32::MAX as u64 + 1] {
        let drawn = baked.frame_at(5, Tick(tick));
        assert!([5, 33, 61, 89].contains(&drawn), "tick {tick} drew {drawn}");
    }
}
