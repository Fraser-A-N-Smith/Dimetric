# Contributing to Dimetric

Patches are welcome. Read this first — the scope boundaries below are not
negotiable, and a PR that crosses one will be closed regardless of quality.

## Non-goals

These are permanently out of scope:

- 3D or 2.5D rendering
- Web, WASM, mobile, or console targets
- Side-scrolling and platformer physics
- General-purpose rigid-body dynamics
- Visual scripting
- A custom UI toolkit
- Custom asset formats where standards already exist
- An asset store
- Networking transport

Each of these is plausible, and each is a plausible way to never finish. The
list is written down so that saying no costs nothing later.

Rollback netcode is a partial exception: the engine is *architected* so it
stays reachable (pure sim, snapshot/restore, abstracted input), but no
transport, prediction, or lobby code belongs here yet.

## Invariants

Ten rules hold everywhere in the engine. They are numbered so CI failures and
code comments can cite them, and several are checked mechanically.

| # | Rule | Checked by |
|---|---|---|
| I1 | Every editor operation exists as a `Command` first | review |
| I2 | Scenes are text, and load-then-save is byte-identical | `dim scene fmt --check`, round-trip tests |
| I3 | No `f32`/`f64` in simulation code | `cargo xtask lint-floats` |
| I4 | No `HashMap`/`HashSet` iteration in simulation | `cargo xtask lint-floats` |
| I5 | No wall-clock time in simulation | review + lint |
| I6 | All randomness comes from a seeded `Rng` stream | review |
| I7 | Rendering never writes simulation state | review |
| I8 | Sim is `(State, Inputs) -> State` | review |
| I9 | Every failure carries a code, a location, and a payload | `Diagnostic` type |
| I10 | Lua bindings and commands are versioned and documented | generated `docs/API.md` |

If you think an invariant is wrong, open an issue. Do not work around one in a
PR — a quiet deviation in determinism or the command bus costs weeks to unwind.

## Before you push

```sh
cargo xtask ci
```

That runs the same gates CI does: formatting, clippy, tests, the float and
`HashMap` lints, the crate dependency-direction check, and scene canonical-form
checking. It is faster to run it than to wait for the robot to tell you.

## Determinism

The most common way to break this engine is to introduce a float, a
`HashMap` iteration, or a wall-clock read into a simulation path. None of these
fail loudly. They fail three weeks later as a replay that diverges at tick
4,117 on someone else's machine.

If you touch `dimetric-core`, `dimetric-scene`, or `dimetric-sim`, add a replay
fixture under `tests/replay/` covering the behaviour you changed.

## Commits

Sign off your commits (`git commit -s`). We use DCO, not a CLA — there is no
paperwork and no copyright assignment.

Commit messages: imperative mood, a short subject, and a body explaining why if
the diff does not make it obvious.

## Licence

Contributions are dual-licensed under MIT and Apache-2.0, matching the project.
