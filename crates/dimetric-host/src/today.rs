//! What day it is, read once, at the edge of the engine.
//!
//! A simulation may not read a clock: invariant I5 says the only notion of time
//! inside a tick is the tick count, because anything else makes a replay depend
//! on when it was replayed. But a **daily run** — one run a day, the same seed
//! for every player — is a thing a deterministic engine is unusually well
//! placed to offer, and the menu that offers it is part of the simulation.
//!
//! So the date is read here, once, before the first tick, by the thing that is
//! already allowed to touch the operating system; it is handed to the sandbox
//! as a fixed string, and it is written into the recording. A replay is told
//! what the recording was told, which is what makes a recording of a title
//! screen still replay next week.
//!
//! UTC, not local time. Two players on one calendar day have to be offered one
//! seed, and "one day" has to mean the same span everywhere or the daily run is
//! a different run either side of a time zone.

/// Today's date in UTC, as `YYYY-MM-DD`.
pub fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    from_unix_seconds(seconds)
}

/// The UTC date a Unix timestamp falls on, as `YYYY-MM-DD`.
///
/// Separate from [`today`] so it can be tested: a date routine nobody checked
/// is one that is wrong on a leap day and right every other day of the year.
pub fn from_unix_seconds(seconds: i64) -> String {
    // Floor division, so a timestamp before 1970 lands on the right day rather
    // than one day late.
    let days = seconds.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a civil date.
///
/// Howard Hinnant's algorithm, which is exact for every date in the proleptic
/// Gregorian calendar and is integer arithmetic throughout. Written out rather
/// than pulled in: a date crate for one function would be a dependency on the
/// one path in this engine that must not surprise anybody.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01, which puts the leap day at the end of the
    // year and makes the month arithmetic a straight line.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // 0 .. 146096
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // 0 .. 399
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0 .. 365
    let mp = (5 * doy + 2) / 153; // 0 .. 11, March-based
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // 1 .. 31
    let m = match mp < 10 {
        true => mp + 3,
        false => mp - 9,
    } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// Whether a string is a date this engine will accept.
///
/// Checked rather than trusted, because a `--date` typed at a terminal and a
/// `date` line in a log both reach `app.today()` and a game will be slicing
/// the string up to build a seed from it.
pub fn is_valid(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| {
        bytes[range.clone()].iter().all(u8::is_ascii_digit) && text[range].parse::<u32>().is_ok()
    };
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let (y, m, d) = (
        text[0..4].parse::<i64>().unwrap_or(0),
        text[5..7].parse::<u32>().unwrap_or(0),
        text[8..10].parse::<u32>().unwrap_or(0),
    );
    (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// How many days a month has, leap years included.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => match year.rem_euclid(4) == 0
            && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
        {
            true => 29,
            false => 28,
        },
        _ => 0,
    }
}
