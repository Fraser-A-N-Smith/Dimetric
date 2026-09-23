# Replay fixtures

Each directory is one recorded run. CI replays all of them on Windows, macOS and
Linux and fails if any hash differs from the one checked in.

```
<name>/
  scene.dim        the scene to simulate
  run.input        the input log, including its seed
  run.hashes       one state hash per tick
  run.probes       optional assertions evaluated during the replay
  scripts/         optional Lua
```

To add one, build the scene, then:

```sh
dim --project tests/replay/<name> --scene scene run \
    --headless --ticks 120 --input run.input --record run.hashes
```

Read the diff before committing the hashes. A changed hash means either you
changed behaviour on purpose, or you broke determinism — and the point of the
fixture is that you have to decide which.

When a replay does diverge, `dim replay` reports the **first** divergent tick
and stops. Every later tick is downstream of it, so that is the one to debug.

## Probes

One per line, in `run.probes`:

```
tick <n> <path> <field> <op> <value>
```

The value runs to the end of the line, so it may contain spaces. Quoting it is
optional and the quotes are **not** part of the value — `== none` and
`== "none"` mean the same thing. Quote it when the value has a leading or
trailing space, when it is empty (`== ""`), or when it contains a `#`, which
outside quotes starts a comment and which is the first character of every
colour this engine writes.

A probe at tick *N* is evaluated **after** tick *N* has been stepped, so it
sees one more tick than `dim state dump --tick N` does.
