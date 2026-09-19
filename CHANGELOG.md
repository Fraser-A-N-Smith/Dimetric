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

### Added: `docs/API.md` says what an id looks like, and what a `.meta` does

`id` was described in the reserved-keys table as "Permanent identity, as
written in the file", which does not say what a valid one is. Anyone
hand-writing or generating a `.meta` had to read `core/src/id.rs` to find out —
which is exactly how a generator came to emit `sprites_ashfen_bogling` and have
84 sidecars overwritten.

There is an **Identifiers** section now, generated from `dimetric_core::id`
through a new `dim api ids`: a prefix, exactly eight characters, `n_` for nodes
and `a_` for assets. It also records something the report did not have, because
it is only visible in the parser: **generation and parsing use different
alphabets.** Generated ids draw from Crockford-style base32 with the
easy-to-misread characters removed; parsing accepts any of `[0-9a-z_]`. So a
hand-written id may contain an underscore. It may not be a different length,
and it may not omit the prefix — which is what the failing id actually got
wrong.

And a **`.meta` sidecar** section saying what absent and malformed now do
differently, since the fix above made them different.

Does not move the state hash.

### Fixed: a malformed `.meta` was silently replaced, destroying it

A sidecar that failed to parse had its error dropped by `.ok()`, a fresh
default invented in its place, and that default written back over the author's
file — no error, no warning, no diagnostic code, exit status zero. It ate 84
files in one project, each carrying hand-written `[[clip]]` declarations, and
the only reason nothing was lost for good is that they were in git.

The distinction the fix turns on, because the two cases wanted opposite
recoveries and shared a code path:

- **Absent** means *no opinion*. Inventing one is helpful, is documented, and
  still happens — dropping a PNG into `assets/` and getting a sidecar works
  exactly as before.
- **Present and unparseable** means *an opinion that did not survive parsing*.
  Inventing one in its place destroys it.

A sidecar that is there and does not parse now fails that asset's import with
**`DIM0602`**, naming the file and the reason. That trips the guard already
written into `write_metas`, which skips failed assets — so the overwrite stops
for free, exactly as the report predicted.

Two smaller silences fixed alongside, both found while confirming the first.
Import failures were put in the JSON and **never printed in text mode**, so the
one command that could tell you a sidecar was broken said "imported 1 asset(s)"
and said nothing else. And the count now excludes what failed, because
"imported 84 assets" next to 84 errors is a sentence that talks the reader out
of reading the errors.

A `.meta` that exists but cannot be *read* — permissions, a directory in its
place — is treated as malformed rather than absent, for the same reason:
something is there and we could not honour it.

Does not move the state hash.

### Added: `flip_h` and `flip_v` on `AnimatedSprite2D`

`Sprite2D` had both and `AnimatedSprite2D` had neither, so `flip_h = true` on
an animated node was `DIM0301` — an unknown property.

It turned out to be two schema entries and nothing else. Both kinds already go
through the same extraction, which was reading `flip_h` and swapping the UVs;
only the declaration was missing, so the renderer had supported this all along
and the schema would not let anyone ask.

Semantics match `Sprite2D` exactly. Under a 2:1 shear a grid actor's four
screen facings are two mirrored pairs, so a mirrored sheet halves the drawn art
for a directional character.

**On the hash, since the request asked:** `flip_h` is authored scene data and
the scene is hashed, so setting it moves the hash the same way `modulate` or
`visible` does. That is not the thing that would be a problem — what matters is
that *drawing* it changes nothing, and the renderer reads it and writes nothing
back (I7). A game picks its facing in a script variable, which is hashed like
any other decision, and the flip is only how that gets drawn.

**On batching, also asked:** a flip does not split a batch. It swaps UVs on the
draw item and never reaches `batch_group`, which is atlas, shader and blend, so
a row of actors facing both ways is one draw call. There is a test.

### Added: a `.meta` can name clips over a plain PNG strip

```toml
frames = 70
frame_ms = 120

[[clip]]
name = "idle_ne"
from = 0
to = 3

[[clip]]
name = "attack_ne"
from = 4
to = 9
looping = false
frame_ms = 60
```

Named clips came only from Aseprite tags, so a strip from anywhere else —
generated placeholders, procedural sheets, art a script assembles — imported as
one unnamed clip and `anim.play(node, "walk_se")` could not reach it. The
sidecar is where facts the file does not carry already live: it could say a PNG
is a strip of 70 frames and not that frames 0 to 3 are an idle.

Ranges are inclusive and absolute. A range past the end, a backwards range, a
missing name and two clips sharing a name are all refused with **`DIM0604`**,
and the out-of-range message names the last usable frame because that is
exactly the confusion. Declaring no clips keeps the old behaviour — one clip
called `default` over every frame — so every existing `.meta` is unaffected.

Two things beyond the request. `looping` and a per-clip `frame_ms`, because an
attack plays once and faster than an idle, and without them the alternative is
importing the same sheet twice.

And **overlap warns (`DIM0605`) rather than failing**, which is a deliberate
departure. Reusing frames across clips is a real technique and Aseprite tags
may overlap, so refusing would be stricter than the tool this pipeline mirrors
— but `0..3` then `3..7` when `4` was meant is the off-by-one ranges exist to
catch, so it is said out loud. `Imported` grew a `warnings` list to carry it,
and `dim asset import` reports them.

Does not move the state hash.

### Added: a game-to-host event channel (`event.emit`)

```lua
event.emit("achievement", { id = "first_light" })
event.emit("setting", { key = "volume_music", value = 70 })
```

Drained by a runtime with `Session::drain_events`. That is how a Steam
achievement fires, how a volume change reaches the mixer, and how anything else
platform-facing gets out. The sandbox keeps no `package`, no FFI and no `io`,
so the simulation still cannot reach a platform SDK — it says what happened and
the process around it decides what that means.

**Never hashed, and structurally so.** The request proposed mirroring the sound
list, which is a field on `SimState` the hasher skips. This takes the *stronger*
form the engine settled on for log lines: not on `SimState` at all, so a later
change cannot start hashing a field that does not exist.

**A rollback re-emits**, and that is the contract rather than an accident.
Events are not snapshotted, so a restore does not resurrect ones the host
already acted on, and re-running a tick emits again. A host needing
exactly-once deduplicates; Steam already does. Snapshotting them would be worse
in every case.

One-way, as asked. Each event carries its tick, so a host draining after
several ticks can still tell them apart. The payload is the `Value` scripts
already store, so lists stay ordered and there is no second serialisation.
`dim run` reports events, so an achievement that fires in a windowed build and
not in CI is visible rather than mysterious.

A tick may emit at most 4,096 events; past that it is reported (`DIM0505`)
rather than filling memory quietly. An empty `kind` is refused, because a host
routes on it.

Does not move the state hash. New fixture `tests/replay/host-events` draws from
a seeded stream immediately before and after each emit and probes both numbers,
so an `emit` that ever consumed randomness breaks a probe rather than a build
three weeks later.

### Fixed: `docs/API.md` never listed the reserved keys — and `z` did nothing

The reference referred to "the reserved `scene` key", said "No properties
beyond the reserved keys" under several kinds, and carried `DIM0104` for
shadowing one, without ever listing them. So `pos`, `visible`, `z` and `layer`
appeared in no table in the whole document.

There is a **Keys every node has** table now, generated from
`dimetric_scene::schema::RESERVED_KEY_DOCS` through a new `dim api reserved`,
with tests holding it against `RESERVED_KEYS` in both directions — the same
shape as the Lua globals table. It ends with how draw order actually sorts,
because the workaround somebody reaches for otherwise is nudging a sprite's Y
to force it in front, and that is never the answer.

**Writing that sentence turned up a real bug.** `z` was reserved, stored on
every node, settable through the command bus, visible in the inspector and
readable by a replay probe — and read by nothing. `SortKey::new` took
`(layer, depth, group, tie)` and `node.z` appeared in no call. The example
project sets it on five nodes, putting bolts at 20 over the player at 10 over
the enemies at 5, and none of it did anything. Same shape as `log.info`
collecting lines nobody printed.

`z` is in the key now, between `layer` and depth: **layer, then z, then depth,
then the batch group, then the node id.** The two authored fields win, because
a game that says a projectile draws over a corpse means it regardless of which
is further down the screen.

It cost nothing to fit. `DEPTH_BITS` was 24 and could only ever reach 16 —
depth comes from `Projection::depth_of`, which returns an `Fx`, and `Fx`
saturates at ±32,768 even for an isometric `x + y` where both terms are at the
limit. Eight bits were unreachable; `z` took those. There is a test that the
depth extremes still do not wrap, and one that the five fields still total 64.

`z` does **not** split a batch: it sits above the group in the key, but the
batcher merges *adjacent* items agreeing on atlas, blend and shader, and
sorting by `z` keeps them adjacent. Three sprites with three different `z`
values are one draw call, and there is a test.

Does not move the state hash — `z` is a node field and was always hashed with
the scene; what changed is the renderer reading it. No fixture re-recorded.

### Added: `ui.measure`, and a determinism lint for Lua

**`ui.measure(font, text)`** returns `{w, h}` in pixels, from the baked integer
metrics. It is a pure function of a font and a string, both of which the engine
already has, and it contains no layout policy — which is why it is built while
the scroll and grid containers that would use it are not. See
`docs/ENGINE-GAPS.md` for that reasoning.

**`dim script check --determinism`** scans a script for the three ways a game
on this engine breaks replay:

- `pairs()`, whose iteration order Lua does not specify.
- A fractional literal, because a Lua number is an f64.
- A `profile.get` flowing into a state write — the hazard the profile layer
  documents and cannot itself prevent.

`CONTRIBUTING.md` already makes this argument for the Rust lints and it holds
word for word here: these do not fail loudly, they fail three weeks later on
somebody else's machine. `-- @ordered` and `-- @presentation` are the escape
hatches, because a conservative lint without one gets turned off.

It is a text scan, not a type system, and is biased toward naming a safe
`pairs()` over missing an unsafe one. One false positive was worth fixing
before shipping: `fx.parse("0.1")` is the *correct* way to get an exact tenth,
and flagging the digits inside those quotes would have fired on the very
pattern the lint recommends. String literals are blanked before the float scan,
and the example project now reports clean.

New code: **`DIM0506`** (warning).

Does not move the state hash.

### Declined, with reasoning in `docs/ENGINE-GAPS.md`

**Grid pathfinding** (`grid.path`, `grid.reachable`, `grid.line`,
`grid.visible`). Close, but a pathfinder is a policy about movement rather than
a query about the world — the `opts` table is where that shows. `tiles.get` is
the read it needs and the determinism lint covers the way a hand-written A*
breaks replay.

**Scroll and grid containers, nine-patch panels.** M12 is defensible against
the "custom UI toolkit" non-goal because its subject is determinism — a click
that replays. These have no determinism content. Noted in that file: *clipping*
is the one piece worth reconsidering, and it is not the one that was asked for
hardest.

### Answered: what an idle tick costs

Measured rather than guessed, with a new
`cargo run --release -p dimetric-sim --example idle_cost`. At twenty actors an
idle tick costs 25 µs to step, 30 µs to hash and 15 µs to snapshot — 0.4% of a
60Hz budget, and ten seconds of CPU across a forty-minute run. Tick and forget.
Full table in `docs/ENGINE-GAPS.md`.

### Added: canvas-to-world unprojection (`camera.to_world`)

```lua
camera.to_world(canvas_point)   -- vec2 in world space
camera.to_canvas(world_point)   -- vec2 in canvas space
camera.center()                 -- where the current camera is looking
```

In fixed point, off the same `Projection` the renderer draws with. A copy of
this arithmetic in Lua would be a second version of the renderer's maths that
nothing keeps in sync, and when it drifted the symptom would be clicks landing
one cell off at certain camera positions — a bug that reproduces for nobody.
The round trip is tested across projections, camera positions and zooms.

Two structural consequences.

`Projection` **moved to `dimetric-core`**, because the simulation now needs it.
Its float methods did not come along: they are `dimetric_render::ProjectionRender`
now, an extension trait. The I3 lint covers `dimetric-core`, and rather than
excusing three `f32` signatures line by line, the exact maths went in the exact
crate and the presentation maths stayed at the presentation boundary. The lint
is what pointed this out.

The **render resolution is now part of the replay contract**, declared as
`[render] resolution` in `project.toml`. It had been pure presentation; it
stopped being so the moment a script could pick a world cell from a pointer,
because a wider viewport shows more world at the same zoom. There is a test
asserting the two differ, which is the justification for moving it.

**Moves the state hash** — the resolution is hashed with the canvas. All
fixtures re-recorded.

### Added: saving a run, and a profile that is nowhere near the hash

Two different things, and conflating them was the bug to avoid.

**Run state.** `dim state save --out <dir>` and `dim state load --from <dir>`.
The save is a directory of text: `scene.dim` written through the canonical
writer, so it round-trips for exactly the reason a scene file does (I2), plus
`state.toml` for everything else. A binary savefile would be the one place this
engine stops being a thing you can read a diff of.

A save carries a format version and the engine that wrote it, and a mismatch on
either is **refused** (`DIM1002`), not warned about. An input log that replays
wrong announces itself as a divergence; a save that restores wrong just keeps
playing.

The test that matters is not that the hash restores — it is that the resumed
run *continues* identically for the next five ticks, which is what says the RNG
streams are where they were rather than merely looking like it.

**Profile state.** `profile.get(key)`, `profile.put(key, value)`,
`profile.clear(key)`, stored as `profile.toml` beside the project.

It is **not a field on `SimState`**. Not one the hasher skips — absent, the way
the log lines are, because kept-out-entirely is one fewer thing to get wrong.
Two players on the same seed have different profiles, so a hashed one would
make their replays diverge for a reason that has nothing to do with the game.
It is not snapshotted either, so a rollback does not take back an award.

**The hazard this cannot fix, stated plainly.** Keeping the profile out of the
hash stops it *being* hashed. It does not stop a script reading from it and
writing what it read into state — `if profile.get(k) then self.spell = 1 end`
makes two players' simulations differ, and hashing the profile would only turn
a silent divergence into a loud one at the cost of making every replay depend
on who is playing. The rule is one the game has to follow: a profile value may
decide what is drawn, offered or unlocked, not what the simulation does. Pick a
loadout at the menu and carry it in through `scene.request_load`, where it is
hashed.

New codes: **`DIM1001`** (a save could not be read or written) and **`DIM1002`**
(a save from a different format or engine version).

Does not move the state hash.

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
