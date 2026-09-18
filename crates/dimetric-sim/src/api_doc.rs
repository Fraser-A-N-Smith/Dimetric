//! What the Lua sandbox offers, as data.
//!
//! `docs/API.md` exists because I10 says the bindings are documented, and the
//! command, kind and diagnostic tables in it are generated from the code so
//! they cannot drift. The Lua table was not: it was a string literal in
//! `xtask`, and it drifted — M12 added a whole `ui` global and the reference
//! went on listing ten of the eleven, which is the failure mode I10 is about.
//!
//! So the table is this manifest now, and [`crate::script::sandbox_globals`]
//! reports what a real sandbox actually registers. The test that compares the
//! two is the part that matters: a new global with no entry here fails, and an
//! entry here for a global that was removed fails too. A literal cannot do
//! that, which is the whole argument for generating.

/// One global in the sandbox.
pub struct Global {
    /// The name a script sees.
    pub name: &'static str,
    /// What it gives you, as the reference renders it.
    pub about: &'static str,
}

/// Every global the sandbox installs, in the order the reference lists them.
///
/// Engine globals only: the Lua standard names the sandbox re-exports
/// (`ipairs`, `string`, a reduced `math` and so on) are listed separately in
/// the prose, because they are borrowed rather than offered.
pub const GLOBALS: &[Global] = &[
    Global {
        name: "scene",
        about: "`find(path)`, `by_id(id)`, `tagged(tag)`, `near(at, radius, tag)`, \
                `nearest(at, radius, tag)`, `spawn(prefab, at, parent)`, \
                `request_load(path, carry)`, `carry()`",
    },
    Global {
        name: "input",
        about: "`move()`, `aim()`, `aim_vector()`, `held(button)`, `pressed(button)`, \
                `released(button)`",
    },
    Global {
        name: "tiles",
        about: "`get(layer, x, y)`, `set(layer, x, y, tile)`, `fill(layer, x, y, w, h, tile)`, \
                `bounds(layer)` — writes land at the end of the tick",
    },
    Global {
        name: "ui",
        about: "`hovered(node)`, `pressed(node)`, `clicked(node)`, `captured()`, \
                `pointer()`, `focused()`, `focus(node)`, `focus_next(step)`, `rect(node)`, `measure(font, text)`",
    },
    Global {
        name: "event",
        about: "`emit(kind, payload)` — tells the host something. Drained by the \
                runtime, **never** hashed",
    },
    Global {
        name: "profile",
        about: "`get(key)`, `put(key, value)`, `clear(key)` — across runs, and \
                **never** in the state hash",
    },
    Global {
        name: "camera",
        about: "`to_world(canvas_point)`, `to_canvas(world_point)`, `center()` — the \
                view\'s inverse, in fixed point",
    },
    Global {
        name: "tick",
        about: "`count()`, `dt()`, `rate`",
    },
    Global {
        name: "rng",
        about: "`range(stream, lo, hi)`, `chance(stream, n, d)`, `unit(stream)`",
    },
    Global {
        name: "vec2",
        about: "`vec2(x, y)`, building a fixed-point vector",
    },
    Global {
        name: "fx",
        about: "`new`, `parse`, `sin`, `cos`, `from_angle`",
    },
    Global {
        name: "log",
        about: "`info`, `warn`, `error` — collected per tick, never hashed",
    },
    Global {
        name: "tween",
        about: "`to(node, property, target, ticks, easing)`, `cancel(node, property)`, \
                `running(node, property)`",
    },
    Global {
        name: "anim",
        about: "`play(node, clip)`, `stop(node)`, `frame(node)`, `playing(node)`, \
                `finished(node)`",
    },
    Global {
        name: "require",
        about: "`require(path)`, another script's returned table",
    },
];

/// The Lua standard names the sandbox re-exports, for the test to subtract.
///
/// `_G` is in here because the sandbox points it at the environment itself, so
/// a script that enumerates its own globals finds it.
///
/// `unpack` is *not*, and was the first thing the cross-check found: the
/// sandbox listed it, Lua 5.4 moved it to `table.unpack`, and the copy loop
/// skips a name the host does not have — so it had been asking for a function
/// that was not there and silently getting nothing. Scripts reach it through
/// `table` either way.
pub const BORROWED: &[&str] = &[
    "_G",
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "math",
    "next",
    "pairs",
    "pcall",
    "rawequal",
    "rawget",
    "rawlen",
    "select",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "xpcall",
];
