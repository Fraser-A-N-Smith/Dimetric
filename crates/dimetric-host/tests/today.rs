//! What day it is, read at the edge of the engine.
//!
//! A simulation may not read a clock (I5), and a **daily run** — one run a day,
//! the same seed for every player — is a thing a deterministic engine is
//! unusually well placed to offer. The menu that offers it is part of the
//! simulation, so the date is read once here, handed to the sandbox as a fixed
//! string, and written into the recording.
//!
//! A date routine nobody checked is one that is wrong on a leap day and right
//! every other day of the year, which is why this file exists.

use dimetric_host::today::{from_unix_seconds, is_valid, today};

#[test]
fn the_epoch_is_the_first_of_january_nineteen_seventy() {
    assert_eq!(from_unix_seconds(0), "1970-01-01");
    assert_eq!(
        from_unix_seconds(86_399),
        "1970-01-01",
        "one second before midnight"
    );
    assert_eq!(from_unix_seconds(86_400), "1970-01-02");
}

#[test]
fn a_leap_day_is_a_day() {
    // 2024 is a leap year: 2024-02-29 exists and 2023-02-29 does not.
    assert_eq!(from_unix_seconds(1_709_164_800), "2024-02-29");
    assert_eq!(from_unix_seconds(1_709_251_200), "2024-03-01");
    assert!(is_valid("2024-02-29"));
    assert!(!is_valid("2023-02-29"));
}

#[test]
fn a_century_is_not_a_leap_year_unless_it_is_a_fourth_one() {
    // The rule everybody gets wrong. 1900 was not a leap year; 2000 was.
    assert!(!is_valid("1900-02-29"));
    assert!(is_valid("2000-02-29"));
    assert!(!is_valid("2100-02-29"));
}

#[test]
fn a_time_before_the_epoch_lands_on_the_right_day() {
    // Floor division, not truncation: one second before the epoch is the day
    // before it, not the day of it.
    assert_eq!(from_unix_seconds(-1), "1969-12-31");
    assert_eq!(from_unix_seconds(-86_400), "1969-12-31");
    assert_eq!(from_unix_seconds(-86_401), "1969-12-30");
}

#[test]
fn a_few_known_dates_come_out_right() {
    for (seconds, date) in [
        (1_000_000_000, "2001-09-09"),
        (1_234_567_890, "2009-02-13"),
        (1_600_000_000, "2020-09-13"),
        (2_000_000_000, "2033-05-18"),
    ] {
        assert_eq!(from_unix_seconds(seconds), date, "{seconds}");
    }
}

#[test]
fn every_day_of_a_leap_year_round_trips() {
    // 2024 has 366 days. Walking them all catches an off-by-one in the
    // month arithmetic that four spot checks would not.
    let start = 1_704_067_200; // 2024-01-01
    let mut seen = Vec::new();
    for day in 0..366 {
        let date = from_unix_seconds(start + day * 86_400);
        assert!(is_valid(&date), "day {day} produced {date}");
        assert!(date.starts_with("2024-"), "day {day} produced {date}");
        seen.push(date);
    }
    seen.dedup();
    assert_eq!(seen.len(), 366, "every day has to be its own");
    assert_eq!(seen.first().map(String::as_str), Some("2024-01-01"));
    assert_eq!(seen.last().map(String::as_str), Some("2024-12-31"));
}

#[test]
fn a_date_is_checked_rather_than_trusted() {
    // A `--date` typed at a terminal and a `date` line in a log both reach
    // `app.today()`, and a game will be slicing the string up to build a seed.
    for good in ["1970-01-01", "2026-10-10", "0001-01-01", "9999-12-31"] {
        assert!(is_valid(good), "{good}");
    }
    for bad in [
        "",
        "2026-10-1",
        "2026-1-10",
        "26-10-10",
        "2026/10/10",
        "2026-13-01",
        "2026-00-01",
        "2026-10-00",
        "2026-10-32",
        "2026-04-31",
        "today",
        "2026-10-10 ",
        "2026-10-10T00:00:00Z",
    ] {
        assert!(!is_valid(bad), "{bad:?} should be refused");
    }
}

#[test]
fn the_clock_produces_something_this_engine_will_read() {
    // The one thing worth asserting about the real clock: whatever it says,
    // `app.today()` has to be able to carry it and a log has to be able to
    // write it.
    let now = today();
    assert!(is_valid(&now), "{now}");
    assert_eq!(now.len(), 10);
}
