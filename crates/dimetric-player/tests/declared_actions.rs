//! A project can declare an input of its own.
//!
//! The engine has nine actions — four directions, `fire`, `alt`, `dash`, `use`,
//! `pause` — and a tactics game on an isometric board wants an **Undo** for a
//! move made this turn, because a misclick otherwise costs a run. The game uses
//! all nine. An undo could be a button on the screen and not a key, and not a
//! pad button.
//!
//! Reusing an action for two meanings makes one key do two things depending on
//! a mode, which is the opposite of what a binding is for, and still leaves a
//! pad player without the verb.
//!
//! The bit a declared action takes is its **position** in the declared list, so
//! that position is part of what a recording means. These tests are mostly
//! about that: appending is safe, reordering is refused rather than replayed.

use dimetric_host::Project;
use dimetric_player::{Action, Bindings};
use dimetric_sim::input::{action_button, buttons, InputLog};

/// A project declaring `actions` and binding them.
fn project(input: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("project.toml"), input).expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    dir
}

const UNDO: &str = r#"
[input]
actions = ["undo", "screens"]
up = ["KeyW"]
down = ["KeyS"]
left = ["KeyA"]
right = ["KeyD"]
fire = ["Space"]
undo = ["KeyZ", "Backspace"]
screens = ["Tab"]
"#;

#[test]
fn a_project_can_name_a_verb_the_engine_does_not_have() {
    let dir = project(UNDO);
    let (declared, problems) = Project::open(dir.path(), 0).declared_actions();
    assert_eq!(declared, ["undo", "screens"]);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_declared_action_is_bindable_like_a_built_in() {
    let dir = project(UNDO);
    let project = Project::open(dir.path(), 0);
    let (declared, _) = project.declared_actions();
    let (bindings, problems) = Bindings::from_declared_with(
        &project.settings.bindings,
        project.settings.bindings_declared,
        &declared,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(bindings.action("KeyZ"), Some(Action::Custom(0)));
    assert_eq!(bindings.action("Backspace"), Some(Action::Custom(0)));
    assert_eq!(bindings.action("Tab"), Some(Action::Custom(1)));
    assert_eq!(bindings.action("Space"), Some(Action::Fire));
}

#[test]
fn a_declared_action_sets_a_bit_of_its_own() {
    // Past the engine's five, and distinct from each other.
    assert_eq!(Action::Custom(0).button(), Some(1 << buttons::CUSTOM_FIRST));
    assert_eq!(
        Action::Custom(1).button(),
        Some(1 << (buttons::CUSTOM_FIRST + 1))
    );
    for built_in in [
        buttons::FIRE,
        buttons::ALT,
        buttons::DASH,
        buttons::USE,
        buttons::PAUSE,
    ] {
        assert_eq!(
            built_in & Action::Custom(0).button().unwrap(),
            0,
            "a declared action must not share a bit with a built-in"
        );
    }
}

#[test]
fn the_runtime_and_a_script_agree_about_which_bit_is_which() {
    // The two halves of the mapping: the runtime turns a key into a bit and a
    // script turns a name into one. If they disagreed, `input.pressed("undo")`
    // would read the wrong button and nothing would say so.
    let declared = vec!["undo".to_string(), "screens".to_string()];
    for (index, name) in declared.iter().enumerate() {
        assert_eq!(
            Action::Custom(index).button(),
            action_button(name, &declared),
            "{name}"
        );
    }
}

#[test]
fn a_built_in_wins_over_a_declaration_of_the_same_name() {
    // A script asking for `fire` must get the engine's bit whatever a project
    // wrote, so declaring one of their names is refused rather than shadowing.
    let dir = project("[input]\nactions = [\"fire\"]\nfire = [\"Space\"]\n");
    let (declared, problems) = Project::open(dir.path(), 0).declared_actions();
    assert!(declared.is_empty(), "the declaration was dropped");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, dimetric_core::Code::BINDING_UNKNOWN);
    assert!(problems[0].message.contains("shadow"), "{}", problems[0]);
    assert_eq!(
        action_button("fire", &["fire".to_string()]),
        Some(buttons::FIRE)
    );
}

#[test]
fn an_undeclared_name_is_still_an_error() {
    // The reason declaring has to be explicit. If an unrecognised action simply
    // became a new one, `fier = ["Space"]` would silently be a verb nothing
    // reads — which is the typo this diagnostic exists to catch.
    let dir = project("[input]\nfier = [\"Space\"]\n");
    let project = Project::open(dir.path(), 0);
    let (declared, _) = project.declared_actions();
    let (_, problems) = Bindings::from_declared_with(
        &project.settings.bindings,
        project.settings.bindings_declared,
        &declared,
    );
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, dimetric_core::Code::BINDING_UNKNOWN);
    assert!(problems[0].message.contains("fier"), "{}", problems[0]);
    assert!(
        problems[0].message.contains("[input] actions"),
        "it should say how to declare one: {}",
        problems[0]
    );
}

#[test]
fn a_duplicate_declaration_is_reported_rather_than_given_two_bits() {
    let dir = project("[input]\nactions = [\"undo\", \"undo\"]\nundo = [\"KeyZ\"]\n");
    let (declared, problems) = Project::open(dir.path(), 0).declared_actions();
    assert_eq!(declared, ["undo"]);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].message.contains("twice"), "{}", problems[0]);
}

#[test]
fn past_the_cap_is_a_diagnostic_rather_than_a_bit_outside_the_field() {
    let names: Vec<String> = (0..buttons::MAX_CUSTOM + 3)
        .map(|i| format!("\"verb{i}\""))
        .collect();
    let dir = project(&format!("[input]\nactions = [{}]\n", names.join(", ")));
    let (declared, problems) = Project::open(dir.path(), 0).declared_actions();
    assert_eq!(declared.len(), buttons::MAX_CUSTOM);
    assert_eq!(problems.len(), 3);
    assert!(problems[0].message.contains("bit left"), "{}", problems[0]);
    // And the last one that fits is still inside a u32.
    assert!(Action::Custom(buttons::MAX_CUSTOM - 1).button().is_some());
    assert_eq!(Action::Custom(buttons::MAX_CUSTOM).button(), None);
}

#[test]
fn a_recording_carries_the_actions_it_was_recorded_against() {
    let log = InputLog::new(7, "test", 1).with_actions(vec!["undo".into(), "screens".into()]);
    let text = log.to_text();
    assert!(text.contains("actions undo screens"), "{text}");
    let read = InputLog::parse(&text).expect("what we wrote parses");
    assert_eq!(read.actions, ["undo", "screens"]);
}

#[test]
fn a_log_with_no_declared_actions_is_byte_for_byte_what_it_always_was() {
    // Every log this engine has written until now. The header line is omitted
    // when the list is empty, so nothing already recorded changes.
    let text = InputLog::new(7, "test", 1).to_text();
    assert!(!text.contains("actions"), "{text}");
}

#[test]
fn an_older_log_replays_against_a_project_that_has_since_declared_one() {
    // The requirement stated plainly: a log recorded before the project
    // declared an action reads back with that action **not held**, because its
    // bit is zero and zero is what not-held means.
    let old = InputLog::new(7, "test", 1);
    assert!(old.actions_agree(&["undo".to_string()]));
    assert!(!old
        .frame(0)
        .player(0)
        .held(Action::Custom(0).button().unwrap()));
}

#[test]
fn appending_an_action_keeps_every_older_recording_readable() {
    let recorded = InputLog::new(7, "test", 1).with_actions(vec!["undo".into()]);
    assert!(
        recorded.actions_agree(&["undo".to_string(), "screens".to_string()]),
        "appending must not invalidate a recording"
    );
    assert_eq!(
        action_button("undo", &["undo".to_string(), "screens".to_string()]),
        action_button("undo", &["undo".to_string()]),
        "and `undo` keeps the bit it had"
    );
}

#[test]
fn reordering_an_action_is_refused_rather_than_replayed_wrongly() {
    // The hazard the list exists to catch. `undo` and `screens` swap bits, so
    // the recorded buttons would mean different verbs — a run that replays and
    // undoes where it opened a screen.
    let recorded = InputLog::new(7, "test", 1).with_actions(vec!["undo".into(), "screens".into()]);
    assert!(!recorded.actions_agree(&["screens".to_string(), "undo".to_string()]));
    assert!(!recorded.actions_agree(&["screens".to_string()]));
    assert!(recorded.actions_agree(&["undo".to_string(), "screens".to_string()]));
}
