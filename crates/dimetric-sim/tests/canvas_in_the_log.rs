//! The canvas a session chose travels in its recording.
//!
//! A project may declare several canvas heights and let the engine pick the one
//! nearest the window's shape, so a 16:10 handheld gets taller rails instead of
//! bars. That makes the canvas a fact about the **session** rather than about
//! the project — and layout decides which control a click lands on, so a replay
//! that laid out against a different canvas would disagree about which button
//! was pressed and diverge for a reason nothing recorded.
//!
//! So it goes in the header, exactly as the date does, and it is omitted when
//! there was no choice to make.

use dimetric_scene::ui::Canvas;
use dimetric_sim::input::{InputFrame, InputLog};

fn log() -> InputLog {
    let mut log = InputLog::new(7, "test", 1);
    log.push(InputFrame::idle(1));
    log.push(InputFrame::idle(1));
    log
}

#[test]
fn a_log_with_no_canvas_is_byte_for_byte_what_it_always_was() {
    // The property that keeps every recording already made readable, and the
    // reason the line is omitted rather than written with a default.
    let text = log().to_text();
    assert!(
        !text.contains("canvas"),
        "a session that chose nothing wrote a canvas line:\n{text}"
    );
}

#[test]
fn a_chosen_canvas_survives_the_log() {
    let text = log().with_canvas(1920, 1200).to_text();
    assert!(text.contains("canvas 1920 1200"), "{text}");
    let read = InputLog::parse(&text).expect("it parses");
    assert_eq!(read.canvas, Some((1920, 1200)));
    // And writing it again is the same text, which is what makes a recording
    // round-trip rather than drift.
    assert_eq!(read.to_text(), text);
}

#[test]
fn the_log_decides_what_a_replay_lays_out_against() {
    let project = Canvas {
        width: 1920,
        height: 1080,
    };
    // A log that chose: its own, whatever the project now declares.
    let chose = log().with_canvas(1920, 1200);
    assert_eq!(
        chose.canvas_or(project),
        Canvas {
            width: 1920,
            height: 1200
        }
    );
    // A log that did not: the project's, because that is what it was laid out
    // against when it was recorded.
    assert_eq!(log().canvas_or(project), project);
}

#[test]
fn a_nonsense_canvas_line_is_refused_rather_than_guessed_at() {
    // The canvas decides what a click hit, so replaying against a wrong one
    // would reproduce a different game and say nothing. Every one of these is
    // a header a hand-edited log could carry.
    for bad in [
        "canvas\n",
        "canvas 1920\n",
        "canvas 1920 x\n",
        "canvas 0 1200\n",
        "canvas 1920 0\n",
        "canvas -1920 1200\n",
    ] {
        let text =
            format!("dimetric-input 1\nseed 7\nengine test\nplayers 1\n{bad}0 0000 0.0 0.0 0.0\n");
        assert!(InputLog::parse(&text).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn the_canvas_line_sits_with_the_other_header_facts() {
    // Beside the date and the actions, all of them things the host told the
    // session rather than things the project fixes. Order matters only in that
    // the writer and the parser agree, which a round trip proves.
    let text = log()
        .with_date("2026-11-06")
        .with_actions(vec!["undo".to_string()])
        .with_canvas(1920, 1200)
        .to_text();
    let read = InputLog::parse(&text).expect("it parses");
    assert_eq!(read.date.as_deref(), Some("2026-11-06"));
    assert_eq!(read.actions, ["undo"]);
    assert_eq!(read.canvas, Some((1920, 1200)));
    assert_eq!(read.to_text(), text);
}
