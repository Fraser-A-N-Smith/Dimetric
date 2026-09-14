//! The window, driven headlessly.
//!
//! Compiling is not evidence that a GUI works. These drive the real widgets
//! through `egui_kittest` — clicking the actual buttons, typing into the actual
//! fields — and assert what reached the project underneath.
//!
//! What they are checking is the same contract the rest of the editor keeps:
//! the client holds no state of its own, so a click has to come out the far end
//! as a command against the scene.

#![cfg(feature = "gui")]

use dimetric_core::NodeUid;
use dimetric_editor::panels::{inspector_rows, tree_rows};
use dimetric_editor::{Action, Editor, Mode};
use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

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
"##;

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "dimetric-shell-{name}-{}",
            std::process::id() as u64 * 67 + name.len() as u64
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("arena01.dim"), ARENA).expect("scene");
        Fixture(dir)
    }

    fn editor(&self) -> Editor {
        Editor::open(&self.0, "arena01", 13).expect("the scene opens")
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

/// A harness over a panel drawn from the editor's own models.
///
/// The widget code is the shell's; what it draws comes from `dimetric-editor`,
/// which is the seam being tested.
fn tree_harness(editor: Editor) -> Harness<'static, Editor> {
    Harness::new_ui_state(
        |ui, editor: &mut Editor| {
            let mut clicked = None;
            let mut folded = None;
            for row in tree_rows(editor) {
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 12.0);
                    if row.has_children {
                        let glyph = if row.folded { "unfold" } else { "fold" };
                        if ui.button(glyph).clicked() {
                            folded = Some(row.uid);
                        }
                    }
                    if ui.selectable_label(row.selected, &row.name).clicked() {
                        clicked = Some(row.uid);
                    }
                });
            }
            if let Some(uid) = clicked {
                editor.dispatch(Action::Select(vec![uid]));
            }
            if let Some(uid) = folded {
                editor.dispatch(Action::ToggleFold(uid));
            }
        },
        editor,
    )
}

#[test]
fn clicking_a_node_in_the_tree_selects_it() {
    let fixture = Fixture::new("select");
    let mut harness = tree_harness(fixture.editor());
    harness.run();

    harness.get_by_label("Brazier").click();
    harness.run();

    assert_eq!(harness.state().selected(), Some(uid("n_brazier1")));
}

#[test]
fn folding_from_the_tree_hides_the_children_it_draws_next_frame() {
    let fixture = Fixture::new("fold");
    let mut harness = tree_harness(fixture.editor());
    harness.run();
    assert!(harness.query_by_label("Brazier").is_some());

    harness.get_by_label("fold").click();
    harness.run();

    assert!(
        harness.query_by_label("Brazier").is_none(),
        "the folded node's child is gone from what the tree draws"
    );
}

#[test]
fn typing_into_an_inspector_field_reaches_the_scene_as_a_command() {
    // The whole contract in one test: a keystroke in a widget becomes a command
    // on the bus, and the file changes because of it.
    let fixture = Fixture::new("inspector");
    let mut editor = fixture.editor();
    editor.dispatch(Action::Select(vec![uid("n_brazier1")]));
    let before = editor.scene_text();

    let mut harness = Harness::new_ui_state(
        |ui, editor: &mut Editor| {
            let node = editor.selected().expect("something is selected");
            let mut edits = Vec::new();
            for row in inspector_rows(editor, node) {
                let mut text = row.literal.clone();
                ui.horizontal(|ui| {
                    ui.label(&row.key);
                    if ui.text_edit_singleline(&mut text).changed() && text != row.literal {
                        edits.push((row.key.clone(), text.clone()));
                    }
                });
            }
            for (key, literal) in edits {
                editor.dispatch(Action::SetProperty {
                    node,
                    key,
                    literal: Some(literal),
                });
            }
        },
        editor,
    );
    harness.run();

    // By role, not by label: the label beside the field is a widget of its own,
    // and typing at it goes nowhere.
    let fields = harness.get_all_by_role(egui::accesskit::Role::TextInput);
    let field = fields.into_iter().nth(4).expect("the z field");
    field.focus();
    field.type_text("5");
    harness.run();

    let after = harness.state().scene_text();
    assert_ne!(after, before, "the keystroke did not reach the scene");
    assert!(
        after.contains("z = 105") || after.contains("z = 5"),
        "{after}"
    );
}

#[test]
fn the_toolbar_drives_playback_without_touching_the_scene() {
    let fixture = Fixture::new("toolbar");
    let editor = fixture.editor();
    let before = editor.scene_text();

    let mut harness = Harness::new_ui_state(
        |ui, editor: &mut Editor| {
            let mut action = None;
            ui.horizontal(|ui| {
                if ui.button("Play").clicked() {
                    action = Some(Action::Play);
                }
                if ui.button("Step").clicked() {
                    action = Some(Action::StepTick);
                }
                if ui.button("Stop").clicked() {
                    action = Some(Action::Stop);
                }
            });
            ui.label(format!("tick {}", editor.playback.tick()));
            if let Some(action) = action {
                editor.dispatch(action);
            }
        },
        editor,
    );
    harness.run();

    harness.get_by_label("Play").click();
    harness.run();
    assert_eq!(harness.state().playback.mode(), Mode::Playing);

    for _ in 0..3 {
        harness.get_by_label("Step").click();
        harness.run();
    }
    assert_eq!(harness.state().playback.tick(), 3);
    assert!(
        harness.query_by_label("tick 3").is_some(),
        "and it is shown"
    );

    harness.get_by_label("Stop").click();
    harness.run();
    assert_eq!(harness.state().playback.mode(), Mode::Editing);
    assert_eq!(
        harness.state().scene_text(),
        before,
        "playing wrote nothing"
    );
}
