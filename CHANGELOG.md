# Changelog

Dimetric is 0.x. The version is a statement about the **formats and the API**,
not about how finished the engine is: until 1.0, a minor bump may change a
`.dim` file, a recorded log, the Lua surface or a command's output, and every
such change is written down here with what it breaks and what to do about it.
After 1.0 the deprecation cycle in §I10 becomes a public promise instead.

The thing worth knowing before reading any entry: **anything that changes what a
tick computes changes every recorded hash after it.** That is not a bug and it
is not avoidable — it is what determinism costs. Entries below say plainly
whether they move the hash, because a project with recorded runs has to
re-record them, and finding that out from a failing replay is a bad afternoon.

## Unreleased

### Added: a script can ask for a different scene (`scene.request_load`)

`ENGINE-GAPS.md` had this under *probably game-specific*, reasoning that the
sorcerer slice made a room a spawned wave rather than a loaded file. That is
right for five arena waves sharing a floor plan and does not carry to twenty
generated floors across six regions.

```lua
scene.request_load("floors/crypt", { hp = 17, depth = 2 })
scene.carry()   -- what this scene was handed when it was loaded
```

**This does not break I8.** A script *requests*; nothing is created or swapped;
the tick that asked finishes over the tree it started with. The swap happens
between ticks, in the host, which is the only layer with a filesystem. Tick N
is still `(State, Inputs) -> State` — tick N+1 simply starts from a different
state.

The tree is replaced; the **run** is not. The tick counter keeps counting and
the RNG streams keep their positions, because a roguelike on its second floor
is still in the same run — resetting the streams would generate every floor
from the same numbers. Everything derived from the old tree goes: velocities,
variables, animation, tweens, queued spawns all name nodes that no longer
exist.

So anything that must survive goes through `carry`, explicitly, as a `Value` —
hashed, snapshotted and rewindable, rather than a global outside the hash.

On the two questions the request asked to decide explicitly. **The undo stack
is untouched**, because this never reaches it: the bus is the authoring path
and a running game has no document. **A replay re-derives the load from the
script** rather than recording it as an event, because the script is already
deterministic and a recorded event could disagree with it. The limitation worth
knowing: a log carries a hash of its *starting* scene only, so a replay whose
later floors changed on disk diverges without saying which file moved.

A load request that nothing honours now raises `DIM0404` rather than sitting in
state doing nothing — `dim run`, `dim replay`, `dim state dump/hash`, the
fixture harness and the windowed player all honour it.

**Moves the state hash** — `carry` is hashed, so every recorded run changed.
All fixtures re-recorded. New fixture: `tests/replay/floor-descent`, which
descends mid-run and probes what crossed the boundary and what did not,
including that the loot stream is further along on the second floor rather than
starting again.

### Added: a tile API for scripts (`tiles`)

The sandbox had no way to read one tile. `TileLayer` stored run-length chunks,
`set_tiles` and `fill_tiles` were proper undoable commands, and all of it was
authoring-time only — so a game that builds its map from a seed could not.

```lua
tiles.get(layer, x, y)               -- tile index, 0 where nothing is painted
tiles.set(layer, x, y, tile)         -- lands at the end of the tick
tiles.fill(layer, x, y, w, h, tile)  -- lands at the end of the tick
tiles.bounds(layer)                  -- {x, y, w, h}, or nil
```

Writes are deferred like `scene.spawn`, for the same reason: a grid that
changed mid-tick would change under every script that had not run yet. The
useful consequence is that **reads need no snapshot at all** — the grid does
not change inside a tick, so a read during one is already the grid as it stood
when the tick began. `scene.near` has to snapshot a broadphase because nodes do
move mid-tick; tiles do not.

This does **not** route through the command bus, which the request expected it
to. See `docs/ENGINE-GAPS.md` — in short, the bus is the authoring path and a
running simulation has no document to keep in step. The chunk handling *is*
shared: it moved to `dimetric_scene::chunk` and the `SetTiles` command now
calls the same function, so there is one implementation rather than two.

New code: **`DIM0505`**, a script passing an argument a binding cannot accept —
a fill larger than a million cells, a tile index outside `u16`, or a node that
is not a `TileLayer`.

**Moves the state hash for any run that paints a tile**, because tiles are part
of the scene and the scene is hashed. No committed fixture changed, because
none of them painted one. New fixture: `tests/replay/generated-floor`, a floor
carved and scattered from the run seed, with probes asserting the tile counts
rather than only the hash.

### Fixed: `docs/API.md` omitted a whole Lua global

The reference said "scripts see exactly these globals and nothing else" and
listed ten. There were eleven — M12's entire `ui` surface was missing.

The cause is the interesting part. The command, kind and diagnostic tables in
`API.md` are generated from the engine, so they cannot drift; the Lua section
was a string literal in `xtask/src/gen_docs.rs`. I10 held everywhere except the
one section a script author actually reads.

It is generated now, from a manifest in `dimetric_sim::api_doc`, via a new
`dim api globals --json` — the same route the other three tables take, rather
than giving `xtask` a dependency on the engine it deliberately does not have.
What makes it stay fixed is `crates/dimetric-sim/tests/api_doc.rs`, which
introspects a real sandbox and compares it against the manifest in both
directions: a global with no entry fails, and an entry with no global fails.

That test found a second thing on its first run. The sandbox's list of Lua
standard names to re-export included `unpack`, which Lua 5.4 moved to
`table.unpack`; the copy loop skips names the host does not have, so it had
been asking for a function that was not there and silently getting nothing.
Removed — scripts reach it through `table` either way.

Also documented, because it is the whole of a separate request: **`ui.pointer()`
returns a canvas pixel, not a world position.**

Does not move the state hash.

Three milestones from the Godot-parity table, in order. Each is the core of
its milestone rather than the whole estimate — what is built and what is not is
stated per entry.

### M11: text and fonts

The engine can draw text. A font is baked at import into a glyph page and a
table of **integer** metrics; layout is integer arithmetic over those metrics,
so it is the same on every machine. Rasterisation is presentation and can come
from a third-party rasteriser; measurement cannot, because anything that
measures text — a centred label, a button sized to its caption — would
otherwise drift between machines.

A `Label` is a `Node2D`, useful before there is any UI to put it in. A font is
baked at one size; import it twice if you need two.

A font is also built in, drawn by hand at five by eight pixels, so a project
with no font asset can still print a frame counter or the message explaining
that the real font failed to load. It is authored rather than generated: at
seven pixels a rasteriser is guessing, and the first attempt closed up the bowl
of a `?` and filled in an `e`. Drawn as rows of `#` in the source, so a change
to a letter shows in a diff as that letter changing shape. `Label.font` is
optional as a result.

**Not built:** no rich text, no shaping for complex scripts, no BiDi.

### M12: UI

`Control` and `Panel` node kinds, anchors and offsets, and a screen-space
render layer the composite blends over the world unlit.

Layout runs against a fixed canvas rather than the window, and that is the
load-bearing decision: UI is clicked as well as drawn, so a layout that moved
with the window would make a click's outcome depend on the window, and two
players on different monitors would diverge. **Moves the state hash** by way of
M13's pointer field.

Interaction runs *inside the tick*, in a `UiUpdate` phase before scripts. It
would be less work to hit-test at the render boundary, where the pointer and
the rectangles both already exist — and it would mean a run replayed headlessly
had nothing to decide a click with. Press capture works the way every toolbar
has for thirty years: sliding off a button before letting go cancels the click.

`Button`, `VBox` and `HBox` are built. A button's interaction state reaches the
renderer as a node property, because `dimetric-render` does not depend on
`dimetric-sim` and should not start.

**Not built:** no themes, and no focus *traversal policy* — the engine tracks
focus and offers the order, but which key walks a menu is a game's decision.
Nothing here scrolls or clips: a list longer than its container overflows.

### Project settings

`project.toml` holds what changes the *meaning* of a recorded run: the tick
rate, the UI canvas, and (as a convenience, since it is where people will look)
the key bindings. A missing file is every default; a file that exists but is
wrong is reported, because a typo in a tick rate silently falling back to 60 is
how a project spends a week wondering why its recordings drift.

### M13: input breadth

Gamepads behind a `pad` feature, and the pointer in the input frame.

An analogue stick is the hardest device to admit to a deterministic engine, and
not because it reports floats: it is that a stick *never stops moving*, so a
naive mapping writes a new line into the input log sixty times a second while
the player sits still. A stick is quantised to one of sixteen magnitudes on an
angle from the engine's own tables — exactly representable, identical
everywhere, stable while a thumb rests on it.

The pointer is in `PlayerInput`, in whole canvas pixels, because a UI click has
to be replayable. The window's size is divided out at the boundary, so a
recorded click lands on the same button on a different monitor.

**Breaking — moves the state hash.** `PlayerInput` gained a `pointer` field, so
every recorded run's hashes change; the input log gained two columns per player.
A log written before this reads back with the pointer at zero rather than being
refused, so old recordings still replay, but their hashes will not match. All
committed fixtures were re-recorded.

Bindings come from `project.toml`. Declaring `[input]` replaces the defaults
rather than adding to them, since rebinding is the point. An unknown *action*
name is an error listing the real ones; an unknown *key* name is allowed,
because key names belong to the window library and differ by platform.

**Not built:** no touch, because there is no platform with one yet.

## 0.1.0

The first release. Everything in the design document's M0 through M10 is built,
the gates in `cargo xtask ci` are green, and the vertical slice — a five-room
sorcerer run with spells that evolve — replays to the same state hash on Linux,
macOS and Windows.

### The engine

- **Fixed point everywhere in gameplay.** `Fx` is 16.16 with exact parse and
  print; a value that is not exactly representable is refused at the boundary
  rather than rounded quietly. Trigonometry reads committed lookup tables,
  because platform `libm` implementations do not agree with each other.
- **Scenes** are `.dim`, a TOML dialect with stable node identity, prefab
  instancing with property overrides, and a canonical form that CI enforces so
  a diff only ever shows a real change.
- **A tick is a pure function of the state it starts from.** Snapshots,
  BLAKE3 state hashing, input logs, and replay that reports the first diverging
  tick and the node that caused it.
- **Physics**: a spatial hash, swept movement with sliding, triggers, one-way
  platforms and a simple push response.
- **Scripting**: Lua 5.4 through mlua, in a sandbox with no clock, no
  filesystem, no `math.random` and no transcendental functions. Script state
  lives in Rust, so hot reload keeps every variable a node had.
- **Rendering**: a wgpu backend with a sprite pass, a light pass and a
  composite, headless capture, and golden images gated on three platforms.
- **Assets**: an import pipeline with content-hashed caching, LDtk levels baked
  to chunks, texture atlases, and reload between ticks rather than inside one.
- **Audio**: buses, a voice pool with stealing, per-clip caps and tweened
  fades, driven from `Sound` nodes. The device is behind the `kira` feature.
- **An editor** whose every rule is a library that draws nothing — tree,
  inspector, viewport, console, assets, play-in-editor and a scrubber — with a
  window over it behind the `gui` feature.
- **An agent interface**: the `dim` CLI, an MCP server over the same command
  layer, and `docs/API.md` and `docs/schemas/` generated from the code so that
  people and agents read the same source of truth.
- **A runtime and packaging**: `dim-play` for a window, `dim build` to stage a
  game, `dim new` to write one to start from.

### Known and deliberate

- `dim build` stages a runnable directory and takes a `dim-play` you built for
  the target. It does not cross-compile and does not archive: producing all
  three platforms from one machine is a CI matrix, which is a packaging
  pipeline rather than an engine feature.
- The Lua call boundary, not the broadphase, is now the ceiling on script-heavy
  scenes: a `scene.nearest` that returns without looking at anything costs
  about 21 us. Measured in `docs/ENGINE-GAPS.md`, not yet addressed.
- A module's constants are frozen against writes through the table, but a
  module function that closes over a local and mutates it is out of the
  engine's reach. That is the one remaining way to hide state from the hash,
  and it is on the author.
- Two gaps stay open on purpose, argued in `docs/ENGINE-GAPS.md`: no direct
  cross-script calls, because writing into the target's own variables keeps the
  ordering explicit and survives the target dying mid-frame; and no scene
  loading from a script, because a mid-tick load would mean the tick was not a
  pure function of the state it started from.

### Migrating from a pre-release checkout

There is no released version before this one, so nothing here is a break from a
published API. These are the changes that would have broken you if you had been
tracking `main`, and all three move the state hash:

- **`require` replaces publishing constants on a node.** A script that reached
  another node for shared tables should require a module instead. A module's
  table is not simulation state, so moving constants out of nodes **changes
  every hash** — re-record your fixtures. `rawset` left the sandbox in the same
  change, because it writes past the guard that holds a module read-only.
- **`dim run --input` runs the log's full length** rather than defaulting to
  sixty ticks. A fixture recorded with the old default covered the first second
  of the run and nothing after it; re-record and the hash file gets longer.
- **`--headless` is no longer a mode switch.** It is accepted, always on, and
  `dim-play` is the window. Written-down commands keep working.

Three more changes are visible but move nothing:

- `log.info`, `log.warn` and `log.error` now actually log. They previously
  built a string and dropped it. The lines are output, not state: they are kept
  out of `SimState` entirely, so a script that logs hashes identically to one
  that does not. `dim run` prints them with the tick and the script.
- `DIM0801` no longer means "not implemented yet" — every command is
  implemented. It now reports a capability the machine is missing, such as a
  graphics adapter for a headless capture, and names which.
- **A log recorded by a different engine version now says so.** Every input log
  has always carried the version that wrote it, `DIM0703` has always been
  defined as meaning exactly that, and nothing ever compared them. It is a
  warning, not a refusal — a log from another version usually still replays,
  and refusing would make every version bump a wall. What it buys is the
  difference between "your change broke this" and "this was recorded by a
  different engine". Found while cutting this release, which is the first
  moment there were two versions to confuse.

  Two smaller things fell out of it: warnings raised during a *successful*
  replay used to be dropped on the floor, because only a failing replay
  forwarded its diagnostics; and the example's own logs now record `0.1.0`, so
  the bundled fixture does not greet you with a warning about itself.
