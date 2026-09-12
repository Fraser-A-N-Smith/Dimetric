//! Import settings and the millisecond-to-tick conversion.

use dimetric_assets::{
    content_hash, ms_to_ticks, Clip, Frame, ImportSettings, MetaError, SourceKind,
};
use dimetric_core::AssetId;

#[test]
fn frame_timings_convert_to_whole_ticks_at_import_time() {
    // 100 ms at 60 Hz is 6 ticks.
    assert_eq!(ms_to_ticks(100, 60), 6);
    assert_eq!(ms_to_ticks(1000, 60), 60);
    // Half away from zero, so 8.4 ticks rounds down and 8.5 rounds up.
    assert_eq!(ms_to_ticks(140, 60), 8);
    assert_eq!(ms_to_ticks(141, 60), 8);
    assert_eq!(ms_to_ticks(142, 60), 9);
}

#[test]
fn a_frame_never_lasts_zero_ticks() {
    // A zero-length frame advances infinitely fast and hangs the frame walker,
    // so a very short source frame becomes one tick rather than none.
    assert_eq!(ms_to_ticks(1, 60), 1);
    assert_eq!(ms_to_ticks(0, 60), 1);
}

#[test]
fn the_same_source_converts_the_same_way_at_every_tick_rate() {
    // Converting at runtime would make this depend on the session's tick rate,
    // which is exactly why the conversion is pinned at import.
    assert_eq!(ms_to_ticks(100, 30), 3);
    assert_eq!(ms_to_ticks(100, 60), 6);
    assert_eq!(ms_to_ticks(100, 120), 12);
}

fn clip() -> Clip {
    Clip {
        name: "attack".into(),
        frames: vec![
            Frame {
                index: 0,
                ticks: 3,
                event: None,
            },
            Frame {
                index: 1,
                ticks: 2,
                event: Some("hitbox_on".into()),
            },
            Frame {
                index: 2,
                ticks: 5,
                event: Some("hitbox_off".into()),
            },
        ],
        looping: false,
    }
}

#[test]
fn a_clip_reports_which_frame_is_showing() {
    let c = clip();
    assert_eq!(c.duration_ticks(), 10);
    assert_eq!(c.frame_at(0).unwrap().index, 0);
    assert_eq!(c.frame_at(2).unwrap().index, 0);
    assert_eq!(c.frame_at(3).unwrap().index, 1);
    assert_eq!(c.frame_at(4).unwrap().index, 1);
    assert_eq!(c.frame_at(5).unwrap().index, 2);
    // A non-looping clip holds its last frame.
    assert_eq!(c.frame_at(99).unwrap().index, 2);
}

#[test]
fn a_looping_clip_wraps() {
    let mut c = clip();
    c.looping = true;
    assert_eq!(c.frame_at(10).unwrap().index, 0);
    assert_eq!(c.frame_at(13).unwrap().index, 1);
}

#[test]
fn the_damage_window_is_frame_accurate() {
    // A spell's damage window is gameplay, so it has to land on the same tick
    // in every replay. Frame 1 carries the event and occupies ticks 3 and 4.
    let c = clip();
    let firing: Vec<u32> = (0..10)
        .filter(|t| c.frame_at(*t).and_then(|f| f.event.as_deref()) == Some("hitbox_on"))
        .collect();
    assert_eq!(firing, vec![3, 4]);
}

#[test]
fn import_settings_round_trip_through_text() {
    let id = AssetId::parse("a_sprite01").unwrap();
    let mut settings = ImportSettings::new(id);
    settings.source_hash = Some("abc123".into());
    settings.nearest = false;
    let text = settings.to_text();
    assert_eq!(ImportSettings::parse(&text).unwrap(), settings);
    assert_eq!(ImportSettings::parse(&text).unwrap().to_text(), text);
}

#[test]
fn import_settings_without_an_id_are_refused() {
    // The id is what survives a rename, so settings without one are useless.
    assert_eq!(
        ImportSettings::parse("nearest = true\n"),
        Err(MetaError::MissingId)
    );
}

#[test]
fn source_kinds_are_recognised_by_extension() {
    use std::path::Path;
    assert_eq!(SourceKind::of(Path::new("a/b.png")), Some(SourceKind::Png));
    assert_eq!(
        SourceKind::of(Path::new("a/b.aseprite")),
        Some(SourceKind::Aseprite)
    );
    assert_eq!(
        SourceKind::of(Path::new("a/b.ASE")),
        Some(SourceKind::Aseprite)
    );
    assert_eq!(SourceKind::of(Path::new("a/b.ogg")), Some(SourceKind::Ogg));
    assert_eq!(
        SourceKind::of(Path::new("a/b.ldtk")),
        Some(SourceKind::Ldtk)
    );
    assert_eq!(SourceKind::of(Path::new("a/b.txt")), None);
}

#[test]
fn content_hashing_is_stable_and_sensitive() {
    assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
    assert_ne!(content_hash(b"hello"), content_hash(b"hellp"));
    assert_eq!(content_hash(b"hello").len(), 64);
}
