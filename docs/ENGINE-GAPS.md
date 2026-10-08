# Engine gaps the slice found

Building the sorcerer slice surfaced the things the engine does not do. §12's
discipline note says to log every one and build none of them mid-slice, because
roughly half turn out to be game-specific and shipping the slice is the only way
to learn which half.

Each entry says what the game wanted, what it did instead, and — the part worth
arguing about — whether it belongs in a reusable engine.

## Fixed

The first pass at the slice logged these; later passes built them, and the slice
was rewritten on top. Everything transient is now spawned — a room is a wave
rather than a file, and a run is five of them. The last entries were found later
still, by declaring node kinds, by packaging a game and listening to it, and by
coming back to close what this list had left open.

**A Lua array could not be stored.** `self.order = { "bolt", "nova" }` came back
as an empty map: the conversion only accepted string keys, so every entry was
dropped with no error at all. `Value::List` existed and nothing could produce
one. Silent data loss, and ordered lists are how a script keeps a spawn table or
an upgrade order reproducible (I4). A table mixing array entries and named keys
is now refused rather than half-kept.

**Scripts could not read input.** Input was latched into `SimState`, hashed, and
drove replays, and no binding exposed it — so a game was unplayable from Lua.
`input.move`, `input.aim`, `input.aim_vector` and `input.held` now read it,
from the simulation's state rather than a device: inside a tick there is no way
to tell a gamepad from a replay log (I8).

**`dim state dump` ignored the log's seed.** `run` and `replay` both prefer the
seed an input log carries; `dump` used its own default, so it quietly simulated
a different game. Two hours went into a "determinism bug" that was this.


**A script can now create a node.** `scene.spawn(prefab, at, parent)` returns
the id the node *will* have and creates nothing yet: a node inserted mid-tick
goes into a tree another script may be walking. It appears at the end of the
tick and gets `on_ready` on the next one. The id is derived from a counter in
the state rather than drawn from the RNG, so it is the same on every machine,
the same again after a rollback, and spawning one fewer projectile does not
shift every gameplay roll after it. The pool of 64 authored projectiles is gone.

**A project can declare its own node kinds.** §12 asks for enemy variants as
instances with property overrides. An override reaches a node's *properties*,
and a project could not declare a property of its own — so the numbers that made
a wraith a wraith had nowhere to live, and ended up in a Lua table where no
designer would look. A `kinds.toml` in the project root now declares them:
`Enemy extends Collider`, plus `max_health`, `speed` and `touch_damage`. They
validate, canonicalise, document and override like any built-in property,
because they go through the same machinery.

Declaring a kind exposed a second thing. The simulation and the renderer asked
what a node was by comparing its `kind` string, so an `Enemy` was not a
collider, was never swept, and enemies stood still while the run reported no
kills. A kind now carries the built-in it behaves as, and every one of those
comparisons goes through that instead. It is the kind of bug an extension point
has exactly once.

**A probe against a variable that does not exist used to pass a `!=` check.**
`var:missing != true` read `<not found>`, which is not `true`, so it passed and
looked like it had verified something. A probe that cannot find its field now
fails whatever it was comparing.

**A sensor moves.** An `Area` was skipped by the sweep entirely. Areas are now
moved where they were told and report what they passed through rather than
being resolved out of it, which is what a projectile wants.

**Buttons have edges.** `input.pressed` and `input.released`, over a
`previous_input` that is part of the state — a rollback that re-derived it from
the log would fire every edge again on the tick it landed on.

**A game can make a noise.** `dimetric-audio` was a complete mixer — buses, a
voice pool with stealing, per-clip caps, tweened fades, two backends — wired to
nothing at all: `Sound` was a registered node kind no code read, and a game
built on the engine was silent. It was found by packaging one and listening.

The wire is the interesting part, because the obvious way to build it is wrong.
A sound cannot be simulation state: if triggering one consumed a random number
or wrote something hashed, muting a game would change how it plays and a run
recorded with audio on would diverge from one played with it off. So the
simulation says what it *would* play, into a list cleared at the start of every
tick and never hashed, and whoever is listening reads it. M6's acceptance
criterion — a headless run with audio triggered producing the same state hash as
a windowed one — is now a test that runs the same scene twice and wipes the
sound bookkeeping from one of them.

**Scripts can ask what is nearby.** `scene.near` and `scene.nearest` read the
broadphase the simulation already builds. See the performance section below:
the first version of this was most of a frame budget on its own.

**Scripts can share a table.** The sandbox had no `require`, so the spell
definitions lived on a `Spellbook` node that published them as script variables
— which any script could read, and which put every one of those numbers into
the snapshot and the hash of every tick. `require("scripts/spellbook.lua")`
now runs that script once and hands back what it returned.

The design cost the first note worried about was hot reload, and it is real but
smaller than it looked: a script that required a module holds the table it
returned, so reloading just the file that changed would leave it reading the
previous version's constants — a hot reload that appears to work and does
nothing. Reloading anything therefore re-runs every script and drops the module
cache. Which scripts depend on which is not tracked, because re-running a
project's scripts costs nothing on a keystroke and a dependency graph is a thing
to get wrong.

What was *not* obvious is that a module has to be read-only. A module's table is
not in `SimState`: not hashed, not snapshotted, not rewound. A script that wrote
to one would have state that survived a rollback rewinding everything around it,
and the divergence would surface hours later in a replay rather than at the
write. So `require` freezes what it returns, all the way down. A metatable on
the table would not have done it — `__newindex` fires only for keys that are
*absent*, so overwriting something the module actually defines, which is the
write worth stopping, goes straight through. What comes back is an empty proxy
forwarding reads to a private copy, and `rawset` left the sandbox because it
walks past exactly that guard.

The freeze stops writes through the table. It cannot stop a module function that
closes over a local and mutates it, because Lua upvalues are not reachable from
the host — that one is on whoever writes the module, and it is now the only way
left to hide state from the hash.

Moving the spellbook off a node took the arena's tick-5 state dump from 9,621
bytes to 6,037. Constants had been a third of the state.

**`log.info` logged.** It had not been. The three functions built a string,
returned it, and dropped it; the `Vec<String>` meant to hold the lines was never
written to and never read. A documented API doing nothing is worse than a
missing one, because a script author reads the table in `API.md`, calls it, sees
no output, and goes looking for the bug in their own code. Nothing in the slice
called `log`, which is how it survived this long.

The lines are output, so they are kept out of `SimState` entirely rather than in
a field the hash skips — same argument as sounds, one fewer thing to get wrong
later. `dim run` prints them with the tick they came from, and a test pins that
a script which logs hashes identically to one that does not.

**The arena has walls.** It had none, so the player could walk out of the room
and fight nothing. Four static colliders, authored in the scene.

## Asked for by the grid roguelike, and declined

A tactical grid roguelike being built on the engine produced a list of nine
things. Four were blocking and are built; three more were built on their
merits. These are the ones that got a no, and the reasoning is here rather than
in a commit message because a decision that is not written down gets re-argued.

**Grid pathfinding, flood fill and line of sight.** `grid.path`,
`grid.reachable`, `grid.line`, `grid.visible` over a `TileLayer`.

Declined, and it was close. The case for is real: `scene.near` is already an
engine-provided spatial query on exactly this argument, the M10 measurements
show a Lua boundary crossing costs about 21 µs before the callee does anything,
and A* per monster per turn crosses it far more than the four hundred calls a
tick that produced the original 139 ms finding. Determinism cuts the same way —
a binary heap with equal keys is easy to tie-break wrongly in Lua, and a
non-deterministic path is a replay that diverges.

What decided it against was the shape of the thing rather than its cost. A
pathfinder is not a query about the world; it is a *policy* about movement. The
`opts` table in the proposal is where that shows: a per-tile cost mapping, a
budget, a blocking predicate, and before long a rule about whether diagonals
cost more, whether an occupied cell blocks, whether a door counts as passable
for a monster that can open it. Every one of those is a game's decision, and an
engine that takes them takes a view on what a tile *means* — which is the line
`TileLayer` currently does not cross and is more useful for not crossing.

`scene.near` is not the same case. It answers a question about geometry the
engine already maintains, with no parameter that encodes a rule.

So it goes in Lua, and the engine's job is to make that cheap and safe rather
than to do it. `tiles.get` is the read it needs; it now costs one boundary
crossing per cell rather than per path. `dim script check --determinism` flags
the `pairs()` that would make a frontier unordered, which is the specific way a
hand-written A* breaks replay. If this is revisited, the thing to measure first
is whether a Lua A* over `tiles.get` is actually too slow at twenty actors on a
40×40 grid — nobody has measured that, and the 21 µs figure is per *call*, not
per cell read.

**Scroll containers, grid containers and nine-patch panels.** Declined;
`ui.measure` built.

`CONTRIBUTING.md` lists "a custom UI toolkit" as a permanent non-goal and M12
then built one, which is a fair thing to point at. The distinction that makes
M12 defensible is that its subject is *determinism*: layout and hit testing run
inside the tick so that a click on a menu replays, and that is the engine's
core concern appearing in a new place. `Control`, anchors, and press capture
exist to make a click reproducible.

A scroll container, a grid container and a nine-patch have no determinism
content at all. They are layout convenience and visual polish — which is what
the non-goal names. `VBox` and `HBox` were built because a container is what
makes anchors usable at all; a third one is where it becomes a toolkit.

`ui.measure` is the exception and was built, because it is not a widget: it is
a pure function of a baked font and a string, both of which the engine already
has, and it contains no layout policy. Without it every piece of text in a game
is sized by guessing, and the metrics are baked integers precisely so that
measuring is exact.

One piece of this is worth reconsidering if it comes back, and it is not the
one that was pushed hardest. **Clipping** is a renderer capability a game
genuinely cannot build from Lua: a scroll offset is already expressible today
(a `VBox`'s `offset_top` is a control property, in state, and replays), so what
is actually missing from a scrollable list is a scissor rect. That is a small,
bounded thing with no layout policy in it. A `ScrollContainer` node is not.

## Open: probably game-specific

**No way for one script to call a function on another.** Damage is written into
the target's own variables (`other.pending_damage = ...`) and read on its next
tick. That is arguably better than a direct call — it keeps the ordering
explicit and survives the target being destroyed mid-frame — so this is probably
not a gap at all. Shared *behaviour*, as opposed to a message, is what modules
are for now: a function two scripts both need goes in one and is required by
both.

**No scene loading from a script.** ~~Still true, and it turned out not to
matter.~~ **Now built** — see the changelog. The reasoning below was right about
the slice and did not carry to a game with twenty generated floors across six
regions, which is what a later request pointed out.

The I8 objection was aimed at loading *mid-tick* and remains correct: a script
requests, the tick finishes over the tree it started with, and the swap happens
between ticks. What the original entry got right is that `scene.spawn` was the
useful half at the time, and the phase-boundary reasoning is exactly what the
load reuses.

> A room is a wave the arena spawns, not a file it loads, so a run of five
> rooms lives in one scene. Loading a scene mid-tick would mean the tick was not
> a pure function of the state it started from (I8).

## Not a gap, but worth writing down

Fixed-point exactness caught a mistake that would otherwise have been a
heisenbug: an input log with `0.4` in it is refused (`DIM0703`) because 0.4 is
not exactly representable. The instinct is to call that pedantic. It is the
reason a recorded run reproduces.

## What an idle tick costs

A turn-based game spends most of its wall-clock time waiting for a person to
decide while the engine ticks at 60Hz regardless. A forty-minute run is roughly
144,000 ticks, the overwhelming majority changing nothing, and every figure in
the section below is about a *busy* tick. So the question was asked, and here
is the answer: **tick and forget.**

`cargo run --release -p dimetric-sim --example idle_cost`, on an idle scene:

| Actors | step | hash | snapshot | all three |
|---|---|---|---|---|
| 20 | 25 µs | 30 µs | 15 µs | 69 µs |
| 100 | 99 µs | 139 µs | 70 µs | 307 µs |

At twenty actors — roughly the density that game describes — that is 0.4% of a
60Hz frame budget, and **ten seconds of CPU spread across a forty-minute run**.
At a hundred actors it is 1.8% and forty-four seconds. Neither is worth a
mechanism.

Two things make the real figure smaller still. A *playing* session snapshots
(for interpolation) but does not hash; hashing is what `dim run --record` and
the replay harness do. And the numbers above are for a scene where nothing is
happening, which is the case being asked about.

On the log: one player at 144,000 ticks is about **4.5 MB** of text, which gzips
to roughly an eighth — the committed fixtures compress 8:1. Replaying it costs
step plus hash, so about eight seconds at twenty actors. Both are fine; a log
that size is worth compressing on disk and is not worth a binary format.

The third part of the question answers itself from the first two. A
session-level "this tick is quiescent" signal would be a determinism hazard if
the game decided it, and at 0.4% of a frame there is nothing to buy. It has not
been considered because nothing has needed it, and these numbers say nothing
will.

## Performance, measured

§12 asks for hundreds of projectiles against dozens of enemies. `examples/sorcerer/stress.dim`
is that, as a scene rather than a claim: 400 live projectiles, 40 enemies, about
1,300 nodes, every one of the projectiles homing.

The first measurement was **139 ms a tick** — eight times a frame budget. Almost
all of it was `scene.nearest`: without homing the same scene cost 9.7 ms.

Three things were wrong with it, and they came off in order:

| Change | ms/tick |
|---|---|
| As first written | 139 |
| `within` sorts by id bytes rather than allocating a `String` per result | 71 |
| `nearest` scans for a minimum instead of building a sorted list and taking its head | 29 |
| Bodies carry a bloom filter over their tags, so a tagged query skips most candidates on an integer test | 20 |

Seven times faster, and still above a 60Hz budget at that density — the slice
itself peaks around 35 live projectiles and costs well under a millisecond. A
per-tag index would take the next chunk; it is not built, because the game does
not need it yet and the stress scene now exists to tell us when it does.

The column is one afternoon on one machine, so read the ratios rather than the
absolutes: the same scene measured again later, over runs of six hundred ticks,
comes in at 12.7 ms.

The lesson is the one the stress scene was for. `sort_by_key(|u| u.body().to_string())`
looks like nothing. At four hundred calls a tick it was half the frame.

### The pass that finished it, and what it found instead

M10 came back to the two things left open: the broadphase and the batcher.

**The broadphase.** A tagged query now walks an index holding only that tag,
rather than the main grid with a bloom filter over each candidate. `cargo bench
-p dimetric-sim --bench queries` measures it on the stress scene's shape — forty
enemies among four hundred projectiles, sharing cells:

| Query | Main grid + bloom | The tag's own index |
|---|---|---|
| `nearest`, tagged | 5.47 us | 1.92 us |
| `near`, tagged | 9.78 us | 2.30 us |

Two and a half to four times faster, against 71 us to rebuild every index and
two rebuilds a tick. At four hundred queries a tick that trades 0.14 ms for
1.4 ms. The index also lets the query answer on its own: confirming a tag used to
mean a scene lookup and a string compare per candidate, and tags are interned
now, so it is an integer.

**And it does not show up end to end.** The stress scene measures the same
either way through Lua. That is the real finding of this pass: a `scene.nearest`
that returns without looking at anything still costs about 21 us, so the
boundary the query sits behind costs more than the query. The broadphase is no
longer where the time goes, and two changes in a row measuring as noise is what
it took to notice — which is why `benches/queries.rs` exists and measures the
query on its own rather than through a script.

**The batcher.** §11 asks for thousands of sprites in a handful of draw calls.
The stress scene drew 440 sprites in **27** draw calls, which is neither.

The sort key had a sixteen-bit texture field in it, and the field was always
zero: everything is in one atlas, so it never distinguished anything. Meanwhile
the thing that *does* split a batch — the blend mode — was not in the key at all,
so alpha enemies and additive projectiles alternated all the way down the sort.
The field now holds whatever the batcher splits on, which is atlas, shader and
blend together, and sprites at the same quantised depth are grouped by it. Same
440 sprites, **19** draw calls, and no visual change: two sprites at the same
depth were already in an arbitrary order and this picks a different arbitrary
one.

What is left is inherent. Correct back-to-front order with two blend modes
interleaved by depth cannot be merged further without drawing something in front
of what it should be behind.

### One that was tried and thrown away

A tick builds the broadphase twice: once before the scripts run, so every query
sees the same world, and once after, to sweep. The second build re-reads every
node to arrive at what the first one already had, which looked like free money.
Moving the existing bodies instead — the set cannot change inside a tick, since
spawns and destroys land on a phase boundary — is a walk and an assignment.

It is not equivalent. An invisible node is not a body, so a script hiding a
collider has to reach the same tick's sweep; a cache carried over from before
the scripts ran is a tick behind. Guarding that is doable: stamp the scene
whenever a node changes in any way but its position, and rebuild when the stamp
moves.

Then the measurement came in. 12.7 ms a tick either way, on the stress scene,
over three runs of six hundred ticks each, with the cache confirmed to be hit on
every tick. The second build is not where the time goes. So the cache is gone
and the scene has no stamp on it. What survives is
`crates/dimetric-sim/tests/visibility.rs`, which pins the behaviour the cache
would have broken, for whoever has this idea next.

## Asked for by the tactical roguelike, and declined

**`Label.in_world`, to put text on the ground plane.** Offered alongside the
fix for glyphs running diagonally under `Isometric` — the thought being that
somebody might want the old behaviour deliberately, for a number painted onto
the floor.

Declined, because what the flag would restore is not that. A label's glyph
quads are drawn axis-aligned, like every other quad in the engine: the shader
projects a centre and adds the quad in screen space, which is what keeps
isometric artwork upright instead of shearing each character into a
parallelogram. So `in_world = true` would give **upright glyphs on a sheared
baseline** — which is not text lying on the ground, it is exactly the defect
that was just fixed, with a property name on it.

Real ground-plane text needs the glyph *quads* sheared too, and that is a
different feature: a sprite whose quad goes through the projection. Nothing has
asked for one, it would need its own answer for what happens to a rotated
sprite, and the engine's position that quads are never sheared is load-bearing
in the renderer and in the artwork pipeline both. A flag that half-did it would
be worse than its absence, because it would look like the feature.

If painted-floor text is ever wanted, the thing to build is the general case —
an opt-in projected quad, available to any sprite — and to measure it against a
decal drawn as artwork, which is how the genre has always done it.

**A runtime guard on a table read back from script state.** Writing into one —
`self.bag.b = 2` — lands in a temporary and is dropped, and used to do so with
no error, no warning and no lint: the only failure in the engine that carried
nothing at all. The proposal was a metatable whose `__newindex` raises a
diagnostic naming the variable; a write-through was the second choice.

The *defect* is real and is fixed. The **runtime** guard is declined, because
neither shape works, and both reasons are pinned as tests in
`crates/dimetric-sim/tests/copied_table_facts.rs`:

- **A metatable on the table sees only keys that are absent.** This engine
  already knows that — it is why `require`'s freeze is a proxy rather than a
  metatable, and the note above says so. So a guard would catch `self.bag.b = 2`
  and sail straight past `self.run.pending.at = 2`, where `at` is already in the
  table. That second case is one of the two the report brought.
- **An empty proxy would see every write, and would destroy the variable.** The
  host converts a Lua table back to an engine value by walking it with
  `lua_next`, which ignores `__pairs`. So `self.pending = pending`, where
  `pending` is a proxy, converts to an empty map — a silent data loss far worse
  than the silent no-op being fixed.

A write-through has the first hole too, and a third problem besides: `to_lua`
hands out copies from several stores — script variables, node properties, the
profile, a signal payload — and only some of those have anywhere to write back
to. Implementing it for one leaves the same silence in the rest.

What makes a text scan the right tool here is that the difference is
**syntactic**, which is the one thing the runtime cannot see. `self.bag.b = 2`
writes through a chained access; `local p = self.pending` / `p[#p+1] = v` /
`self.pending = p` does not — and the second is the pattern that works, the one
the proposed diagnostic's own wording recommends, and the one the engine's own
example game uses in three places. A guard that refused it would have broken the
slice. `DIM0507` names the file, the line, the variable and the spelling that
works, which is better located than a runtime error anyway.

## Asked for by the grid roguelike, and partly declined

**An icon on the executable itself, so a Windows build looks right in
Explorer.** Asked for alongside the window and taskbar icon, which is built —
`[game] icon` in `project.toml`, staged by `dim build`, read by the runtime
through whatever the project is read through, so a single-file game finds its
icon inside itself.

The executable's own icon is declined, and the reason is structural rather than
a reluctance to depend on a resource compiler.

`dim build` does not link anything. It is handed a `dim-play` that somebody
else built — `--runtime` takes a binary, because `dim` does not compile Rust
and says so — and it copies that binary. A resource compiler runs at *link*
time, in the crate being linked, and that link happened on another machine, for
another target, possibly for a different game. By the time `dim build` sees the
runtime there is nothing to compile a resource into: there is only a finished PE
file. Putting an icon in it would mean editing that file's resource directory in
place — rewriting `.rsrc`, fixing the section headers, the data directory and
every RVA that moved. That is a PE editor, and the engine would be maintaining
one on three platforms to change a picture.

`--single` makes it worse in a way worth naming: the payload is appended after
the executable's own bytes and the footer carries a BLAKE3 of it, so any surgery
would have to happen before the fold, on the copy, and `dim inspect --verify`
would be verifying a binary that two separate steps had rewritten.

The honest alternative is the other shipping model: build the runtime per game,
with the icon embedded by a build script at link time. That is a real option for
a project that wants it, and it is not something `dim build` can do for one,
because a project that hands over a prebuilt runtime has already chosen not to.

What is built is most of the value and all of the running game: the title bar
and the taskbar both show the game's name and icon, on all three platforms. What
is missing is the file's appearance in a file manager before it is launched.

## Asked for by the grid roguelike, and answered with a smaller thing

**A hook that fires after a run is resumed, so a script can rebuild its Lua
caches.** Not asked for directly — it is what the obvious idiom needs once
suspending exists:

```lua
local mark                     -- a handle, kept for the life of the script
function on_ready(self) mark = scene.find("/World/Mark") end
```

`on_ready` does not fire again after a resume. `readied` says those nodes are
ready, which is correct and must stay correct: re-firing initialisation would
re-roll a run's starting loadout, and a resumed run that handed out a second
sword is worse than one that raises. So the local is nil, and the first use of
it raises `DIM0502` naming the file and the line.

An `on_resumed` hook would fix the idiom and cannot be built safely, for one
reason: it fires **only** on the resumed run. Anything it writes to simulation
state is a difference between a run that was suspended and one that was not,
which is exactly the divergence the whole shape exists to avoid — and nothing
can stop a hook writing state, because that is what hooks do. A hook whose only
safe use is "touch nothing hashed" is a hook whose misuse is undetectable and
whose failure is a replay that diverges at tick 4,117.

The symmetric answer is the engine's existing one, and it is already written
down three times in `crates/dimetric-sim/src/script.rs`: script state lives in
Rust, because a Lua table cannot be snapshotted. A file-scope `local` written
inside a hook is state in Lua. So instead of a hook:

- **`DIM0508`** names the pattern in `dim script check`, with the two spellings
  that work — keep it in `self`, or look it up in the hook that needs it — and
  `-- @transient` for a local that is rebuilt every tick anyway. It took two
  further rounds to make it say only that: the first version read every
  indented line as a hook and every `name = value` as an assignment, so a
  constructor field and a file-scope `for` filling a module table both tripped
  it. Eighteen false warnings in one project, which is worse than no lint. It
  is a block walk now rather than three heuristics — see `lint::scopes`.
- Three of this repository's own replay fixtures used the hazardous idiom and
  now do not, because an engine that ships a lint its own examples trip is an
  engine arguing with itself.
- `crates/dimetric-sim/tests/restore_into_fresh_host.rs` pins both halves: a
  script that keeps nothing in Lua carries on identically, and one that does
  diverges **and reports it**.

The same hazard already applied to a hot reload and to a rollback, and had gone
unnamed in both. Suspending is what made it reachable from a game.

## Still open, and known

**A comment in a `.meta` does not survive the next import.** `write_metas`
compares the file against `ImportSettings::to_text()` and rewrites it whenever
they differ, and `to_text` hand-writes the keys it knows about — so a comment
somebody added to explain a `[[clip]]` or a `[[tile]]` block is silently
deleted the next time `dim asset import` runs. The settings themselves round
trip exactly; only the prose is lost. Worth knowing before writing an
explanation into a sidecar, and worth fixing by editing through `toml_edit`
rather than re-emitting, which is a bigger change than any one request has
needed yet.

**An agent adding a spell changes the run.** The upgrade roll samples a list, so
appending to it shifts every later choice — the fixture had to be re-recorded
after the acceptance test added `frost`. That is correct behaviour rather than a
bug, and it is worth knowing before wondering why a replay stopped reproducing.
