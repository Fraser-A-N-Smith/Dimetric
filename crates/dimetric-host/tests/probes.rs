//! Probe files: `tick <n> <path> <field> <op> <value>`.
//!
//! The value used to be `parts[5..].join(" ")` over a whitespace split, which
//! had three consequences and no warnings:
//!
//!   * a quoted literal was compared *including its quote characters*, so it
//!     never matched anything — and quoting is the natural thing to write,
//!     since every other literal in this toolchain is TOML-ish;
//!   * an empty string could not be written at all, because bare `""` is two
//!     quote characters against nothing;
//!   * a `#` anywhere on the line started a comment, so the first character of
//!     every colour this engine writes truncated the probe.

use dimetric_host::replay::parse_probes;

fn value(line: &str) -> String {
    let probes = parse_probes(line).unwrap_or_else(|d| panic!("{d}"));
    assert_eq!(probes.len(), 1, "one probe: {line:?}");
    probes[0].value.clone()
}

#[test]
fn a_bare_value_is_itself() {
    assert_eq!(value("tick 2 /T var:plain == none"), "none");
}

#[test]
fn a_quoted_value_loses_its_quotes() {
    // The defect. `"none"` was compared as six characters against four.
    assert_eq!(value("tick 2 /T var:plain == \"none\""), "none");
}

#[test]
fn quoting_is_optional_and_means_the_same_thing() {
    assert_eq!(
        value("tick 2 /T var:plain == none"),
        value("tick 2 /T var:plain == \"none\"")
    );
}

#[test]
fn a_value_may_contain_spaces_either_way() {
    assert_eq!(value("tick 2 /T var:s == two words"), "two words");
    assert_eq!(value("tick 2 /T var:s == \"two words\""), "two words");
}

#[test]
fn the_value_keeps_its_own_spacing() {
    // Rejoining a whitespace split with single spaces quietly rewrote this.
    assert_eq!(value("tick 2 /T var:s == \"two  words\""), "two  words");
    assert_eq!(value("tick 2 /T var:s == \" padded \""), " padded ");
}

#[test]
fn an_empty_string_can_be_asserted() {
    assert_eq!(value("tick 2 /T var:s == \"\""), "");
}

#[test]
fn a_hash_inside_quotes_is_not_a_comment() {
    // Every colour this engine writes starts with one, and scripts can set
    // colours now, so this is a value somebody will want to assert on.
    assert_eq!(value("tick 2 /T var:tint == \"#ff8f4aff\""), "#ff8f4aff");
}

#[test]
fn a_hash_outside_quotes_still_is_one() {
    assert_eq!(value("tick 2 /T var:s == none  # a note"), "none");
    assert!(parse_probes("# just a comment").expect("parses").is_empty());
}

#[test]
fn an_unterminated_quote_is_reported_rather_than_read_raw() {
    let d = parse_probes("tick 2 /T var:s == \"none").expect_err("refused");
    assert!(d.message.contains("closing quote"), "{d}");
}

#[test]
fn a_line_with_no_value_is_still_refused() {
    let d = parse_probes("tick 2 /T var:s ==").expect_err("refused");
    assert!(d.message.contains("expected"), "{d}");
}

#[test]
fn the_other_fields_are_unchanged() {
    let probes = parse_probes("tick 40 /Stage/Lamp var:r >= 41").expect("parses");
    assert_eq!(probes[0].tick, 40);
    assert_eq!(probes[0].path, "/Stage/Lamp");
    assert_eq!(probes[0].field, "var:r");
    assert_eq!(probes[0].value, "41");
}
