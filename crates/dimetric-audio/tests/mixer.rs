//! The mixer: stealing, caps, fades and bus gain, all against the mock backend.

use dimetric_audio::backend::{stopped, Backend, Event, Log, Mock};
use dimetric_audio::{Bus, Curve, Mixer, Play, Rejected, Tween};

/// A mixer with three clips loaded, plus the backend's event log.
fn mixer_with_log(capacity: usize, per_clip: usize) -> (Mixer, Log) {
    let backend = Mock::new();
    let log = backend.log();
    let mut mixer = Mixer::new(Box::new(backend), capacity, per_clip, 7);
    for clip in ["sfx/step", "sfx/spell", "music/arena"] {
        mixer
            .load(clip, b"not really an ogg")
            .expect("the mock loads anything");
    }
    (mixer, log)
}

fn mixer(capacity: usize, per_clip: usize) -> Mixer {
    mixer_with_log(capacity, per_clip).0
}

#[test]
fn a_sound_that_was_never_loaded_does_not_take_a_voice() {
    let mut mixer = mixer(4, 4);
    assert!(mixer.play(Play::clip("sfx/nonexistent")).is_err());
    assert_eq!(
        mixer.pool().len(),
        0,
        "the slot is not held by a silent voice"
    );
}

#[test]
fn a_full_pool_steals_the_oldest_lowest_priority_voice() {
    let mut mixer = mixer(2, 4);
    let quiet = mixer.play(Play::clip("sfx/step").priority(10)).unwrap();
    mixer.advance(1.0);
    let loud = mixer.play(Play::clip("sfx/spell").priority(200)).unwrap();
    mixer.advance(1.0);

    let third = mixer.play(Play::clip("sfx/step").priority(100)).unwrap();
    let live: Vec<u64> = mixer.pool().voices().iter().map(|v| v.handle).collect();
    assert!(!live.contains(&quiet), "the footstep loses");
    assert!(live.contains(&loud), "the boss roar survives");
    assert!(live.contains(&third));
}

#[test]
fn nothing_is_stolen_from_a_higher_priority_than_the_newcomer() {
    let mut mixer = mixer(1, 4);
    mixer.play(Play::clip("sfx/spell").priority(200)).unwrap();
    assert_eq!(
        mixer.play(Play::clip("sfx/step").priority(10)),
        Err(Rejected::NoVoiceAvailable)
    );
}

#[test]
fn one_noisy_clip_cannot_evict_everything_else() {
    let mut mixer = mixer(8, 2);
    mixer.play(Play::clip("sfx/step")).unwrap();
    mixer.play(Play::clip("sfx/step")).unwrap();
    assert_eq!(
        mixer.play(Play::clip("sfx/step")),
        Err(Rejected::ClipCapReached)
    );
    // And a different clip is unaffected.
    assert!(mixer.play(Play::clip("sfx/spell")).is_ok());
}

#[test]
fn a_fade_out_keeps_the_voice_until_it_reaches_silence() {
    let mut mixer = mixer(4, 4);
    let handle = mixer
        .play(Play::clip("music/arena").on(Bus::Music))
        .unwrap();
    mixer.stop(handle, 2.0);
    assert_eq!(mixer.pool().len(), 1, "still audible half way through");
    mixer.advance(1.0);
    assert_eq!(mixer.pool().len(), 1);
    mixer.advance(1.5);
    assert_eq!(mixer.pool().len(), 0, "the voice is released at the end");
}

#[test]
fn a_stop_with_no_fade_is_immediate() {
    let mut mixer = mixer(4, 4);
    let handle = mixer.play(Play::clip("sfx/step")).unwrap();
    mixer.stop(handle, 0.0);
    assert_eq!(mixer.pool().len(), 0);
}

#[test]
fn a_bus_fade_arrives_where_it_was_sent() {
    let mut mixer = mixer(4, 4);
    assert_eq!(mixer.bus_gain(Bus::Music), 1.0);
    mixer.fade_bus(Bus::Music, 0.0, 2.0, Curve::Linear);
    mixer.advance(1.0);
    let half = mixer.bus_gain(Bus::Music);
    assert!(half > 0.0 && half < 1.0, "half way: {half}");
    mixer.advance(1.0);
    assert_eq!(mixer.bus_gain(Bus::Music), 0.0);
    // And the other buses are untouched: ducking the music should not duck the
    // interface.
    assert_eq!(mixer.bus_gain(Bus::Ui), 1.0);
}

#[test]
fn a_fade_interrupted_by_another_does_not_jump() {
    let mut mixer = mixer(4, 4);
    mixer.fade_bus(Bus::Music, 0.0, 4.0, Curve::Linear);
    mixer.advance(2.0);
    let midway = mixer.bus_gain(Bus::Music);

    mixer.fade_bus(Bus::Music, 1.0, 4.0, Curve::Linear);
    // The new fade starts from where the old one had got to, not from where it
    // was heading.
    assert!((mixer.bus_gain(Bus::Music) - midway).abs() < 0.001);
}

#[test]
fn pitch_variation_comes_from_a_stream_of_its_own() {
    // Sharing the simulation's stream would mean that triggering one fewer
    // sound effect shifted every gameplay roll after it.
    let mut a = mixer(8, 8);
    let mut b = mixer(8, 8);
    for _ in 0..4 {
        a.play(Play::clip("sfx/spell").pitch_spread(0.2)).unwrap();
    }
    for _ in 0..4 {
        b.play(Play::clip("sfx/spell").pitch_spread(0.2)).unwrap();
    }
    let pitches_a: Vec<f32> = a.pool().voices().iter().map(|v| v.pitch).collect();
    let pitches_b: Vec<f32> = b.pool().voices().iter().map(|v| v.pitch).collect();
    assert_eq!(pitches_a, pitches_b, "same seed, same sequence");
    assert!(
        pitches_a.iter().any(|p| (*p - 1.0).abs() > 0.001),
        "and it actually varies: {pitches_a:?}"
    );
    assert!(pitches_a.iter().all(|p| (*p - 1.0).abs() <= 0.2));
}

#[test]
fn no_spread_means_no_variation() {
    let mut mixer = mixer(4, 4);
    mixer.play(Play::clip("sfx/step")).unwrap();
    assert_eq!(mixer.pool().voices()[0].pitch, 1.0);
}

#[test]
fn stopping_a_bus_leaves_the_others_playing() {
    let mut mixer = mixer(8, 8);
    mixer
        .play(Play::clip("music/arena").on(Bus::Music))
        .unwrap();
    mixer.play(Play::clip("sfx/step").on(Bus::Sfx)).unwrap();
    mixer.stop_bus(Bus::Music, 0.0);
    assert_eq!(mixer.pool().len(), 1);
    assert_eq!(mixer.pool().voices()[0].bus, Bus::Sfx);
}

#[test]
fn the_headless_backend_is_what_a_default_device_opens() {
    let backend = dimetric_audio::Device::default().open();
    assert_eq!(backend.name(), "mock", "no device I/O in a headless run");
}

// -- tweens -------------------------------------------------------------

#[test]
fn a_zero_length_tween_is_a_cut_rather_than_a_division_by_zero() {
    let tween = Tween::new(0.0, 1.0, 0.0, Curve::Linear);
    assert_eq!(tween.value(), 1.0);
    assert!(tween.is_done());
}

#[test]
fn every_curve_starts_and_ends_where_it_says_it_does() {
    for curve in [
        Curve::Linear,
        Curve::EaseIn,
        Curve::EaseOut,
        Curve::EaseInOut,
    ] {
        let mut tween = Tween::new(2.0, 8.0, 1.0, curve);
        assert_eq!(tween.value(), 2.0, "{curve:?} at the start");
        tween.advance(1.0);
        assert_eq!(tween.value(), 8.0, "{curve:?} at the end");
    }
}

#[test]
fn a_tween_does_not_overshoot_when_time_runs_past_it() {
    let mut tween = Tween::new(0.0, 1.0, 1.0, Curve::Linear);
    tween.advance(100.0);
    assert_eq!(tween.value(), 1.0);
}

#[test]
fn ease_out_holds_the_value_up_before_dropping_it() {
    // Which is why it is the curve a fade-out wants.
    let mut linear = Tween::new(1.0, 0.0, 1.0, Curve::Linear);
    let mut ease = Tween::new(1.0, 0.0, 1.0, Curve::EaseOut);
    linear.advance(0.25);
    ease.advance(0.25);
    assert!(
        ease.value() < linear.value(),
        "{} vs {}",
        ease.value(),
        linear.value()
    );
}

#[test]
fn the_mock_backend_writes_down_what_it_was_asked_to_do() {
    let mut backend = Mock::new();
    let log = backend.log();
    backend.load("sfx/step", b"bytes").unwrap();
    assert!(backend.is_loaded("sfx/step"));
    assert!(matches!(log.borrow()[0], Event::Loaded { .. }));
}

#[test]
fn a_stolen_voice_is_stopped_on_the_device_rather_than_left_ringing() {
    let (mut mixer, log) = mixer_with_log(1, 4);
    let first = mixer.play(Play::clip("sfx/step")).unwrap();
    let second = mixer.play(Play::clip("sfx/spell")).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        stopped(&log.borrow()),
        [first],
        "the pool evicted it, so the backend has to hear about it"
    );
}
