//! A shipped game can start a different run each launch.
//!
//! A packaged game boots with the seed in its manifest, and every stream a
//! script draws from is derived from that one number — so the first run of
//! every launch of the same build was identical, and the second was identical
//! to the second. For a roguelike whose title screen offers a new run, that is
//! the same three essences every time somebody quits and comes back. Rebuilding
//! did not help: the build stamped the same seed.
//!
//! Nothing a script can read differed between two launches. `app.today()` is
//! the only thing from outside and it holds still for a day, which is the Daily
//! Descent and deliberately not this.

use dimetric_host::package::BootSeed;

#[test]
fn a_number_is_the_same_run_every_launch() {
    // Which is right, and still the default: a fixture, a capture and a bug
    // report all need it.
    let seed = BootSeed::Fixed(11);
    assert_eq!(seed.resolve(), 11);
    assert_eq!(seed.resolve(), 11, "and again");
}

#[test]
fn launch_is_a_different_run_each_time() {
    // Sixteen resolutions, all distinct. Not a probabilistic claim about
    // entropy: the clock advances between calls and the process id covers a
    // coarse one, so two launches cannot land on one number by accident.
    let mut seen: Vec<u64> = (0..16).map(|_| BootSeed::Launch.resolve()).collect();
    let before = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), before, "two launches must not be one run");
}

#[test]
fn a_launch_seed_is_spread_across_the_whole_number() {
    // Nanoseconds a moment apart differ only in their low bits, and a seed is
    // a whole number to `Rng::new`. Hashing is what stops two launches in the
    // same second looking related — so the high half has to move too.
    let highs: std::collections::BTreeSet<u32> = (0..16)
        .map(|_| (BootSeed::Launch.resolve() >> 32) as u32)
        .collect();
    assert!(
        highs.len() > 8,
        "the high half barely moved across 16 launches: {} distinct",
        highs.len()
    );
}

#[test]
fn a_manifest_says_which_it_wants_and_reads_back() {
    for (text, expected) in [
        ("11", BootSeed::Fixed(11)),
        ("0", BootSeed::Fixed(0)),
        ("launch", BootSeed::Launch),
        ("18446744073709551615", BootSeed::Fixed(u64::MAX)),
    ] {
        assert_eq!(BootSeed::parse(text), Some(expected), "{text}");
        assert_eq!(expected.to_string(), text, "{text} did not print back");
    }
}

#[test]
fn a_seed_this_build_cannot_read_is_refused_rather_than_guessed() {
    // `dim build` refuses it before anything is copied, and the runtime says so
    // rather than quietly always playing run zero.
    for bad in ["lanch", "", "-1", "1.5", "launch ", "Launch", "0x10"] {
        assert_eq!(BootSeed::parse(bad), None, "{bad:?} should be refused");
    }
}

#[test]
fn the_default_is_the_run_it_always_was() {
    // Every manifest written before this existed names a number, and a project
    // that says nothing still gets run zero.
    assert_eq!(BootSeed::default(), BootSeed::Fixed(0));
    assert_eq!(BootSeed::default().resolve(), 0);
}
