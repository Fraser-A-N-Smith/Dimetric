//! The two rules M7 is graded on.
//!
//! One: every editor mutation goes through the command bus. Two: opening and
//! closing a scene produces zero diff in the `.dim`.
//!
//! Both are checked by driving a scripted editing session with no window open
//! at all, which is the whole reason the editor's logic does not live inside a
//! paint callback.

use std::path::{Path, PathBuf};

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_editor::{Action, Editor};

const ARENA: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

# ── Environment ───────────────────────────────────

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena01"

[[node]]
id = "n_floor001"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"          # /Arena01
tileset = "asset:tilesets/dungeon"

[[node]]
id = "n_brazier1"
kind = "Sprite2D"
name = "Brazier"
parent = "n_root0000"          # /Arena01
pos = [96.0, 48.0]
texture = "asset:sprites/props/brazier"
z = 10

[[node]]
id = "n_glow0001"
kind = "Light2D"
name = "Glow"
parent = "n_brazier1"          # /Arena01/Brazier
color = "#ffb347e0"
radius = 72.0
"##;

const SKELETON: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_skroot01"

[[node]]
id = "n_skroot01"
kind = "Node2D"
name = "Skeleton"

[[node]]
id = "n_skbody01"
kind = "Sprite2D"
name = "Body"
parent = "n_skroot01"
texture = "asset:sprites/skeleton"
"##;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "dimetric-editor-{name}-{}",
            std::process::id() as u64 * 41 + name.len() as u64
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prefabs")).expect("temp dir");
        std::fs::write(dir.join("arena01.dim"), ARENA).expect("scene");
        std::fs::write(dir.join("prefabs/skeleton.dim"), SKELETON).expect("prefab");
        Fixture(dir)
    }

    fn open(&self) -> Editor {
        Editor::open(&self.0, "arena01", 7).expect("the scene opens")
    }

    fn scene_text(&self) -> String {
        std::fs::read_to_string(self.0.join("arena01.dim")).expect("scene")
    }

    fn path(&self) -> &Path {
        &self.0
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

/// Every action a user might take that does not edit the scene.
fn view_actions() -> Vec<Action> {
    vec![
        Action::Select(vec![uid("n_brazier1")]),
        Action::ToggleSelect(uid("n_glow0001")),
        Action::ToggleSelect(uid("n_glow0001")),
        Action::ToggleFold(uid("n_brazier1")),
        Action::ToggleFold(uid("n_root0000")),
        Action::LookAt(Vec2Fx::from_ints(64, 48)),
        Action::Zoom("2.0".to_string()),
        Action::SelectNone,
        Action::Play,
        Action::StepTick,
        Action::Pause,
        Action::ScrubTo(0),
        Action::Stop,
    ]
}

/// A scripted editing session: everything a designer might do in a sitting.
fn editing_actions() -> Vec<Action> {
    vec![
        Action::Select(vec![uid("n_brazier1")]),
        Action::Rename {
            node: uid("n_brazier1"),
            name: "Torch".to_string(),
        },
        Action::SetProperty {
            node: uid("n_brazier1"),
            key: "z".to_string(),
            literal: Some("20".to_string()),
        },
        Action::Move {
            node: uid("n_brazier1"),
            to: Vec2Fx::from_ints(32, 16),
        },
        Action::Create {
            kind: "Sprite2D".to_string(),
            name: "Crate".to_string(),
            parent: Some(uid("n_root0000")),
        },
        Action::Instance {
            source: "prefabs/skeleton".to_string(),
            name: "Guard".to_string(),
            parent: uid("n_root0000"),
        },
        Action::Reparent {
            node: uid("n_glow0001"),
            parent: uid("n_root0000"),
        },
        Action::SetProperty {
            node: uid("n_glow0001"),
            key: "radius".to_string(),
            literal: Some("96.0".to_string()),
        },
    ]
}

#[test]
fn opening_and_closing_a_scene_produces_zero_diff() {
    // The rule the sidecar exists for. Looking at a scene is not editing it.
    let fixture = Fixture::new("zero-diff");
    let before = fixture.scene_text();

    let mut editor = fixture.open();
    for action in view_actions() {
        let outcome = editor.dispatch(action.clone());
        assert!(
            !outcome.rejected(),
            "{action:?} was rejected: {}",
            outcome.diagnostics
        );
    }
    editor.dispatch(Action::Save);

    assert_eq!(fixture.scene_text(), before, "the scene changed");
}

#[test]
fn nothing_that_is_not_an_edit_produces_a_command() {
    let fixture = Fixture::new("no-commands");
    let mut editor = fixture.open();
    for action in view_actions() {
        let outcome = editor.dispatch(action.clone());
        assert!(
            outcome.commands.is_empty(),
            "{action:?} produced {:?}",
            outcome.commands
        );
    }
}

#[test]
fn every_edit_goes_through_the_command_bus() {
    // The other half: an action that changed the file must have produced a
    // command. A widget that wrote to the scene directly would fail here, which
    // is the point — the rule is checkable rather than aspirational.
    let fixture = Fixture::new("through-the-bus");
    let mut editor = fixture.open();

    for action in editing_actions() {
        let before = editor.scene_text();
        let outcome = editor.dispatch(action.clone());
        assert!(
            !outcome.rejected(),
            "{action:?} was rejected: {}",
            outcome.diagnostics
        );
        let after = editor.scene_text();

        if after != before {
            assert!(
                !outcome.commands.is_empty(),
                "{action:?} changed the scene without producing a command"
            );
        }
        assert_eq!(
            action.edits_scene(),
            after != before,
            "{action:?} disagrees with `edits_scene`"
        );
    }
}

#[test]
fn a_whole_editing_session_undoes_back_to_the_file_it_started_from() {
    // Because the bus is the only mutation path, undo cannot drift. If any
    // action had gone around it, this is where the drift would show.
    let fixture = Fixture::new("undo-all");
    let mut editor = fixture.open();
    let before = editor.scene_text();

    let mut applied = 0;
    for action in editing_actions() {
        let outcome = editor.dispatch(action);
        assert!(!outcome.rejected(), "{}", outcome.diagnostics);
        applied += outcome.commands.len();
    }
    assert!(applied > 0);
    assert_ne!(editor.scene_text(), before, "the session did nothing");

    for _ in 0..applied {
        let outcome = editor.dispatch(Action::Undo);
        assert!(!outcome.rejected(), "{}", outcome.diagnostics);
    }
    assert_eq!(
        editor.scene_text(),
        before,
        "undo did not go all the way back"
    );
}

#[test]
fn a_saved_session_is_what_the_editor_had_in_memory() {
    let fixture = Fixture::new("save");
    let mut editor = fixture.open();
    for action in editing_actions() {
        editor.dispatch(action);
    }
    let in_memory = editor.scene_text();
    editor.dispatch(Action::Save);
    assert_eq!(fixture.scene_text(), in_memory);
}

#[test]
fn the_sidecar_carries_the_view_state_and_the_scene_does_not() {
    let fixture = Fixture::new("sidecar");
    let mut editor = fixture.open();
    editor.dispatch(Action::Select(vec![uid("n_brazier1")]));
    editor.dispatch(Action::ToggleFold(uid("n_root0000")));
    editor.dispatch(Action::LookAt(Vec2Fx::from_ints(64, 48)));
    editor.dispatch(Action::Save);

    let sidecar = std::fs::read_to_string(fixture.path().join("arena01.dim.editor"))
        .expect("the sidecar is written beside the scene");
    assert!(sidecar.contains("n_brazier1"), "{sidecar}");
    assert!(sidecar.contains("64.0"), "{sidecar}");
    assert!(
        !fixture.scene_text().contains("n_brazier1\"]"),
        "not in the scene"
    );

    // And reopening restores it.
    let reopened = fixture.open();
    assert_eq!(reopened.selected(), Some(uid("n_brazier1")));
    assert!(reopened.sidecar.folded.contains(&uid("n_root0000")));
}

#[test]
fn a_bad_value_typed_into_the_inspector_is_a_diagnostic_rather_than_a_panic() {
    let fixture = Fixture::new("bad-value");
    let mut editor = fixture.open();
    let before = editor.scene_text();

    let outcome = editor.dispatch(Action::SetProperty {
        node: uid("n_glow0001"),
        key: "radius".to_string(),
        literal: Some("the colour blue".to_string()),
    });
    assert!(outcome.rejected());
    assert!(outcome.commands.is_empty());
    assert_eq!(editor.scene_text(), before);
    assert_eq!(editor.console.errors(), 1, "and it is in the console");
}

#[test]
fn a_value_that_is_not_exactly_representable_is_refused_here_too() {
    // The same rule the loader applies. An inspector that quietly rounded would
    // be a second, looser way into the scene.
    let fixture = Fixture::new("inexact");
    let mut editor = fixture.open();
    let outcome = editor.dispatch(Action::SetProperty {
        node: uid("n_glow0001"),
        key: "radius".to_string(),
        literal: Some("0.1".to_string()),
    });
    assert!(outcome.rejected());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == dimetric_core::Code::NOT_REPRESENTABLE));
}
