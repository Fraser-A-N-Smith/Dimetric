//! The panel models: what each one shows, without a window open.

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_editor::panels::{
    asset_rows, gizmos, inspector_rows, tree_rows, viewport::pick, Viewport,
};
use dimetric_editor::{Action, Editor, Mode};

const ARENA: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena01"

[[node]]
id = "n_brazier1"
kind = "Sprite2D"
name = "Brazier"
parent = "n_root0000"
pos = [96.0, 48.0]
texture = "asset:sprites/props/brazier"
z = 10

[[node]]
id = "n_glow0001"
kind = "Light2D"
name = "Glow"
parent = "n_brazier1"
pos = [0.0, -8.0]
color = "#ffb347e0"
radius = 72.0
"##;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "dimetric-panels-{name}-{}",
            std::process::id() as u64 * 53 + name.len() as u64
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("arena01.dim"), ARENA).expect("scene");
        Fixture(dir)
    }

    fn open(&self) -> Editor {
        Editor::open(&self.0, "arena01", 11).expect("the scene opens")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uid(text: &str) -> NodeUid {
    NodeUid::parse(text).expect("a valid uid")
}

// -- scene tree ---------------------------------------------------------

#[test]
fn the_tree_is_depth_first_with_depths_a_client_can_indent_by() {
    let fixture = Fixture::new("tree");
    let editor = fixture.open();
    let rows = tree_rows(&editor);
    let shape: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
    assert_eq!(shape, [("Arena01", 0), ("Brazier", 1), ("Glow", 2)]);
}

#[test]
fn a_folded_node_hides_its_descendants_entirely() {
    // The client draws the list it is given rather than reimplementing folding.
    let fixture = Fixture::new("fold");
    let mut editor = fixture.open();
    editor.dispatch(Action::ToggleFold(uid("n_brazier1")));

    let rows = tree_rows(&editor);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Arena01", "Brazier"]);
    let brazier = rows.iter().find(|r| r.name == "Brazier").unwrap();
    assert!(
        brazier.folded && brazier.has_children,
        "so a twisty is drawn"
    );
}

#[test]
fn folding_a_leaf_hides_nothing() {
    let fixture = Fixture::new("fold-leaf");
    let mut editor = fixture.open();
    editor.dispatch(Action::ToggleFold(uid("n_glow0001")));
    assert_eq!(tree_rows(&editor).len(), 3);
}

#[test]
fn the_tree_marks_the_selection() {
    let fixture = Fixture::new("tree-select");
    let mut editor = fixture.open();
    editor.dispatch(Action::Select(vec![uid("n_glow0001")]));
    let rows = tree_rows(&editor);
    assert!(rows.iter().filter(|r| r.selected).count() == 1);
    assert!(rows.iter().find(|r| r.selected).unwrap().name == "Glow");
}

// -- inspector ----------------------------------------------------------

#[test]
fn the_inspector_shows_the_transform_then_the_kinds_own_properties() {
    let fixture = Fixture::new("inspector");
    let editor = fixture.open();
    let rows = inspector_rows(&editor, uid("n_brazier1"));
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(&keys[..6], ["pos", "rot", "scale", "visible", "z", "layer"]);
    assert!(keys.contains(&"texture"), "{keys:?}");
}

#[test]
fn the_inspector_is_driven_by_the_schema_rather_than_a_form_per_kind() {
    // Every property the kind declares appears, in the order it was declared.
    let fixture = Fixture::new("schema-driven");
    let editor = fixture.open();
    let registry = &editor.project.registry;
    let declared: Vec<&str> = registry
        .get("Light2D")
        .unwrap()
        .properties
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let rows = inspector_rows(&editor, uid("n_glow0001"));
    let shown: Vec<&str> = rows.iter().skip(6).map(|r| r.key.as_str()).collect();
    assert_eq!(shown, declared);
}

#[test]
fn a_property_at_its_default_says_so_and_one_the_file_spells_out_does_not() {
    let fixture = Fixture::new("defaults");
    let editor = fixture.open();
    let rows = inspector_rows(&editor, uid("n_brazier1"));
    let row = |key: &str| rows.iter().find(|r| r.key == key).unwrap();

    assert!(row("rot").is_default, "the file does not set it");
    assert!(!row("z").is_default, "the file sets it to 10");
    assert_eq!(row("z").literal, "10");
    assert_eq!(row("pos").literal, "[96.0, 48.0]");
}

#[test]
fn a_reference_comes_back_as_the_text_a_field_holds() {
    let fixture = Fixture::new("refs");
    let editor = fixture.open();
    let rows = inspector_rows(&editor, uid("n_brazier1"));
    let texture = rows.iter().find(|r| r.key == "texture").unwrap();
    assert_eq!(texture.literal, "asset:sprites/props/brazier");
    assert!(texture.required, "the schema requires a texture");
}

#[test]
fn what_the_inspector_shows_is_what_setting_it_back_accepts() {
    // The round trip that matters: a client reads a field, the user does not
    // touch it, and writing it back is a no-op rather than an error.
    let fixture = Fixture::new("round-trip");
    let mut editor = fixture.open();
    let before = editor.scene_text();

    for row in inspector_rows(&editor, uid("n_brazier1")) {
        if row.literal.is_empty() {
            continue;
        }
        let outcome = editor.dispatch(Action::SetProperty {
            node: uid("n_brazier1"),
            key: row.key.clone(),
            literal: Some(row.literal.clone()),
        });
        assert!(
            !outcome.rejected(),
            "writing back {}={:?} was rejected: {}",
            row.key,
            row.literal,
            outcome.diagnostics
        );
    }
    assert_eq!(editor.scene_text(), before, "writing back changed the file");
}

// -- viewport -----------------------------------------------------------

#[test]
fn a_gizmo_sits_where_the_node_is_in_the_world() {
    // World space, not local: the glow is eight above a brazier at (96, 48).
    let fixture = Fixture::new("gizmo");
    let editor = fixture.open();
    let handles = gizmos(&editor);
    let glow = handles
        .iter()
        .find(|g| g.node == uid("n_glow0001"))
        .unwrap();
    assert_eq!(glow.at, Vec2Fx::from_ints(96, 40));
}

#[test]
fn the_viewport_maps_between_the_screen_and_the_world_both_ways() {
    let fixture = Fixture::new("viewport");
    let mut editor = fixture.open();
    editor.dispatch(Action::LookAt(Vec2Fx::from_ints(96, 48)));
    editor.dispatch(Action::Zoom("2.0".to_string()));

    let viewport = Viewport::from_sidecar(&editor, (200, 100));
    // The camera centre lands in the middle of the drawing area.
    assert_eq!(viewport.to_screen(Vec2Fx::from_ints(96, 48)), (100.0, 50.0));
    // And doubling the zoom doubles the pixels per world unit.
    assert_eq!(
        viewport.to_screen(Vec2Fx::from_ints(106, 48)),
        (120.0, 50.0)
    );
    assert_eq!(viewport.to_world((120.0, 50.0)), Vec2Fx::from_ints(106, 48));
}

#[test]
fn a_zoom_of_zero_is_treated_as_unset_rather_than_collapsing_the_view() {
    let fixture = Fixture::new("zero-zoom");
    let mut editor = fixture.open();
    editor.dispatch(Action::Zoom("0.0".to_string()));
    let viewport = Viewport::from_sidecar(&editor, (200, 100));
    assert_eq!(viewport.zoom, dimetric_core::Fx::ONE);
}

#[test]
fn clicking_a_handle_picks_its_node_and_clicking_empty_space_picks_nothing() {
    let fixture = Fixture::new("pick");
    let mut editor = fixture.open();
    editor.dispatch(Action::LookAt(Vec2Fx::ZERO));
    let viewport = Viewport::from_sidecar(&editor, (256, 256));
    let handles = gizmos(&editor);

    let on_brazier = viewport.to_screen(Vec2Fx::from_ints(96, 48));
    assert_eq!(
        pick(&handles, &viewport, on_brazier),
        Some(uid("n_brazier1"))
    );
    assert_eq!(pick(&handles, &viewport, (5.0, 5.0)), None);
}

#[test]
fn dragging_a_gizmo_moves_the_node_through_the_bus() {
    let fixture = Fixture::new("drag");
    let mut editor = fixture.open();
    editor.dispatch(Action::LookAt(Vec2Fx::ZERO));
    let viewport = Viewport::from_sidecar(&editor, (256, 256));

    let dropped = viewport.to_world((160.0, 160.0));
    let outcome = editor.dispatch(Action::Move {
        node: uid("n_brazier1"),
        to: dropped,
    });
    assert_eq!(outcome.commands.len(), 1, "one command, through the bus");
    assert_eq!(
        dimetric_editor::world_position(&editor.project, uid("n_brazier1")),
        Some(dropped)
    );
}

// -- assets and console -------------------------------------------------

#[test]
fn the_asset_browser_lists_what_the_catalogue_holds() {
    let fixture = Fixture::new("assets");
    let mut editor = fixture.open();
    assert!(asset_rows(&editor).is_empty(), "nothing scanned yet");

    std::fs::create_dir_all(fixture.0.join("assets/sprites")).expect("dir");
    dimetric_assets::encode_png(
        &fixture.0.join("assets/sprites/hero.png"),
        &[255u8, 0, 0, 255].repeat(16),
        4,
        4,
    )
    .expect("png");
    editor.project.scan_assets();

    let rows = asset_rows(&editor);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "sprites/hero");
    assert_eq!(rows[0].kind, "png");
    assert!(rows[0].stale, "never imported");
}

#[test]
fn the_console_keeps_diagnostics_in_order_and_counts_the_errors() {
    let fixture = Fixture::new("console");
    let mut editor = fixture.open();
    assert_eq!(editor.console.errors(), 0);

    editor.dispatch(Action::SetProperty {
        node: uid("n_glow0001"),
        key: "radius".to_string(),
        literal: Some("not a number".to_string()),
    });
    editor.dispatch(Action::Rename {
        node: uid("n_glow0001"),
        name: "Halo".to_string(),
    });

    assert_eq!(editor.console.errors(), 1);
    assert!(!editor.console.is_empty());
}

// -- playback -----------------------------------------------------------

#[test]
fn playing_does_not_touch_the_scene_so_stopping_needs_no_cleanup() {
    let fixture = Fixture::new("play");
    let mut editor = fixture.open();
    let before = editor.scene_text();

    editor.dispatch(Action::Play);
    assert_eq!(editor.playback.mode(), Mode::Playing);
    for _ in 0..10 {
        editor.dispatch(Action::StepTick);
    }
    assert_eq!(editor.playback.tick(), 10);
    editor.dispatch(Action::Stop);

    assert_eq!(editor.playback.mode(), Mode::Editing);
    assert!(!editor.playback.is_running());
    assert_eq!(editor.scene_text(), before);
}

#[test]
fn the_scrubber_goes_backwards_by_restoring_a_snapshot_and_stepping() {
    let fixture = Fixture::new("scrub");
    let mut editor = fixture.open();
    editor.dispatch(Action::Play);
    for _ in 0..90 {
        editor.dispatch(Action::StepTick);
    }
    assert_eq!(editor.playback.furthest(), 90);

    editor.dispatch(Action::ScrubTo(30));
    assert_eq!(editor.playback.tick(), 30);
    // And scrubbing is inspection, so it leaves the editor paused.
    assert_eq!(editor.playback.mode(), Mode::Paused);
}

#[test]
fn scrubbing_to_a_tick_twice_lands_on_the_same_state() {
    // A tick is a pure function of the state before it, so any tick can be
    // reached by restoring a snapshot and stepping — the same thing rollback
    // netcode does, and the reason the determinism work came first.
    let fixture = Fixture::new("scrub-twice");
    let mut editor = fixture.open();
    editor.dispatch(Action::Play);
    for _ in 0..90 {
        editor.dispatch(Action::StepTick);
    }

    editor.dispatch(Action::ScrubTo(45));
    let first = editor.playback.sim().unwrap().hash();
    editor.dispatch(Action::ScrubTo(80));
    editor.dispatch(Action::ScrubTo(45));
    let second = editor.playback.sim().unwrap().hash();
    assert_eq!(first, second);
}

#[test]
fn the_scrubber_cannot_be_dragged_past_what_has_been_played() {
    let fixture = Fixture::new("scrub-clamp");
    let mut editor = fixture.open();
    editor.dispatch(Action::Play);
    for _ in 0..5 {
        editor.dispatch(Action::StepTick);
    }
    editor.dispatch(Action::ScrubTo(1000));
    assert_eq!(editor.playback.tick(), 5);
}

#[test]
fn pausing_and_playing_again_resumes_rather_than_restarting() {
    let fixture = Fixture::new("resume");
    let mut editor = fixture.open();
    editor.dispatch(Action::Play);
    for _ in 0..10 {
        editor.dispatch(Action::StepTick);
    }
    editor.dispatch(Action::Pause);
    editor.dispatch(Action::Play);
    assert_eq!(editor.playback.mode(), Mode::Playing);
    assert_eq!(editor.playback.tick(), 10, "it did not start over");
}
