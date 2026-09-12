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
