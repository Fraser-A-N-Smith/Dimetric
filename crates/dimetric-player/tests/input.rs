//! The device boundary: everything a player touches becoming a `PlayerInput`.
//!
//! None of this needs a gamepad plugged in, which is the point. The hard part
//! of gamepad support is not talking to the device — it is that a stick never
//! stops moving, and turning that into something a replay can hold is
//! arithmetic that can be tested on any machine.

use dimetric_core::{Angle, Vec2Fx};
use dimetric_player::bindings::{quantize_stick, Action, Held, STICK_DEAD_ZONE};
use dimetric_player::pad::window_to_canvas;
use dimetric_scene::ui::Canvas;

#[test]
fn a_resting_stick_reads_as_exactly_centred() {
    // The one that matters most. A stick at rest reports small non-zero
    // numbers that differ every poll; without a dead zone every one of them
    // would be a new line in the input log and a new state hash.
    for (x, y) in [(0.0, 0.0), (0.01, -0.02), (0.1, 0.1), (-0.15, 0.05)] {
        assert_eq!(
            quantize_stick(x, y),
            Vec2Fx::ZERO,
            "({x}, {y}) should be inside the dead zone"
        );
    }
}

#[test]
fn a_stick_held_still_produces_the_same_value_every_poll() {
    // Jitter within a step has to quantise away, or holding a direction writes
    // a different input sixty times a second.
    let steady = quantize_stick(0.700, 0.0);
    for jitter in [-0.004, -0.002, 0.0, 0.002, 0.004] {
        assert_eq!(quantize_stick(0.700 + jitter, 0.0), steady);
    }
}

#[test]
fn a_quantised_stick_survives_a_round_trip_through_a_log() {
    // What comes out of the boundary has to be exactly representable, or the
    // log writes a number it cannot read back (DIM0703).
    for (x, y) in [
        (1.0, 0.0),
        (0.5, 0.5),
        (-0.8, 0.3),
        (0.0, -1.0),
        (0.6, -0.6),
    ] {
        let stick = quantize_stick(x, y);
        for component in [stick.x, stick.y] {
            let text = component.to_exact_string();
            assert_eq!(
                dimetric_core::Fx::parse_exact(&text).expect("reads back"),
                component,
                "{text} did not round trip"
            );
        }
    }
}

#[test]
fn a_stick_never_exceeds_full_deflection() {
    // A pad reporting slightly over 1.0 on a diagonal — most of them do —
    // must not make the player faster than the keyboard does.
    let full = quantize_stick(1.0, 0.0).length();
    for (x, y) in [(1.0, 1.0), (1.2, 0.0), (-1.1, -1.1)] {
        let stick = quantize_stick(x, y);
        assert!(
            stick.length() <= full,
            "({x}, {y}) gave {:?}, longer than full deflection",
            stick.length()
        );
    }
}

#[test]
fn a_pushed_stick_points_where_it_was_pushed() {
    let east = quantize_stick(1.0, 0.0);
    assert!(east.x > dimetric_core::Fx::ZERO && east.y.abs() < dimetric_core::Fx::from_int(1) / 8);

    // Down is positive y, as it is everywhere else in the engine.
    let south = quantize_stick(0.0, 1.0);
    assert!(south.y > dimetric_core::Fx::ZERO);
}

#[test]
fn the_dead_zone_has_an_edge_and_the_steps_are_evenly_spaced() {
    // Behaviour rather than the constants themselves: asserting
    // `STICK_STEPS >= 4` only tells you what you already typed.
    assert_eq!(quantize_stick(STICK_DEAD_ZONE - 0.01, 0.0), Vec2Fx::ZERO);
    assert_ne!(quantize_stick(STICK_DEAD_ZONE + 0.01, 0.0), Vec2Fx::ZERO);

    // Full deflection is one; each step is a whole fraction of it, so a
    // half-pushed stick is exactly half as fast and not a rounding of it.
    let full = quantize_stick(1.0, 0.0).x;
    let half = quantize_stick(0.5, 0.0).x;
    assert_eq!(full, dimetric_core::Fx::from_int(1));
    assert_eq!(half * dimetric_core::Fx::from_int(2), full);

    // And the steps really are distinct, or the quantisation is throwing away
    // more than jitter.
    let quarter = quantize_stick(0.25, 0.0).x;
    assert!(quarter < half && quarter > dimetric_core::Fx::ZERO);
}

#[test]
fn a_stick_overrides_the_keys_rather_than_adding_to_them() {
    let mut held = Held::new();
    held.set(Action::Right, true);
    let keys_only = held.player_input().move_dir;
    assert_eq!(keys_only, Vec2Fx::from_ints(1, 0));

    held.push_stick(Some(quantize_stick(0.0, 1.0)));
    let with_stick = held.player_input().move_dir;
    assert!(
        with_stick.length() <= dimetric_core::Fx::from_int(1),
        "a hand on both devices must not move at double speed"
    );
    assert!(with_stick.y > dimetric_core::Fx::ZERO, "the stick won");

    // Letting go of the stick hands control back to the keys.
    held.push_stick(None);
    assert_eq!(held.player_input().move_dir, keys_only);
}

#[test]
fn releasing_everything_clears_what_was_held() {
    // What happens when a window loses focus, or a pad is unplugged mid-game.
    let mut held = Held::new();
    held.set(Action::Fire, true);
    held.set(Action::Left, true);
    assert_ne!(held.player_input().buttons, 0);

    held.release_all();
    held.push_stick(None);
    let input = held.player_input();
    assert_eq!(input.buttons, 0);
    assert_eq!(input.move_dir, Vec2Fx::ZERO);
}

// -- The pointer ----------------------------------------------------------

fn canvas() -> Canvas {
    Canvas {
        width: 320,
        height: 180,
    }
}

#[test]
fn a_window_pixel_becomes_a_canvas_pixel() {
    // The window's size is divided out here and never reaches a tick, which is
    // what lets a recorded click land on the same button on a different
    // monitor.
    let c = canvas();
    assert_eq!(window_to_canvas((0.0, 0.0), (1280, 720), c), Vec2Fx::ZERO);
    assert_eq!(
        window_to_canvas((640.0, 360.0), (1280, 720), c),
        Vec2Fx::from_ints(160, 90)
    );
}

#[test]
fn the_same_fraction_of_any_window_is_the_same_canvas_pixel() {
    // Two players, two monitors, one recorded input log.
    let c = canvas();
    let small = window_to_canvas((320.0, 180.0), (640, 360), c);
    let large = window_to_canvas((960.0, 540.0), (1920, 1080), c);
    assert_eq!(small, large);
}

#[test]
fn a_pointer_outside_the_window_is_clamped_onto_the_canvas() {
    let c = canvas();
    let out = window_to_canvas((-50.0, 5000.0), (1280, 720), c);
    assert_eq!(out, Vec2Fx::from_ints(0, 179));
}

#[test]
fn the_pointer_reaches_the_player_input() {
    let mut held = Held::new();
    held.point_at(Vec2Fx::from_ints(12, 34));
    assert_eq!(held.player_input().pointer, Vec2Fx::from_ints(12, 34));
    assert_eq!(held.pointer(), Vec2Fx::from_ints(12, 34));
}

#[test]
fn aim_is_untouched_by_the_pointer() {
    // They are different things: the pointer is where the cursor is, aim is
    // which way the player is facing, and a pad sets the second without a
    // cursor existing at all.
    let mut held = Held::new();
    held.aim_at(Angle::from_degrees_str("90.0").unwrap());
    held.point_at(Vec2Fx::from_ints(5, 5));
    assert_eq!(
        held.player_input().aim,
        Angle::from_degrees_str("90.0").unwrap()
    );
}
