# Engine gaps the slice found

Building the sorcerer slice surfaced the things the engine does not do. §12's
discipline note says to log every one and build none of them mid-slice, because
roughly half turn out to be game-specific and shipping the slice is the only way
to learn which half.

Each entry says what the game wanted, what it did instead, and — the part worth
arguing about — whether it belongs in a reusable engine.

## Fixed

The first pass at the slice logged these; the second pass built them, and the
slice was rewritten on top. Everything transient is now spawned — a room is a
wave rather than a file, and a run is five of them.

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

**Scripts can ask what is nearby.** `scene.near` and `scene.nearest` read the
broadphase the simulation already builds. See the performance section below:
the first version of this was most of a frame budget on its own.

## Open: probably game-specific

**No module system for scripts.** The sandbox has no `require`, so two scripts
cannot share a table. The spell definitions live on a `Spellbook` node that
publishes them as node variables, which any script can read. Slightly odd, works
fine, and adding a module loader means deciding what a module's identity is when
a script is hot-reloaded — a real design cost for something one node solves.

**No way for one script to call a function on another.** Damage is written into
the target's own variables (`other.pending_damage = ...`) and read on its next
tick. That is arguably better than a direct call — it keeps the ordering
explicit and survives the target being destroyed mid-frame — so this is probably
not a gap at all.

**No scene loading from a script.** Still true, and it turned out not to
matter: a room is a wave the arena spawns, not a file it loads, so a run of five
rooms lives in one scene. Loading a scene mid-tick would mean the tick was not a
pure function of the state it started from (I8), so this is probably right as
it stands.

**No walls.** The arena has no bounds, so the player can walk out of it. A
tilemap collision layer exists; the fixture simply does not use one. Not an
engine gap, a scene that was never finished.

## Not a gap, but worth writing down

Fixed-point exactness caught a mistake that would otherwise have been a
heisenbug: an input log with `0.4` in it is refused (`DIM0703`) because 0.4 is
not exactly representable. The instinct is to call that pedantic. It is the
reason a recorded run reproduces.

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

## Still open, and known

**An agent adding a spell changes the run.** The upgrade roll samples a list, so
appending to it shifts every later choice — the fixture had to be re-recorded
after the acceptance test added `frost`. That is correct behaviour rather than a
bug, and it is worth knowing before wondering why a replay stopped reproducing.
