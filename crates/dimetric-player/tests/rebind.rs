//! Bindings change while the game runs, and a pad button is as bindable as a
//! key.
//!
//! Keys were read once from `project.toml` and nothing a script could do
//! reached them; pad buttons were a constant table — `South` *was* fire — not
//! bindable at all. So a Controls page could not exist, and a player who wanted
//! confirm and cancel the other way round, which is the other half of the
//! world's convention, had no way to say so.
//!
//! Remapping in script would be the wrong answer: reading `fire` and deciding
//! it meant `alt` puts the player's preference into the simulation's *reading*
//! of the input, so the same recording would replay differently under another
//! profile. Bindings belong to the host, before the input frame is built — and
//! because a recording stores actions, a session played under any bindings
//! replays identically under any other.

use dimetric_host::Project;
use dimetric_player::bindings::{apply_event, is_pad_button, BIND, PAD_BUTTONS};
use dimetric_player::{Action, Bindings};
use dimetric_scene::Value;
use dimetric_sim::event::GameEvent;

fn bind_event(payload: Value) -> GameEvent {
    GameEvent {
        tick: dimetric_core::Tick(0),
        kind: BIND.to_string(),
        payload,
    }
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn keys(names: &[&str]) -> Value {
    Value::List(names.iter().map(|n| Value::Str(n.to_string())).collect())
}

#[test]
fn a_game_can_rebind_an_action_while_it_runs() {
    let mut bindings = Bindings::wasd();
    assert_eq!(bindings.action("Space"), Some(Action::Fire));
    assert_eq!(bindings.action("KeyF"), None);

    let problem = apply_event(
        &mut bindings,
        &[],
        &bind_event(map(&[
            ("action", Value::Str("fire".into())),
            ("keys", keys(&["KeyF"])),
        ])),
    );
    assert!(problem.is_none(), "{problem:?}");
    assert_eq!(bindings.action("KeyF"), Some(Action::Fire));
    assert_eq!(
        bindings.action("Space"),
        None,
        "the set is replaced, not added to: a Controls page showing one key \
         has to mean one key"
    );
}

#[test]
fn rebinding_leaves_every_other_action_alone() {
    let mut bindings = Bindings::wasd();
    apply_event(
        &mut bindings,
        &[],
        &bind_event(map(&[
            ("action", Value::Str("fire".into())),
            ("keys", keys(&["KeyF"])),
        ])),
    );
    assert_eq!(bindings.action("KeyW"), Some(Action::Up));
    assert_eq!(bindings.action("ShiftLeft"), Some(Action::Dash));
    assert_eq!(bindings.action("Escape"), Some(Action::Pause));
}

#[test]
fn a_pad_button_rebinds_exactly_as_a_key_does() {
    // Swap South and East, which is the other half of the world's convention.
    let mut bindings = Bindings::wasd();
    assert_eq!(bindings.action("PadSouth"), Some(Action::Fire));
    assert_eq!(bindings.action("PadEast"), Some(Action::Dash));

    for (action, buttons) in [("fire", &["PadEast"][..]), ("dash", &["PadSouth"][..])] {
        let problem = apply_event(
            &mut bindings,
            &[],
            &bind_event(map(&[
                ("action", Value::Str(action.into())),
                ("keys", keys(buttons)),
            ])),
        );
        assert!(problem.is_none(), "{problem:?}");
    }
    assert_eq!(bindings.action("PadEast"), Some(Action::Fire));
    assert_eq!(bindings.action("PadSouth"), Some(Action::Dash));
}

#[test]
fn a_declared_action_is_rebindable_too() {
    let mut bindings = Bindings::wasd();
    let actions = vec!["undo".to_string()];
    let problem = apply_event(
        &mut bindings,
        &actions,
        &bind_event(map(&[
            ("action", Value::Str("undo".into())),
            ("keys", keys(&["KeyZ", "PadLB"])),
        ])),
    );
    assert!(problem.is_none(), "{problem:?}");
    assert_eq!(bindings.action("KeyZ"), Some(Action::Custom(0)));
    assert_eq!(bindings.action("PadLB"), Some(Action::Custom(0)));
}

#[test]
fn a_controls_page_can_read_back_what_it_set() {
    let mut bindings = Bindings::wasd();
    apply_event(
        &mut bindings,
        &[],
        &bind_event(map(&[
            ("action", Value::Str("fire".into())),
            ("keys", keys(&["KeyF", "PadRB"])),
        ])),
    );
    assert_eq!(bindings.keys_for(Action::Fire), ["KeyF", "PadRB"]);
}

#[test]
fn an_action_can_be_bound_to_nothing() {
    // A player clearing a row. It has to be allowed, and it has to leave the
    // key doing nothing rather than doing what it did before.
    let mut bindings = Bindings::wasd();
    apply_event(
        &mut bindings,
        &[],
        &bind_event(map(&[
            ("action", Value::Str("dash".into())),
            ("keys", Value::List(Vec::new())),
        ])),
    );
    assert_eq!(bindings.action("ShiftLeft"), None);
    assert!(bindings.keys_for(Action::Dash).is_empty());
}

#[test]
fn a_payload_this_build_cannot_read_is_reported_and_changes_nothing() {
    let before = Bindings::wasd();
    for (payload, why) in [
        (Value::Str("fire".into()), "not a table"),
        (map(&[("keys", keys(&["KeyF"]))]), "no action"),
        (map(&[("action", Value::Str("fire".into()))]), "no keys"),
        (
            map(&[
                ("action", Value::Str("fier".into())),
                ("keys", keys(&["KeyF"])),
            ]),
            "a typo in the action",
        ),
        (
            map(&[
                ("action", Value::Str("fire".into())),
                ("keys", Value::List(vec![Value::Int(7)])),
            ]),
            "a key that is not a string",
        ),
    ] {
        let mut bindings = Bindings::wasd();
        let problem = apply_event(&mut bindings, &[], &bind_event(payload));
        let d = problem.unwrap_or_else(|| panic!("{why} should be refused"));
        assert_eq!(d.code, dimetric_core::Code::BINDING_UNKNOWN, "{why}");
        assert_eq!(d.severity, dimetric_core::Severity::Warning, "{why}");
        assert_eq!(
            bindings.keys_for(Action::Fire),
            before.keys_for(Action::Fire),
            "{why}: the bindings must be untouched"
        );
    }
}

#[test]
fn an_event_of_another_kind_is_left_alone() {
    let mut bindings = Bindings::wasd();
    let event = GameEvent {
        tick: dimetric_core::Tick(0),
        kind: "audio.bus_volume".to_string(),
        payload: map(&[("bus", Value::Str("Music".into()))]),
    };
    assert!(apply_event(&mut bindings, &[], &event).is_none());
    assert_eq!(bindings.action("Space"), Some(Action::Fire));
}

#[test]
fn every_pad_button_name_is_bindable() {
    // The names are the binding vocabulary, so every one of them has to be a
    // name `Bindings` will take. A name in the list that nothing accepts would
    // be a Controls page with a dead row.
    for button in PAD_BUTTONS {
        assert!(is_pad_button(button), "{button}");
        let mut bindings = Bindings::wasd();
        apply_event(
            &mut bindings,
            &[],
            &bind_event(map(&[
                ("action", Value::Str("fire".into())),
                ("keys", keys(&[button])),
            ])),
        );
        assert_eq!(bindings.action(button), Some(Action::Fire), "{button}");
    }
}

#[test]
fn a_project_that_declares_keys_keeps_the_engines_pad() {
    // The regression this rule exists to prevent. Declaring `[input]` replaces
    // the defaults, and the pad used to be a separate constant nothing could
    // reach — so moving it into the table would silently take the controller
    // away from every project that has not written a pad layout yet.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("project.toml"),
        "[input]\nfire = [\"Space\"]\nup = [\"KeyW\"]\n",
    )
    .expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    let project = Project::open(dir.path(), 0);
    let (bindings, problems) = Bindings::from_declared(
        &project.settings.bindings,
        project.settings.bindings_declared,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(bindings.action("Space"), Some(Action::Fire));
    assert_eq!(
        bindings.action("PadSouth"),
        Some(Action::Fire),
        "a project that says nothing about the pad keeps the engine's layout"
    );
}

#[test]
fn a_project_that_names_a_pad_button_owns_the_whole_pad() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("project.toml"),
        "[input]\nfire = [\"Space\", \"PadEast\"]\n",
    )
    .expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n",
    )
    .expect("scene");
    let project = Project::open(dir.path(), 0);
    let (bindings, _) = Bindings::from_declared(
        &project.settings.bindings,
        project.settings.bindings_declared,
    );
    assert_eq!(bindings.action("PadEast"), Some(Action::Fire));
    assert_eq!(
        bindings.action("PadSouth"),
        None,
        "it said what the pad does, so the engine does not second-guess it"
    );
}
