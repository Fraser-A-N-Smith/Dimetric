//! Tags becoming clips: frame order, and milliseconds becoming ticks.

use dimetric_assets::{clip_from_range, Playback};

/// 100ms, 50ms, 200ms, 50ms.
const DURATIONS: [u32; 4] = [100, 50, 200, 50];

#[test]
fn a_forward_tag_is_its_frames_in_order() {
    let clip = clip_from_range("walk", 0, 3, Playback::Forward, &DURATIONS, 60);
    let indices: Vec<u32> = clip.frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, [0, 1, 2, 3]);
    assert_eq!(clip.name, "walk");
}

#[test]
fn a_reverse_tag_is_expanded_rather_than_flagged() {
    // The runtime frame walker only goes forwards, so the direction has to be
    // gone by the time the clip reaches it.
    let clip = clip_from_range("unwind", 0, 3, Playback::Reverse, &DURATIONS, 60);
    let indices: Vec<u32> = clip.frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, [3, 2, 1, 0]);
}

#[test]
fn a_ping_pong_tag_does_not_hold_its_turnaround_frames_twice() {
    let clip = clip_from_range("sway", 0, 3, Playback::PingPong, &DURATIONS, 60);
    let indices: Vec<u32> = clip.frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, [0, 1, 2, 3, 2, 1]);
}

#[test]
fn a_single_frame_tag_is_one_frame_whichever_way_it_plays() {
    for playback in [Playback::Forward, Playback::Reverse, Playback::PingPong] {
        let clip = clip_from_range("idle", 2, 2, playback, &DURATIONS, 60);
        let indices: Vec<u32> = clip.frames.iter().map(|f| f.index).collect();
        assert_eq!(indices, [2], "{playback:?}");
    }
}

#[test]
fn durations_are_ticks_by_the_time_the_clip_exists() {
    // At 60Hz a tick is 16.667ms. 100ms is 6 ticks, 50ms is 3, 200ms is 12.
    let clip = clip_from_range("walk", 0, 3, Playback::Forward, &DURATIONS, 60);
    let ticks: Vec<u32> = clip.frames.iter().map(|f| f.ticks).collect();
    assert_eq!(ticks, [6, 3, 12, 3]);
    assert_eq!(clip.duration_ticks(), 24);
}

#[test]
fn the_tick_rate_changes_the_clip_because_it_was_applied_at_import() {
    let sixty = clip_from_range("walk", 0, 3, Playback::Forward, &DURATIONS, 60);
    let thirty = clip_from_range("walk", 0, 3, Playback::Forward, &DURATIONS, 30);
    assert_eq!(sixty.duration_ticks(), 24);
    // Not 12: each frame rounds on its own, and 50ms at 30Hz is one and a half
    // ticks, which rounds up. Halving the rate does not halve the clip, and the
    // only way that stays predictable is by converting once — here — rather
    // than on whatever tick rate a session happened to be running.
    assert_eq!(thirty.duration_ticks(), 13);
}

#[test]
fn a_frame_too_short_to_round_to_a_tick_still_lasts_one() {
    // 5ms at 60Hz rounds to zero, and a zero-length frame advances infinitely
    // fast and hangs the walker.
    let clip = clip_from_range("flicker", 0, 1, Playback::Forward, &[5, 5], 60);
    assert_eq!(
        clip.frames.iter().map(|f| f.ticks).collect::<Vec<_>>(),
        [1, 1]
    );
}

#[test]
fn walking_a_clip_lands_on_the_right_frame() {
    let clip = clip_from_range("walk", 0, 3, Playback::Forward, &DURATIONS, 60);
    // Frame boundaries at 0, 6, 9, 21, 24.
    assert_eq!(clip.frame_at(0).unwrap().index, 0);
    assert_eq!(clip.frame_at(5).unwrap().index, 0);
    assert_eq!(clip.frame_at(6).unwrap().index, 1);
    assert_eq!(clip.frame_at(8).unwrap().index, 1);
    assert_eq!(clip.frame_at(9).unwrap().index, 2);
    assert_eq!(clip.frame_at(20).unwrap().index, 2);
    assert_eq!(clip.frame_at(21).unwrap().index, 3);
    // And it loops.
    assert_eq!(clip.frame_at(24).unwrap().index, 0);
    assert_eq!(clip.frame_at(30).unwrap().index, 1);
}

#[test]
fn a_reversed_range_is_taken_as_a_range_rather_than_a_direction() {
    let clip = clip_from_range("odd", 3, 1, Playback::Forward, &DURATIONS, 60);
    let indices: Vec<u32> = clip.frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, [1, 2, 3]);
}
