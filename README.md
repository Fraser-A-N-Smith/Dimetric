# Dimetric

A 2D game engine in Rust for top-down and isometric games, built around one
idea: **the same seed and the same input log produce byte-identical state, on
every machine, forever.**

That constraint is what makes the rest of it work. An agent can verify its own
changes by replaying a run and asserting on the result. A bug reproduces from a
seed instead of a bug report. And rollback netcode, if it is ever wanted, is a
networking problem rather than a rewrite.

## Status

Early. The simulation layer is real and tested; the renderer and editor are not
built yet. See [Milestones](#milestones) for what that means in practice.

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

Three things about the format are worth knowing:

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
kept there would drop silently out of rollback and out of the state hash.

**Arithmetic goes through `fx` and `vec2`.** Lua numbers are `f64`, and floats
written into simulation state are the easiest way to break replay. The sandbox
has no `os`, `io`, `require` or `math.random`, and no `math.sin` either —
platform maths libraries do not agree with each other, so `fx.sin` reads a
committed lookup table instead.

## Layout

```
crates/
  dimetric-core       fixed point, identity, seeded RNG, diagnostics
  dimetric-scene      node tree, .dim format, prefab instancing
  dimetric-sim        tick loop, physics, scripting, snapshots
  dimetric-host       command bus, undo, project, replay harness
  dimetric-agent      the dim CLI
  dimetric-render     projection, sort keys, batching
  dimetric-audio      mixer buses, voice pool
  dimetric-assets     asset identity, import settings
  dimetric-platform   input sources, project paths
  dimetric-editor     editor view state
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
| M6 | Assets, tiles, audio, animation | done — import pipeline, LDtk baking, mixer and fades, tweens and frame animation; the audio **device** is behind the `kira` feature |
| M7 | Editor | tree, inspector, viewport, console, assets, play-in-editor and scrubber; the window is behind the `gui` feature |
| M8 | Agent interface | done — full CLI, generated docs and schemas |
| M9 | Vertical slice | a five-room run, spells and evolutions, the agent acceptance test passing; density measured and improved 7x |
| M10 | Hardening | not started |

Commands that exist but are not implemented fail with `DIM0801` naming the
milestone they belong to, rather than pretending to succeed.

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
`scene.nearest` cost 139 ms a tick; it now costs 20. What that cost and what was
wrong with it is in `docs/ENGINE-GAPS.md`, along with everything else the slice
found — the four gaps that turned out to belong in the engine, the ones that did
not, and the two still open.

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

The mixer decides what plays and the backend makes noise, which is what lets
voice stealing, per-clip caps and fades be tested with no sound card involved.
Headless runs and CI get the mock backend and produce no device I/O at all. The
real one is behind the `kira` feature, off by default because it needs ALSA and
D-Bus development headers on Linux.

Pitch variation draws from a presentation RNG stream, deliberately not the
simulation's: sharing one would mean that triggering one fewer sound effect
shifted every gameplay roll after it.

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

## Documentation

[`docs/API.md`](docs/API.md) lists every command, node kind and diagnostic code.
It is generated by `cargo xtask gen-docs` and CI fails if it is stale, so it
cannot disagree with the code. JSON schemas are in `docs/schemas/`.

[`CONTRIBUTING.md`](CONTRIBUTING.md) has the invariants and the scope
boundaries. The boundaries are not negotiable — in an open-source project the
main scope-creep vector is not your own ambition, it is well-meaning pull
requests, and saying no is far cheaper when the line was drawn first.

## Licence

MIT or Apache-2.0, at your option. Contributions are DCO sign-off, no CLA.
