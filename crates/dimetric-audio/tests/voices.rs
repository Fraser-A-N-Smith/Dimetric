//! Voice allocation, stealing and caps.

use dimetric_audio::{db_to_linear, pitch_variation, Bus, Rejected, Voice, VoicePool};
use dimetric_core::Rng;

fn voice(clip: &str, priority: u8) -> Voice {
    Voice {
        clip: clip.into(),
        bus: Bus::Sfx,
        volume_db: 0.0,
        pitch: 1.0,
        pan: 0.0,
        priority,
        handle: 0,
        age: 0,
    }
}

#[test]
fn a_clip_cannot_flood_the_pool() {
    let mut pool = VoicePool::new(32, 3);
    for _ in 0..3 {
        assert!(pool.play(voice("impact", 1)).is_ok());
    }
    assert_eq!(pool.play(voice("impact", 1)), Err(Rejected::ClipCapReached));
    // Other clips are unaffected: the cap is per clip, not a global limit.
    assert!(pool.play(voice("cast", 1)).is_ok());
}

#[test]
fn a_full_pool_steals_the_oldest_lowest_priority_voice() {
    let mut pool = VoicePool::new(2, 8);
    let quiet = pool.play(voice("footstep", 1)).unwrap();
    pool.age();
    pool.age();
    let loud = pool.play(voice("roar", 5)).unwrap();
    pool.age();

    // A new high-priority sound evicts the footstep, not the roar.
    let new = pool.play(voice("thunder", 5)).unwrap();
    let clips: Vec<&str> = pool.voices().iter().map(|v| v.clip.as_str()).collect();
    assert!(clips.contains(&"roar"), "higher priority survives: {clips:?}");
    assert!(!clips.contains(&"footstep"));
    assert_ne!(new, quiet);
    assert_ne!(new, loud);
}

#[test]
fn a_low_priority_sound_cannot_evict_a_high_priority_one() {
    let mut pool = VoicePool::new(1, 8);
    pool.play(voice("boss_roar", 9)).unwrap();
    assert_eq!(
        pool.play(voice("footstep", 1)),
        Err(Rejected::NoVoiceAvailable),
        "a footstep must not cut off the boss"
    );
}

#[test]
fn stopping_frees_a_slot() {
    let mut pool = VoicePool::new(1, 8);
    let handle = pool.play(voice("hum", 1)).unwrap();
    assert!(pool.stop(handle));
    assert!(pool.is_empty());
    assert!(pool.play(voice("hum", 1)).is_ok());
    assert!(!pool.stop(handle), "stopping twice is not an error, just false");
}

#[test]
fn a_bus_can_be_silenced_without_touching_the_others() {
    let mut pool = VoicePool::new(8, 8);
    let mut music = voice("theme", 1);
    music.bus = Bus::Music;
    pool.play(music).unwrap();
    pool.play(voice("hit", 1)).unwrap();
    pool.stop_bus(Bus::Music);
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.voices()[0].clip, "hit");
}

#[test]
fn bus_gain_scales_a_voice() {
    let mut pool = VoicePool::new(4, 4);
    pool.play(voice("hit", 1)).unwrap();
    pool.set_gain(Bus::Sfx, 0.5);
    let gain = pool.effective_gain(&pool.voices()[0]);
    assert!((gain - 0.5).abs() < 1e-5, "expected half gain, got {gain}");
}

#[test]
fn decibels_convert_the_way_everyone_expects() {
    assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    assert!((db_to_linear(-6.0) - 0.501).abs() < 0.01);
    assert!(db_to_linear(-80.0) < 0.001);
}

#[test]
fn pitch_variation_stays_within_its_spread_and_repeats_for_a_seed() {
    let mut a = Rng::new(11, 1);
    let mut b = Rng::new(11, 1);
    for _ in 0..64 {
        let pitch = pitch_variation(&mut a, 0.2);
        assert!((0.8..=1.2).contains(&pitch), "{pitch} left its spread");
        assert_eq!(pitch, pitch_variation(&mut b, 0.2));
    }
    assert_eq!(pitch_variation(&mut a, 0.0), 1.0, "no spread means no change");
}
