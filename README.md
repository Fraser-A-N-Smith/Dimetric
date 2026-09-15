# Dimetric

A 2D game engine in Rust for top-down and isometric games, built around one
idea: **the same seed and the same input log produce byte-identical state, on
every machine, forever.**

That constraint is what makes the rest of it work. An agent can verify its own
changes by replaying a run and asserting on the result. A bug reproduces from a
seed instead of a bug report. And rollback netcode, if it is ever wanted, is a
networking problem rather than a rewrite.

## Status

**0.1.0**, the first release. Early, and further along than that suggests: the
simulation, the renderer, the editor, the CLI and the runtime all exist and are
tested, and a game packaged by `dim build` runs on its own, with sound. The gap
between this and something to write a real game in is content tooling and time
rather than missing layers. See [Milestones](#milestones).

The version is a statement about the formats and the API rather than about how
finished the engine is. Until 1.0 a minor bump may change a `.dim` file, a
recorded log, the Lua surface or a command's output, and
[`CHANGELOG.md`](CHANGELOG.md) says what each one breaks — including whether it
moves the state hash, which is the one that costs you a re-record.

## On the name

What pixel-art games call "isometric" almost never is. True isometric
projection puts 120° between all three axes; the 2:1 tile ratio the genre
actually ships is *dimetric* projection, where one axis foreshortens
differently from the others. The engine is named for the projection it really
uses.

## The three ideas

**The command bus is the engine.** Every mutation is a serializable, invertible
command. The editor, the CLI and an agent are all clients of one layer. Undo
falls out for free, and because the bus is the only way in, it cannot
desynchronize.

**Determinism is an invariant, not a feature.** No floats in simulation, no
`HashMap` iteration, no wall-clock reads, all randomness from a seeded stream.
These are checked by `cargo xtask lint-sim` and gated in CI, because none of
them fails loudly on its own — they fail three weeks later as a replay that
diverges on someone else's computer.

**Perspective is a camera matrix.** The world is free-form 2D. Top-down is the
identity; isometric is a 2:1 shear applied at render time. One flag, no engine
fork.

## Try it

```sh
cargo run -p dimetric-agent -- --project examples/sorcerer --scene arena01 scene tree
```

Start a project of your own, and play it:

```sh
dim new mygame
cargo run -p dimetric-player --features gui -- mygame
```

Run the simulation headlessly and record what happened:

```sh
dim --project examples/sorcerer --scene arena01 \
    run --headless --ticks 120 --seed 42 --record run.hashes
```

Replay it and assert on the result:

```sh
dim --project examples/sorcerer --scene arena01 \
    replay --input tests/full-run.input \
           --hashes tests/arena01.hashes \
           --assert tests/arena01.probes
```

Every command takes `--json` and returns a structured envelope. Every failure
carries a `DIM####` code with machine-readable fields, so nothing has to parse
prose to find out what went wrong.

## Playing, and shipping

```sh
cargo run -p dimetric-player --features gui -- examples/sorcerer --record run.input
```

A window, a fixed tick, WASD or the arrows, mouse to aim. `--record` writes
what you did as an input log, so a run you played reproduces under `dim replay`
— which means a bug you hit by playing arrives as evidence rather than as a
description of it.

The window is the small half. The clock that decides how many ticks a frame
owes, the bindings that turn keys into input, and the session that owns the
simulation are all in `dimetric-player` as a library with no display in it, and
they are tested that way.

```sh
dim --project examples/sorcerer build --target linux --runtime target/release/dim-play
```

stages the game into `build/linux/`: the scenes, the scripts, the prefabs, the
assets and the import cache, plus a manifest saying which scene to boot. The
runtime reads that manifest from beside itself, so the staged directory runs
with no arguments. `dim` does not compile Rust — cargo does — so `--runtime`
takes a `dim-play` built for the target you asked for.

## Scenes

Scenes are TOML. Nodes are a flat list, each naming its parent by id, and the
tree is rebuilt on load:

```toml
format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena01"

[[node]]
id = "n_m9v2ht5w"
kind = "Sprite2D"
name = "Brazier"
parent = "n_root0000"          # /Arena01
pos = [96.0, 48.0]
texture = "asset:sprites/props/brazier"
```

Flat rather than nested because reparenting a subtree should be a one-line
change, not a fifty-line re-indentation, and because two branches adding nodes
in different places should touch different regions of the file. Canonical form
orders nodes depth-first, so it still reads as a tree.

Four things about the format are worth knowing:

- **Loading and saving an unedited scene reproduces it byte for byte**, comments
  included. Edits patch the parsed document rather than re-rendering it, so
  formatting is an explicit step (`dim scene fmt`) and not something that fires
  on every save and buries the real change. Formatting patches that same
  document, so your comments survive it.
- **Unknown properties are a hard error.** `raduis = 72.0` is caught on load,
  and the message names the property you probably meant.
- **Numbers are read from the literal text, not from a float.** `0.1` is not
  exactly representable in fixed point, so it is refused (`DIM0203`) rather than
  silently rounded. Silent rounding is how replay determinism dies quietly.
- **A project can add node kinds of its own.** A `kinds.toml` in the project
  root declares them — `Enemy extends Collider`, with `max_health` and
  `speed` — and they validate, canonicalise and override exactly like the
  built-ins, because it is the same machinery. The engine treats a kind as
  whatever it extends, so an `Enemy` collides.

## Scripting

Gameplay is Lua 5.4, so iteration does not mean recompiling the engine.

```lua
function on_ready(self)
  self.health = 40
end

function on_tick(self)
  local player = scene.find("/Arena01/Player")
  if not player then return end

  local to = player.pos - self.pos
  if to:length() < fx.new(180) then
    self:set_velocity(to:normalized() * fx.new(35))
  end
end
```

Two rules make this safe to replay:

**Script state lives in Rust.** `self.health = 40` writes into simulation state,
not a Lua global. A Lua table cannot be snapshotted bit-for-bit, so anything
kept there would drop silently out of rollback and out of the state hash. This
is also why `require` hands back a table the engine has frozen: a module is
constants and pure functions, and a module you could write to would be a place
for state to hide from the hash.

**Arithmetic goes through `fx` and `vec2`.** Lua numbers are `f64`, and floats
written into simulation state are the easiest way to break replay. The sandbox
has no `os`, `io` or `math.random`, and no `math.sin` either — platform maths
libraries do not agree with each other, so `fx.sin` reads a committed lookup
table instead.

## Layout

```
crates/
  dimetric-core       fixed point, identity, seeded RNG, diagnostics
  dimetric-scene      node tree, .dim format, prefab instancing
  dimetric-sim        tick loop, physics, scripting, snapshots
  dimetric-host       command bus, undo, project, replay harness
  dimetric-agent      the dim CLI and the MCP server
  dimetric-render     projection, sort keys, batching
  dimetric-audio      mixer buses, voice pool
  dimetric-assets     asset identity, import settings
  dimetric-platform   input sources, project paths
  dimetric-editor     editor view state
  dimetric-player     the runtime: clock, bindings, session
```

Dependencies run strictly downward and CI enforces it.

## Milestones

| | Milestone | State |
|---|---|---|
| M0 | Spike | done — throwaway, findings in `spikes/m0-quad/README.md` |
| M1 | Core and scenes | done — exact fixed point, `.dim` round trip, prefab overrides |
| M2 | Determinism harness | done — snapshots, state hashing, replay with first-divergence reporting |
| M3 | Renderer | done — wgpu backend, three passes, headless capture, golden images on three platforms |
| M4 | Scripting | done — mlua, handles, sandbox, structured errors, cost measured at 2000 entities |
| M5 | Physics | done — spatial hash, swept movement with sliding, triggers |
| M6 | Assets, tiles, audio, animation | done — import pipeline, LDtk baking, tweens and frame animation, and audio from a `Sound` node through the mixer to a device; the device itself is behind the `kira` feature |
| M7 | Editor | done — tree, inspector, viewport, console, assets, play-in-editor and scrubber; the window is behind the `gui` feature |
| M8 | Agent interface | done — full CLI, MCP server, generated docs and schemas |
| M9 | Vertical slice | done — a five-room run, spells and evolutions, the agent acceptance test passing; density measured and improved 7x |
| M10 | Hardening | done — runtime, packaging, `dim new`, and the broadphase and batcher passes |
| M11 | Text and fonts | done — TTF baked at import to a glyph page and integer metrics, a `Label` node, layout that is the same on every machine |
| M12 | UI | core done — `Control` and `Panel`, anchors and offsets, fixed-point layout against a canvas, a screen-space render layer. Widgets and containers beyond a panel are not built |
| M13 | Input breadth | core done — gamepads behind the `pad` feature, an analogue stick quantised into something a replay can hold, the pointer in the input frame. Touch is not built: there is no platform with one yet |

Every command in the CLI is implemented. `DIM0801` no longer means "this build
does not do that yet" — it is what you get when a command needs something the
machine has not got, such as a graphics adapter for a headless capture or a
`dim-play` built for the target you are packaging for. It says which.

## Editor

```sh
cargo run -p dimetric-editor --features gui -- <project> [scene]
```

Behind a feature, so building the engine does not build a GUI toolkit.

The interesting half is not the window. Every rule the editor has lives in
`dimetric-editor` as a library that draws nothing: the tree, the inspector, the
asset browser, play-in-editor and the scrubber are models over engine state, and
the window is a few hundred lines that draws them and dispatches `Action`s.
§16 names editor scope as the highest risk in the project, and a client that
holds no logic cannot grow any.

Two rules hold, and both are tests rather than intentions. Every mutation goes
through the command bus — a scripted session asserts that anything which changed
the file produced a command. And opening and closing a scene produces zero diff:
camera, selection and fold state live in a committed `.dim.editor` sidecar.

## The slice

`examples/sorcerer` is a run of a top-down game, and the reason the engine has
the shape it does. Five rooms, each a wave the arena spawns; clear one, take one
of three upgrades, and two base spells held together evolve into a third.

```sh
dim --project examples/sorcerer --scene arena01 \
    replay --input tests/full-run.input \
           --hashes tests/arena01.hashes \
           --assert tests/arena01.probes
```

That replay is the interesting part. It asserts the run clears all five waves,
kills twenty enemies, and ends holding the **Forking Arc** — the same spell
evolution, every time, on every machine. Spell definitions are a Lua table on
one node, so balance tuning is an edit and a reload.

`stress.dim` is not a game: four hundred live projectiles against forty enemies,
which is the density §12 asks the engine to survive. It found that
`scene.nearest` cost 139 ms a tick, and seven times faster later it found
something better — that the broadphase is no longer where the time goes, and the
Lua boundary in front of it is. `docs/ENGINE-GAPS.md` has the measurements, the
one optimisation that was built and thrown away, and everything else the slice
turned up: the gaps that belonged in the engine, the ones that did not, and the
ones still open.

## Assets

Sources live in `assets/`, settings in a sibling `.meta`, imported artifacts in
`.import/` keyed by the content hash of the source. The cache is derived and
gitignored; deleting it costs a reimport and nothing else.

```sh
dim asset list              # names, ids, hashes, what is stale
dim asset reimport          # import what moved
dim asset info sprites/hero # id, hash, where it sits in the atlas, its clips
```

A `.meta` can also declare a plain PNG to be a sprite strip, which nothing in
the file itself says:

```toml
frames = 3
frame_ms = 100
```

That imports as an animation with one clip, the same shape an Aseprite document
produces from its tags.

Three things happen at import rather than at runtime, all for the same reason.
Atlas packing, so a golden image is of one layout rather than whichever the
packer happened to produce. Aseprite frame durations, which are milliseconds in
the source and ticks in the simulation — converting on load would make frame
advance depend on the tick rate a session happened to run at. And identity: the
id lives in the `.meta`, not in the path, so renaming a file is free.

LDtk is an authoring front-end and import is one way:

```sh
dim tile import-ldtk assets/levels/arena.ldtk --level Arena01
```

Tile layers bake to native chunks through the command bus, so an import undoes
like any other edit and the tile CLI works the same whether a level came from
LDtk or from `dim tile fill`. LDtk owns tile layers; `.dim` owns every entity.

## Audio

A sound is a node:

```toml
[[node]]
id = "n_bump0000"
kind = "Sound"
name = "Bump"
parent = "n_player00"
stream = "asset:sfx/bump"
pitch_variation = 0.125
```

and a script says when, not what:

```lua
self:find("Bump"):play()
```

What plays, on which bus, how loud and how much the pitch wanders are
properties of the node, so an instance override can change them and a designer
can find them.

**The simulation never plays a sound.** It says what it would play, into a list
cleared at the start of every tick and never hashed. Whoever is listening reads
it — a device in the player, nothing at all in a headless run. That line is not
tidiness: if triggering a sound consumed a random number or wrote something
hashed, then muting a game would change how it plays, and a run recorded with
audio on would diverge from one played with it off. `dim run` counts the sounds
a headless run asked for, which is how you check a scene makes a noise without
owning a sound card.

Everything after that list is presentation. The mixer decides what survives
voice stealing and the per-clip caps; the backend makes the noise. Keeping them
apart is what lets those rules be tested with no sound card involved. Pitch
variation draws from a presentation RNG stream, deliberately not the
simulation's: sharing one would mean that triggering one fewer sound effect
shifted every gameplay roll after it.

The real device is behind the `kira` feature (`--features gui,sound` on the
player), off by default because it needs ALSA and D-Bus development headers on
Linux. Without it, and with `--mute`, the mixer still runs and nothing reaches a
device.

## Rendering

```sh
dim --project tests/golden/scenes --scene room \
    frame capture --png room.png --width 256 --height 192 --internal 128x96
```

Headless and windowed rendering run the same passes and differ only in the
target they are handed, so a captured frame is the frame a person would have
seen. Headless works on a machine with no GPU — CI renders on a software
adapter — which is what makes the golden-image tests possible at all.

Two things about the isometric path are worth knowing before drawing anything:

- **The projection moves positions, not shapes.** A sprite's position goes
  through the 2:1 shear and its quad is drawn upright. That is how the genre
  works: the artwork is already drawn in projection. Shearing the quad as well
  turns every character into a parallelogram.
- **So isometric tile art is diamond-shaped**, at the 2:1 ratio the projection
  places it on. Square tiles under the shear tessellate into columns with gaps,
  which is the artwork being wrong rather than the engine.

## Development

```sh
cargo xtask ci
```

runs what CI runs: formatting, clippy, the full test suite, the simulation
invariant lint, the dependency-direction check and scene canonical form.

If you touch `dimetric-core`, `dimetric-scene` or `dimetric-sim`, add a replay
fixture under `tests/replay/` covering what you changed.

`cargo bench -p dimetric-sim` measures what a Lua node handle costs, at entity
counts the slice game will reach. Its findings are in the benchmark's own
documentation. The short version: handle validation is cheap and flat, and
`scene.find` is not — resolve paths once in `on_ready` and keep the id.

## Agents

Every command is a `dim` subcommand, takes `--json`, and fails with a `DIM####`
code rather than prose. The same commands are served over the Model Context
Protocol:

```sh
dim mcp
```

Stdin and stdout, one JSON-RPC message per line. The tool list is read from the
CLI's own argument parser at startup rather than written out beside it, so there
is one command surface and no second copy to go stale — a subcommand added to
the CLI is a tool. `dim api tools` prints the same list without starting a
server.

A command that says no comes back as a tool result carrying `isError` and its
diagnostics, not as a protocol error: the request was well-formed and the answer
was no, and an agent should be able to tell those apart.

## Documentation

[`docs/API.md`](docs/API.md) lists every command, node kind, diagnostic code and
MCP tool. It is generated by `cargo xtask gen-docs` and CI fails if it is stale,
so it cannot disagree with the code. JSON schemas are in `docs/schemas/`.

[`CHANGELOG.md`](CHANGELOG.md) is the per-release breaking-change log, and
[`docs/ENGINE-GAPS.md`](docs/ENGINE-GAPS.md) is what the engine does not do,
with the argument for each — including the ones deliberately left undone.

[`CONTRIBUTING.md`](CONTRIBUTING.md) has the invariants and the scope
boundaries. The boundaries are not negotiable — in an open-source project the
main scope-creep vector is not your own ambition, it is well-meaning pull
requests, and saying no is far cheaper when the line was drawn first.

## Licence

MIT or Apache-2.0, at your option. Contributions are DCO sign-off, no CLA.
