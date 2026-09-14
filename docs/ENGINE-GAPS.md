# Engine gaps the slice found

Building the sorcerer slice surfaced the things the engine does not do. §12's
discipline note says to log every one and build none of them mid-slice, because
roughly half turn out to be game-specific and shipping the slice is the only way
to learn which half.

Each entry says what the game wanted, what it did instead, and — the part worth
arguing about — whether it belongs in a reusable engine.

## Fixed during the slice

Three were not missing features but existing ones that were wrong or absent in a
way no game could work around. They were fixed rather than logged.

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

## Open: probably belongs in the engine

**No way to create a node from a script.** The slice wants projectiles, and
there is `destroy` and no spawn. Worked around by authoring a pool of 64
projectiles in the scene and taking from a free list — which is what a game at
this sprite density would do anyway, so the workaround is not a bad outcome. But
"the engine can destroy but not create" is an asymmetry a second game would hit
in its first hour. Ids would have to come from the seeded stream to stay
reproducible, which is the design question worth answering before building it.

**A moving trigger volume has no representation.** An `Area` is never swept, so
a projectile authored as one sits where it spawned with a velocity it cannot
use. Worked around by making projectiles `Collider`s, which are swept and
report contacts; they slide off an enemy for the one tick before they park.
Cost the longest debugging session of the slice, because a bolt that spawns and
does not move looks like a scripting bug and is not.

**No per-instance authored data.** §12 asks for enemy variants as instances with
property overrides. An override reaches a node's *properties*, and a project
cannot declare a property of its own on a node kind — so the numbers that make a
wraith a wraith have nowhere to live. Worked around with a tag per variant and a
table in the enemy script, which means the stats are in Lua rather than in the
scene, where a designer would look for them. This is the gap most likely to
annoy somebody who is not the person who wrote the engine.

**No edge-triggered input.** `PlayerInput::pressed` exists in Rust and is not
exposed; only `held` is. You clear a room with the fire button down, so an
upgrade offer read from a held button is taken the instant it appears and the
player never sees it. Worked around in four lines — remember last tick's state
and compare — which every script that reads a button will now repeat. Cheap to
fix, cheap to work around, and worth fixing on volume alone.

## Open: probably game-specific

**No module system for scripts.** The sandbox has no `require`, so two scripts
cannot share a table. The spell definitions live on a `Spellbook` node that
publishes them as node variables, which any script can read. Slightly odd, works
fine, and adding a module loader means deciding what a module's identity is when
a script is hot-reloaded — a real design cost for something one node solves.

**No spatial query from a script.** Homing projectiles want the nearest enemy
and get it by walking every node tagged `enemy`, which is linear per projectile
and quadratic overall. The engine *has* a spatial hash; it is built for the
sweep and thrown away. Exposing a query would be a genuine engine feature, but
the slice runs fine at its density and a game that needed it at scale might want
something more specific than "nearest".

**No way for one script to call a function on another.** Damage is written into
the target's own variables (`other.pending_damage = ...`) and read on its next
tick. That is arguably better than a direct call — it keeps the ordering
explicit and survives the target being destroyed mid-frame — so this is probably
not a gap at all.

**No scene loading from a script.** A run is a graph of rooms and the slice is
one room, because a script cannot load the next scene. The room graph would be
driven from outside the simulation, which is likely correct: loading a scene
mid-tick would mean the tick was not a pure function of the state it started
from (I8).

**No walls.** The arena has no bounds, so the player can walk out of it. A
tilemap collision layer exists; the fixture simply does not use one. Not an
engine gap, a scene that was never finished.

## Not a gap, but worth writing down

Fixed-point exactness caught a mistake that would otherwise have been a
heisenbug: an input log with `0.4` in it is refused (`DIM0703`) because 0.4 is
not exactly representable. The instinct is to call that pedantic. It is the
reason a recorded run reproduces.
