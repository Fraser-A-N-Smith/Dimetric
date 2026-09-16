//! A determinism lint for Lua, because `cargo xtask lint-sim` only sees Rust.
//!
//! `CONTRIBUTING.md` makes the argument for the Rust lints and it applies here
//! word for word: the ways a game breaks replay do not fail loudly, they fail
//! three weeks later as a divergence at tick 4,117 on someone else's machine.
//! Game logic is Lua, and until now it was held to I3 and I4 by discipline.
//!
//! Three things reach the state and should not:
//!
//! **`pairs()` over a table.** Lua's hash iteration order is unspecified. A
//! loop that writes to state in that order writes a different state on a
//! different machine, or after an unrelated allocation. `ENGINE-GAPS.md`
//! already records this as the reason `OFFER_ORDER` exists in the slice.
//!
//! **Raw float arithmetic.** `docs/API.md` calls going through `fx` "the rule
//! that matters" and then leaves it to discipline. A Lua number is an f64, and
//! `self.x = self.x + 0.1` puts one into the hash.
//!
//! **A profile read reaching a state write.** Added with the profile itself:
//! it is outside the hash by construction, which stops it *being* hashed and
//! does nothing about a script branching on it. Two players with different
//! unlocks then run different simulations.
//!
//! # What this is not
//!
//! It is a text scan, not a type system. It reads a script line by line and
//! looks for a hazard and a state write near each other; it cannot follow a
//! value through a function call, and it does not try. So it is deliberately
//! biased: it would rather name a safe `pairs()` than miss an unsafe one,
//! because the cost of the first is one comment and the cost of the second is
//! a week of bisecting. `-- @ordered` on the line says the author checked.

use dimetric_core::{Code, Diagnostic, Span};

/// The comment that says an author has checked a `pairs()` is safe.
pub const ORDERED_MARK: &str = "@ordered";
/// The comment that says a float literal never reaches state.
pub const PRESENTATION_MARK: &str = "@presentation";

/// Scan one script for determinism hazards.
///
/// `path` is only used for the diagnostics' spans.
pub fn check(path: &str, source: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let code = blank_strings(&strip_comment(raw));
        let acknowledged = |mark: &str| raw.contains(mark);

        if mentions_pairs(&code) && !acknowledged(ORDERED_MARK) {
            out.push(
                Diagnostic::new(
                    Code::SCRIPT_NONDETERMINISM,
                    "`pairs()` iterates in an unspecified order; a state write inside one \
                     is not reproducible. Use `ipairs` over an array, sort the keys, or \
                     write `-- @ordered` if the order cannot reach state.",
                )
                .with_span(Span::at(path.to_string(), line as u32))
                .with_field("hazard", "pairs"),
            );
        }

        if let Some(literal) = float_literal(&code) {
            if !acknowledged(PRESENTATION_MARK) {
                out.push(
                    Diagnostic::new(
                        Code::SCRIPT_NONDETERMINISM,
                        format!(
                            "`{literal}` is a Lua number, which is a float. Arithmetic on it \
                             is not exact and does not belong in state — use `fx.new` or \
                             `fx.parse`, or write `-- @presentation` if it never reaches a \
                             state write."
                        ),
                    )
                    .with_span(Span::at(path.to_string(), line as u32))
                    .with_field("hazard", "float"),
                );
            }
        }

        if code.contains("profile.get") && writes_state(&code) {
            out.push(
                Diagnostic::new(
                    Code::SCRIPT_NONDETERMINISM,
                    "a profile value is being written into simulation state. The profile is \
                     outside the hash on purpose, so two players with different unlocks \
                     would run different simulations from the same seed. Read it at the \
                     menu and carry the choice in through `scene.request_load`.",
                )
                .with_span(Span::at(path.to_string(), line as u32))
                .with_field("hazard", "profile"),
            );
        }
    }
    out
}

/// Everything before a `--` comment.
///
/// Crude on purpose: a `--` inside a string literal ends the code early, which
/// can only ever cause a hazard to be *missed* on that line, never invented.
/// The alternative is a Lua parser, and a lint nobody can read the source of
/// is a lint nobody trusts.
fn strip_comment(line: &str) -> String {
    match line.find("--") {
        Some(i) => line[..i].to_string(),
        None => line.to_string(),
    }
}

/// Replace the contents of string literals with spaces.
///
/// Because `fx.parse("0.1")` is the *correct* way to get an exact tenth, and
/// flagging the digits inside those quotes would fire on the very pattern this
/// lint exists to recommend. A lint that cries wolf on the right answer is a
/// lint people turn off.
///
/// Lengths are preserved so the column of anything found afterwards is still
/// right.
fn blank_strings(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in code.chars() {
        match quote {
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
                out.push(c);
            }
            Some(q) => {
                if escaped {
                    escaped = false;
                    out.push(' ');
                } else if c == '\\' {
                    escaped = true;
                    out.push(' ');
                } else if c == q {
                    quote = None;
                    out.push(c);
                } else {
                    out.push(' ');
                }
            }
        }
    }
    out
}

/// Whether a line calls `pairs`, as opposed to `ipairs` or a name ending in it.
fn mentions_pairs(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut from = 0;
    while let Some(found) = code[from..].find("pairs") {
        let at = from + found;
        let before = at.checked_sub(1).map(|i| bytes[i] as char);
        // `ipairs` and `my_pairs` are not the hazard.
        let is_word_start = !matches!(before, Some(c) if c.is_alphanumeric() || c == '_');
        let after = code[at + 5..].trim_start().starts_with('(');
        if is_word_start && after {
            return true;
        }
        from = at + 5;
    }
    false
}

/// A decimal literal with a fractional part, if the line has one.
///
/// Whole numbers are exempt: `self.hp = 20` is exact, and flagging it would
/// make the lint useless noise. What is caught is the fraction, which is where
/// f64 stops being exact.
fn float_literal(code: &str) -> Option<String> {
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let before = start.checked_sub(1).map(|j| bytes[j] as char);
            // Part of an identifier like `vec2`, or a field like `a.b2`.
            let in_name = matches!(before, Some(c) if c.is_alphanumeric() || c == '_');
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let text = &code[start..i];
            if !in_name && text.contains('.') && text.ends_with(|c: char| c.is_ascii_digit()) {
                return Some(text.to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Whether a line looks like it writes simulation state.
///
/// `self.x = ...`, a node handle's field, or `set_var`. Deliberately shallow:
/// the point is to notice a hazard and a write on the same line, not to prove
/// one flows into the other.
fn writes_state(code: &str) -> bool {
    let Some(eq) = find_assignment(code) else {
        return false;
    };
    let left = code[..eq].trim();
    left.starts_with("self.")
        || left.contains(":set")
        || (left.contains('.') && !left.starts_with("local "))
}

/// The index of a plain `=`, skipping `==`, `<=`, `>=`, `~=`.
fn find_assignment(code: &str) -> Option<usize> {
    let bytes = code.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'=' {
            continue;
        }
        let prev = i.checked_sub(1).map(|j| bytes[j]);
        let next = bytes.get(i + 1).copied();
        if matches!(prev, Some(b'=' | b'<' | b'>' | b'~')) || next == Some(b'=') {
            continue;
        }
        return Some(i);
    }
    None
}
