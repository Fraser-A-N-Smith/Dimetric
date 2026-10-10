//! A named stream can be started over.
//!
//! A stream is created from the run seed the first time it is asked for and
//! kept for the rest of the session, which is right for a run that draws once
//! from each — and wrong for a game that plays the same run twice in one
//! launch. A run named by a code draws from `floor#KQPRMX` instead of `floor`,
//! so the first time a code is played it is exactly its run and the second time
//! it continues where the first left off, while the screen says they are the
//! same run.

use dimetric_core::{Rng, RngStreams};

/// Twelve draws from a stream, as a run's opening would be.
fn draws(streams: &mut RngStreams, name: &str) -> Vec<i32> {
    (0..12)
        .map(|_| streams.stream(name).range_i32(0, 1_000))
        .collect()
}

#[test]
fn a_stream_kept_running_is_a_different_run_the_second_time() {
    // The defect, which is the reason this exists.
    let mut streams = RngStreams::new(7);
    let first = draws(&mut streams, "floor#KQPRMX");
    let second = draws(&mut streams, "floor#KQPRMX");
    assert_ne!(first, second, "the second play has to differ, or no bug");
}

#[test]
fn a_reset_stream_is_exactly_the_run_again() {
    let mut streams = RngStreams::new(7);
    let first = draws(&mut streams, "floor#KQPRMX");
    streams.reset_stream("floor#KQPRMX");
    assert_eq!(draws(&mut streams, "floor#KQPRMX"), first);
}

#[test]
fn a_reset_is_as_if_the_stream_had_never_been_drawn_from() {
    // Stated against a fresh set of streams rather than against itself, so
    // this is about the construction and not merely about repeatability.
    let mut fresh = RngStreams::new(7);
    let expected = draws(&mut fresh, "floor#KQPRMX");

    let mut used = RngStreams::new(7);
    for _ in 0..5 {
        draws(&mut used, "floor#KQPRMX");
    }
    used.reset_stream("floor#KQPRMX");
    assert_eq!(draws(&mut used, "floor#KQPRMX"), expected);
}

#[test]
fn resetting_one_stream_leaves_the_others_where_they_were() {
    // The whole point of naming streams: a run resets its seven and nothing
    // else in the session shifts.
    let mut streams = RngStreams::new(7);
    let _ = draws(&mut streams, "floor");
    let before = draws(&mut streams, "damage");
    streams.reset_stream("floor");
    let after = draws(&mut streams, "damage");
    assert_ne!(before, after, "damage carried on, it did not restart");

    let mut control = RngStreams::new(7);
    let _ = draws(&mut control, "floor");
    let _ = draws(&mut control, "damage");
    assert_eq!(
        draws(&mut control, "damage"),
        after,
        "and carried on to exactly where it would have"
    );
}

#[test]
fn resetting_a_stream_never_used_creates_it_at_its_beginning() {
    let mut reset_first = RngStreams::new(7);
    reset_first.reset_stream("floor");
    let mut untouched = RngStreams::new(7);
    assert_eq!(
        draws(&mut reset_first, "floor"),
        draws(&mut untouched, "floor")
    );
}

#[test]
fn a_seeded_stream_does_not_depend_on_the_session_seed() {
    // What makes a run code portable. `reset` reconstructs from the session
    // seed, so the same code in two launches is the same run only if the
    // session seed is; an explicit seed does not depend on it at all.
    let mut one = RngStreams::new(7);
    let mut another = RngStreams::new(99_999);
    one.seed_stream("floor", 0xC0FFEE);
    another.seed_stream("floor", 0xC0FFEE);
    assert_eq!(draws(&mut one, "floor"), draws(&mut another, "floor"));

    // And two different seeds are two different runs, or it would not be a
    // seed.
    let mut third = RngStreams::new(7);
    third.seed_stream("floor", 0xC0FFEF);
    assert_ne!(
        draws(&mut RngStreams::new(7), "floor"),
        draws(&mut third, "floor")
    );
}

#[test]
fn a_reset_is_a_seed_of_the_session_seed() {
    // The relationship between the two, so neither can drift.
    let mut by_reset = RngStreams::new(7);
    by_reset.reset_stream("floor");
    let mut by_seed = RngStreams::new(7);
    by_seed.seed_stream("floor", 7);
    assert_eq!(draws(&mut by_reset, "floor"), draws(&mut by_seed, "floor"));
}

#[test]
fn the_name_still_decides_the_sequence_under_an_explicit_seed() {
    // Two streams on one seed are unrelated, which is what naming is for.
    let mut streams = RngStreams::new(7);
    streams.seed_stream("floor", 1);
    streams.seed_stream("damage", 1);
    assert_ne!(draws(&mut streams, "floor"), draws(&mut streams, "damage"));
}

#[test]
fn every_64_bit_pattern_is_a_usable_seed() {
    // Nothing reserved, zero included, so a game need not special-case a value
    // its code arithmetic happened to produce.
    for seed in [0u64, 1, u64::MAX, u64::MAX / 2] {
        let mut streams = RngStreams::new(7);
        streams.seed_stream("floor", seed);
        let first = draws(&mut streams, "floor");
        streams.seed_stream("floor", seed);
        assert_eq!(draws(&mut streams, "floor"), first, "seed {seed}");
    }
}

#[test]
fn a_reset_stream_matches_the_generator_it_claims_to_be() {
    // Against `Rng::named` directly, which is the definition the doc comment
    // gives. If the two ever disagree the doc is a lie and a run code is not
    // reproducible from the outside.
    let mut streams = RngStreams::new(7);
    let _ = draws(&mut streams, "floor");
    streams.reset_stream("floor");
    let mut plain = Rng::named(7, "floor");
    for i in 0..12 {
        assert_eq!(
            streams.stream("floor").range_i32(0, 1_000),
            plain.range_i32(0, 1_000),
            "draw {i}"
        );
    }
}
