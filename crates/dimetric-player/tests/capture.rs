//! A Controls page learns which key was pressed.
//!
//! G53 built the half that binds. `input.*` speaks actions, though: a key bound
//! to nothing never reaches a script, and a key bound to something arrives as
//! that something — so there was no moment at which a script could learn "the
//! player pressed F". The page had no name to put in `input.bind`, none to show
//! in its row, and none to save for the next launch.
//!
//! Cycling each row through a list of key names with left and right is a
//! Controls page no one would ship, and preset layouts are a different feature
//! that would stand in for this one.

use dimetric_player::bindings::{
    read_capture_event, settle_capture, Captured, CANCEL_CAPTURE_KEY, CAPTURE,
};
use dimetric_player::{Action, Bindings};
use dimetric_scene::Value;
use dimetric_sim::event::GameEvent;

fn event(kind: &str, payload: Value) -> GameEvent {
    GameEvent {
        tick: dimetric_core::Tick(0),
        kind: kind.to_string(),
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

fn asking(action: &str) -> GameEvent {
    event(CAPTURE, map(&[("action", Value::Str(action.into()))]))
}

#[test]
fn a_page_asks_for_the_next_key_and_gets_its_name() {
    let mut bindings = Bindings::wasd();
    let capture = read_capture_event(&[], &asking("fire"))
        .expect("this is a capture event")
        .expect("and a readable one");
    assert_eq!(capture.action, Action::Fire);
    assert_eq!(capture.name, "fire");

    assert_eq!(
        settle_capture(&mut bindings, &capture, "KeyF"),
        Captured::Bound("KeyF".into())
    );
    assert_eq!(bindings.action("KeyF"), Some(Action::Fire));
    assert_eq!(
        bindings.action("Space"),
        None,
        "the key it had is taken, as a Controls page row implies"
    );
    assert_eq!(bindings.keys_for(Action::Fire), ["KeyF"]);
}

#[test]
fn a_pad_button_captures_exactly_as_a_key_does() {
    let mut bindings = Bindings::wasd();
    let capture = read_capture_event(&[], &asking("fire")).unwrap().unwrap();
    assert_eq!(
        settle_capture(&mut bindings, &capture, "PadEast"),
        Captured::Bound("PadEast".into())
    );
    assert_eq!(bindings.action("PadEast"), Some(Action::Fire));
    assert_eq!(
        bindings.action("PadSouth"),
        None,
        "South confirmed before and does not now"
    );
}

#[test]
fn escape_cancels_and_binds_nothing() {
    let mut bindings = Bindings::wasd();
    let before = bindings.keys_for(Action::Fire).join(" ");
    let capture = read_capture_event(&[], &asking("fire")).unwrap().unwrap();
    assert_eq!(
        settle_capture(&mut bindings, &capture, CANCEL_CAPTURE_KEY),
        Captured::Cancelled
    );
    assert_eq!(
        bindings.keys_for(Action::Fire).join(" "),
        before,
        "a cancelled capture leaves everything as it was"
    );
    assert_eq!(
        bindings.action(CANCEL_CAPTURE_KEY),
        Some(Action::Pause),
        "and Escape keeps doing whatever it did"
    );
}

#[test]
fn a_declared_action_can_be_captured_for() {
    let actions = vec!["undo".to_string()];
    let mut bindings = Bindings::wasd();
    let capture = read_capture_event(&actions, &asking("undo"))
        .unwrap()
        .unwrap();
    assert_eq!(capture.action, Action::Custom(0));
    settle_capture(&mut bindings, &capture, "KeyZ");
    assert_eq!(bindings.action("KeyZ"), Some(Action::Custom(0)));
}

#[test]
fn an_event_of_another_kind_is_not_a_capture() {
    assert!(read_capture_event(&[], &event("input.bind", map(&[]))).is_none());
    assert!(read_capture_event(&[], &event("audio.bus_volume", map(&[]))).is_none());
}

#[test]
fn a_payload_this_build_cannot_read_is_reported() {
    for (payload, why) in [
        (Value::Str("fire".into()), "not a table"),
        (map(&[]), "no action"),
        (
            map(&[("action", Value::Str("fier".into()))]),
            "a typo in the action",
        ),
    ] {
        let d = read_capture_event(&[], &event(CAPTURE, payload))
            .unwrap_or_else(|| panic!("{why}: it is still a capture event"))
            .expect_err(why);
        assert_eq!(d.code, dimetric_core::Code::BINDING_UNKNOWN, "{why}");
        assert_eq!(d.severity, dimetric_core::Severity::Warning, "{why}");
        assert!(
            d.message.contains("nothing is being captured"),
            "{why}: {}",
            d.message
        );
    }
}

#[test]
fn capturing_one_action_leaves_the_rest_alone() {
    let mut bindings = Bindings::wasd();
    let capture = read_capture_event(&[], &asking("dash")).unwrap().unwrap();
    settle_capture(&mut bindings, &capture, "KeyQ");
    assert_eq!(bindings.action("KeyW"), Some(Action::Up));
    assert_eq!(bindings.action("Space"), Some(Action::Fire));
    assert_eq!(bindings.action("PadSouth"), Some(Action::Fire));
    assert_eq!(bindings.keys_for(Action::Dash), ["KeyQ"]);
}

#[test]
fn a_captured_key_is_a_single_key_not_an_addition() {
    // A page asking to capture for an action is asking "what one key should
    // this be", because that is the gesture: press the key you want. A row
    // showing two keys that meant three would be the same lie `input.bind`
    // avoids by replacing the set.
    let mut bindings = Bindings::wasd();
    assert_eq!(
        bindings.keys_for(Action::Up),
        ["KeyW", "ArrowUp", "PadUp"],
        "the key, the arrow and the pad's own"
    );
    let capture = read_capture_event(&[], &asking("up")).unwrap().unwrap();
    settle_capture(&mut bindings, &capture, "KeyI");
    assert_eq!(bindings.keys_for(Action::Up), ["KeyI"]);
}
