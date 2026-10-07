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
//! **A file-scope `local` written inside a hook.** Script state lives in Rust
//! so that `self.hp` survives a snapshot; a Lua local does not, and a host that
//! starts running over a state it did not create has never run `on_ready` for
//! those nodes. Suspending and resuming a run makes that reachable from a
//! game, and a rollback and a hot reload already did.
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
/// The comment that says a Lua local is rebuilt rather than restored.
pub const TRANSIENT_MARK: &str = "@transient";

/// Scan one script for determinism hazards.
///
/// `path` is only used for the diagnostics' spans.
pub fn check(path: &str, source: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Locals that hold a profile value. The check used to look for a read and
    // a state write on the *same line*, which catches
    //
    //     self.spell = profile.get("knows_fire")
    //
    // and misses the two-line form anybody would actually write:
    //
    //     local known = profile.get("knows_fire")
    //     self.spell = known
    //
    // A name is tainted when it is assigned from a profile read, and stays
    // tainted for the rest of the file. Crude in the same direction as the
    // rest of this lint: it would rather name a safe write than miss an
    // unsafe one.
    let mut tainted: std::collections::BTreeSet<String> = Default::default();
    // Names the file declares at its own scope. Whatever a hook assigns to one
    // of these is gone the moment a host starts over a state it did not build,
    // so the set has to be known before the hooks are read.
    // What is open where, so a write can be told from a field and a hook from
    // the module's own body. See [`scopes`].
    let scopes = scopes(source);
    let file_locals = file_scope_locals(source, &scopes);
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

        // A local taking a profile value becomes tainted.
        if code.contains("profile.get") {
            if let Some(name) = assigned_local(&code) {
                tainted.insert(name);
            }
        }

        if let Some(name) = rebound_file_local(&code, &file_locals, scopes.get(index)) {
            if !acknowledged(TRANSIENT_MARK) {
                out.push(
                    Diagnostic::new(
                        Code::SCRIPT_LUA_STATE,
                        format!(
                            "`{name}` is a local at the file's own scope, so what this \
                             writes to it lives in Lua rather than in the state. A host \
                             that restores a run it did not play — a resume, a rollback, \
                             a hot reload — starts from a fresh environment and has not \
                             run `on_ready` for these nodes, so the value is gone and the \
                             next use of it raises. Keep it in `self`, or look it up \
                             again in the hook that needs it, or write `-- @transient` if \
                             it is rebuilt every tick."
                        ),
                    )
                    .with_span(Span::at(path.to_string(), line as u32))
                    .with_field("hazard", "lua-state")
                    .with_field("variable", name),
                );
            }
        }

        if let Some(variable) = lost_table_write(&code) {
            out.push(
                Diagnostic::new(
                    Code::SCRIPT_LOST_WRITE,
                    format!(
                        "`{variable}` is read back as a fresh copy of script state, so this \
                         write lands in a temporary and is dropped. Read it into a local, \
                         change that, and assign the local back: `local t = self.{variable}` \
                         / `t.field = …` / `self.{variable} = t`."
                    ),
                )
                .with_span(Span::at(path.to_string(), line as u32))
                .with_field("hazard", "lost-table-write")
                .with_field("variable", variable),
            );
        }

        if (code.contains("profile.get") || reads_tainted(&code, &tainted)) && writes_state(&code) {
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
/// The name a `local x = ...` or plain `x = ...` binds, when the line binds one.
///
/// Only bare names: a `self.x` or a `t.y` on the left is a state write rather
/// than a local, and is the thing being looked for elsewhere.
fn assigned_local(code: &str) -> Option<String> {
    let eq = find_assignment(code)?;
    let mut left = code[..eq].trim();
    if let Some(rest) = left.strip_prefix("local ") {
        left = rest.trim();
    }
    // `local a, b = ...` binds two names and this lint is not a parser. Taint
    // both rather than neither.
    if left.contains(',') {
        return None;
    }
    if left.is_empty() || left.contains('.') || left.contains(':') || left.contains('[') {
        return None;
    }
    if !left.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(left.to_string())
}

/// Does the right-hand side mention a name that holds a profile value?
fn reads_tainted(code: &str, tainted: &std::collections::BTreeSet<String>) -> bool {
    let Some(eq) = find_assignment(code) else {
        return false;
    };
    let right = &code[eq + 1..];
    tainted.iter().any(|name| mentions_word(right, name))
}

/// A whole-word search, so `known` does not match `unknown`.
fn mentions_word(haystack: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        let before_ok = start == 0
            || !haystack.as_bytes()[start - 1].is_ascii_alphanumeric()
                && haystack.as_bytes()[start - 1] != b'_';
        let after_ok = end >= haystack.len()
            || !haystack.as_bytes()[end].is_ascii_alphanumeric()
                && haystack.as_bytes()[end] != b'_';
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn writes_state(code: &str) -> bool {
    let Some(eq) = find_assignment(code) else {
        return false;
    };
    let left = code[..eq].trim();
    left.starts_with("self.")
        || left.contains(":set")
        || (left.contains('.') && !left.starts_with("local "))
}

/// The variable in `self.<name>.<field> = …`, which is a write that vanishes.
///
/// A script variable holding a map or a list is converted to a *fresh* Lua
/// table on every read, so a field written through one lands in a temporary and
/// is dropped. It used to be the only failure in the engine that carried nothing
/// at all — no error, no warning, no lint — which is what I9 forbids.
///
/// It is caught here rather than at the write because neither runtime guard
/// works, and both failures are recorded as tests in
/// `tests/copied_table_facts.rs`. A metatable on the table sees only keys that
/// are *absent*, so it would catch `self.bag.b = 2` and miss
/// `self.run.pending.at = 2` where `at` is already there. An empty proxy would
/// see every write, but the host's own table walk is `lua_next` and ignores
/// `__pairs`, so assigning the proxy back would quietly empty the variable.
///
/// A text scan has no such hole, because the difference is syntactic: this
/// matches a write *through* a chained access, and leaves alone the local that
/// is assigned back, which is the pattern that works and that the engine's own
/// example game uses.
fn lost_table_write(code: &str) -> Option<String> {
    let eq = find_assignment(code)?;
    let left = code[..eq].trim();
    // `local x.y = ...` is not Lua; a declaration cannot have a path in it.
    let rest = left.strip_prefix("self.")?;
    // Two or more levels of access past `self`: `self.a.b`, `self.a[i]`,
    // `self.a.b.c`. One level — `self.a` — is the write that works.
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || name.len() == rest.len() {
        return None;
    }
    // Whatever follows the name has to be a field or an index, not a call or
    // a comparison that happened to sit left of an `=`.
    let next = rest[name.len()..].chars().next()?;
    match next {
        '.' | '[' => Some(name),
        _ => None,
    }
}

/// The index of a plain `=`, skipping `==`, `<=`, `>=`, `~=`.
/// Names declared `local` at the file's own scope.
///
/// Column zero is the test, which is what a text scan can see: Lua's scoping is
/// lexical and a `local` inside a function is indented by every style anybody
/// writes, including this repository's own scripts. A `local function` is left
/// out — rebinding one is pathological rather than a cache, and naming it would
/// be noise.
fn file_scope_locals(source: &str, scopes: &[LineScope]) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for (index, raw) in source.lines().enumerate() {
        // Nothing open at all, which is stricter than "not indented": a `local`
        // inside a file-scope `for` belongs to the loop, and naming it here
        // would make the loop's own temporaries look like module state.
        if !scopes.get(index).is_some_and(|s| s.file_scope) {
            continue;
        }
        let code = blank_strings(&strip_comment(raw));
        let code = code.trim_start().to_string();
        let Some(rest) = code.strip_prefix("local ") else {
            continue;
        };
        if rest.trim_start().starts_with("function") {
            continue;
        }
        // `local a, b = …` declares both, and the names stop at the `=`.
        let declared = match find_assignment(rest) {
            Some(eq) => &rest[..eq],
            None => rest,
        };
        for name in declared.split(',') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// The name in `thing = …` inside a hook, where `thing` is a file-scope local.
///
/// "Inside a hook" is the whole question, and [`scopes`] answers it: a write in
/// a function body is lost when a host restores a run it did not play, and a
/// write at load time — the module's own body, or a file-scope `for` or `if` —
/// is re-established by the require that loaded the scripts. The test used to
/// be "is the line indented", which read a file-scope loop as a hook.
///
/// A write *through* the local counts: `M.count = 1` and `seen[k] = true` keep
/// their value in Lua exactly as the binding does.
fn rebound_file_local(
    code: &str,
    file_locals: &std::collections::BTreeSet<String>,
    scope: Option<&LineScope>,
) -> Option<String> {
    // Inside a function body, which is the only place a write to one of these
    // is lost. A write at load time — in the module's own body, or in a
    // file-scope `for` or `if` — runs again on every require, which is what
    // makes a file-scope local safe in the first place.
    let (eq, enclosing) = scope?.assignment?;
    if enclosing != Enclosing::Function {
        return None;
    }
    let target = code.get(..eq)?.trim();
    if target.starts_with("local ") {
        return None;
    }
    // The root of the target, so a write *through* the local counts too: a
    // table's field lives in Lua exactly as the binding does, and a hook that
    // sets `M.count` loses it on a restore for the same reason.
    let name: String = target
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || !file_locals.contains(&name) {
        return None;
    }
    // `name`, `name.field` or `name[key]`, and nothing else. A target this does
    // not recognise is one this scan has no business guessing at.
    match target[name.len()..].chars().next() {
        None | Some('.') | Some('[') => Some(name),
        _ => None,
    }
}

/// What encloses a point in the source, as far as an assignment cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Enclosing {
    /// Nothing, or only load-time blocks — a file-scope `for`, `if` or `do`.
    ///
    /// Code here runs when the module is required, in whatever environment is
    /// requiring it, which is exactly what makes a file-scope local assigned
    /// here safe: a host that restores a run it did not play loaded the scripts
    /// first, so the assignment has already happened again.
    LoadTime,
    /// A table constructor. A `name = value` here is a **field**, and a field
    /// whose key matches a module's name is the commonest thing in Lua.
    Constructor,
    /// A function body. A `name = value` here runs when the hook does, and a
    /// file-scope local written here is state kept in Lua.
    Function,
}

/// One line's answer, from the walk below.
#[derive(Clone, Copy, Debug)]
struct LineScope {
    /// Nothing at all is open at the start of this line, so a `local` here
    /// declares a name at the file's own scope.
    file_scope: bool,
    /// The first assignment `=` on the line that is not a constructor field,
    /// as a byte offset into the line's blanked code, and what encloses it.
    assignment: Option<(usize, Enclosing)>,
}

/// What is open where, line by line.
///
/// This replaced two heuristics that were each wrong in one direction, and the
/// reason it is a stack rather than a third heuristic is that the two
/// directions are the same question asked twice.
///
/// **Indentation meant "inside a function".** It does not. These lines are
/// indented and run at *load* time, inside a file-scope loop:
///
/// ```lua
/// local BY_FAMILY = {}
/// for i = 1, #M.ORDER do
///   M.ELITE_OF[M.ORDER[i]] = "elite"
///   BY_FAMILY[i] = {}
/// end
/// ```
///
/// Each builds a constant from data every time the module is required, which is
/// the safe case by the same argument that makes `local M = {}` safe. One
/// game's project reported six warnings of this shape.
///
/// The round before had cleared eleven warnings in that same project by
/// skipping constructor fields, and said all eighteen it reported were that
/// shape. They were not — these six and the one below were the rest, and the
/// claim was corrected rather than left standing.
///
/// **Brace depth measured at the start of a line** missed a constructor that
/// opens mid-line, so the second line of this call was read as a write to `M`:
///
/// ```lua
/// queue_fx(m, now, "prefabs/fx_sigil",
///          M.world_of(cell_w, cell_h, g, a.cell), INK_MINE, 14, { fade = 10 })
/// ```
///
/// The `=` belongs to `{ fade = 10 }`. Asking what encloses the `=` at *its own
/// position* answers that exactly, and the line above it — the same call, the
/// same constructor — went unflagged only because it did not happen to start
/// with a module's name.
///
/// It also closes the blind spot the previous round wrote down: a function
/// literal inside a constructor holds statements, and the innermost grouping
/// there is the function rather than the braces.
///
/// # What it tracks, and what it does not
///
/// Blocks (`function`, `if`, `for`, `while`, `do`, `repeat`), table
/// constructors, short strings, line comments and long brackets — `[[ ]]`,
/// `--[[ ]]` and their `[==[` forms — because a commented-out `function` or a
/// brace in a long string would otherwise unbalance the stack for the rest of
/// the file, and a lint that silently switches itself off part-way through a
/// file is worse than no lint.
///
/// It is not a Lua parser and answers one question: of the groupings that can
/// hold an assignment, which is innermost. Everything else about the grammar —
/// operator precedence, scoping rules, whether a name exists — is none of its
/// business.
fn scopes(source: &str) -> Vec<LineScope> {
    /// A thing closed by `end`, `until` or `}`.
    enum Open {
        Constructor,
        Function,
        /// `if`, `for`, `while`, `do` — closed by `end`, and transparent here:
        /// an assignment inside one is enclosed by whatever encloses it.
        Block,
        /// `repeat`, closed by `until`.
        Repeat,
    }

    let innermost = |stack: &[Open]| {
        stack
            .iter()
            .rev()
            .find_map(|open| match open {
                Open::Constructor => Some(Enclosing::Constructor),
                Open::Function => Some(Enclosing::Function),
                _ => None,
            })
            .unwrap_or(Enclosing::LoadTime)
    };

    let mut out = Vec::with_capacity(source.lines().count());
    let mut stack: Vec<Open> = Vec::new();
    // The `do` of a `for … do` or `while … do` opens nothing of its own.
    let mut expect_do = false;
    // `Some(level)` while inside `[=*[ … ]=*]`, whether comment or string.
    let mut long: Option<usize> = None;

    for raw in source.lines() {
        let line: Vec<char> = raw.chars().collect();
        let mut scope = LineScope {
            file_scope: long.is_none() && stack.is_empty(),
            assignment: None,
        };
        // Byte offsets, because that is what the caller indexes the line's
        // blanked code with; the two strings are the same characters.
        let mut byte = 0usize;
        let mut i = 0usize;

        while i < line.len() {
            let c = line[i];
            let width = c.len_utf8();

            if let Some(level) = long {
                if let Some(after) = long_bracket_close(&line, i, level) {
                    long = None;
                    byte += line[i..after].iter().map(|c| c.len_utf8()).sum::<usize>();
                    i = after;
                    continue;
                }
                byte += width;
                i += 1;
                continue;
            }

            // A line comment, unless it opens a long one.
            if c == '-' && line.get(i + 1) == Some(&'-') {
                match long_bracket_open(&line, i + 2) {
                    Some((level, after)) => {
                        long = Some(level);
                        byte += line[i..after].iter().map(|c| c.len_utf8()).sum::<usize>();
                        i = after;
                        continue;
                    }
                    None => break,
                }
            }
            if let Some((level, after)) = long_bracket_open(&line, i) {
                long = Some(level);
                byte += line[i..after].iter().map(|c| c.len_utf8()).sum::<usize>();
                i = after;
                continue;
            }
            if c == '"' || c == '\'' {
                let mut j = i + 1;
                while j < line.len() {
                    if line[j] == '\\' {
                        j += 2;
                        continue;
                    }
                    if line[j] == c {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                let stop = j.min(line.len());
                byte += line[i..stop].iter().map(|c| c.len_utf8()).sum::<usize>();
                i = stop;
                continue;
            }

            if c.is_alphanumeric() || c == '_' {
                let mut j = i;
                while j < line.len() && (line[j].is_alphanumeric() || line[j] == '_') {
                    j += 1;
                }
                let word: String = line[i..j].iter().collect();
                match word.as_str() {
                    "function" => stack.push(Open::Function),
                    "if" => stack.push(Open::Block),
                    "for" | "while" => {
                        stack.push(Open::Block);
                        expect_do = true;
                    }
                    "do" => match expect_do {
                        true => expect_do = false,
                        false => stack.push(Open::Block),
                    },
                    "repeat" => stack.push(Open::Repeat),
                    "end" => {
                        // Lenient about a stack that has run out: a file this
                        // scan has misread should stop reporting rather than
                        // start reporting nonsense.
                        if matches!(stack.last(), Some(Open::Function) | Some(Open::Block)) {
                            stack.pop();
                        }
                    }
                    "until" => {
                        if matches!(stack.last(), Some(Open::Repeat)) {
                            stack.pop();
                        }
                    }
                    _ => {}
                }
                byte += word.len();
                i = j;
                continue;
            }

            match c {
                '{' => stack.push(Open::Constructor),
                '}' => {
                    if matches!(stack.last(), Some(Open::Constructor)) {
                        stack.pop();
                    }
                }
                '=' => {
                    let prev = i.checked_sub(1).map(|j| line[j]);
                    let next = line.get(i + 1).copied();
                    let comparison =
                        matches!(prev, Some('=' | '<' | '>' | '~')) || next == Some('=');
                    if !comparison {
                        let here = innermost(&stack);
                        // A field's `=` is skipped rather than ending the
                        // search, so `f({ a = 1 }); M.x = 2` is still seen.
                        if here != Enclosing::Constructor && scope.assignment.is_none() {
                            scope.assignment = Some((byte, here));
                        }
                    }
                }
                _ => {}
            }
            byte += width;
            i += 1;
        }

        out.push(scope);
    }
    out
}

/// A long bracket opening at `i`, as `(level, index just past it)`.
///
/// `[[` is level 0 and `[==[` is level 2. The level has to come back out so the
/// close can be matched to it: `]]` does not end a `[==[`.
fn long_bracket_open(line: &[char], i: usize) -> Option<(usize, usize)> {
    if line.get(i) != Some(&'[') {
        return None;
    }
    let mut j = i + 1;
    let mut level = 0;
    while line.get(j) == Some(&'=') {
        level += 1;
        j += 1;
    }
    match line.get(j) == Some(&'[') {
        true => Some((level, j + 1)),
        false => None,
    }
}

/// The index just past a long bracket of `level` closing at `i`, if one does.
fn long_bracket_close(line: &[char], i: usize, level: usize) -> Option<usize> {
    if line.get(i) != Some(&']') {
        return None;
    }
    let mut j = i + 1;
    let mut seen = 0;
    while line.get(j) == Some(&'=') {
        seen += 1;
        j += 1;
    }
    match seen == level && line.get(j) == Some(&']') {
        true => Some(j + 1),
        false => None,
    }
}

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
