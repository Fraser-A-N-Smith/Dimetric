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

### Added: a tile can animate

`crates/dimetric-render/src/extract.rs`, `fn tiles` read a cell's id, subtracted
one, sliced the sheet, and consulted nothing else. That is every tile a still
forever: water on a played floor could not ripple, and a region built out of
seven terrains in four looks each had no way to make any of them move.

A tileset's sidecar can now say which ids move:

```toml
[[tile]]
id = 5
frames = [5, 33, 61, 89]
frame_ms = 240
```

`frames` are tile ids, the same numbers an author paints with, so the painted
id usually comes first and a frame may repeat — `[3, 4, 3, 2]` is a ripple that
goes out and comes back. Tile `0` is the empty cell and cannot be animated. A
block with no `id`, no `frames`, an empty cycle, a frame of `0`, or a second
block for the same id fails the import naming the file, rather than quietly
falling back to a still: a floor that does not move looks exactly like art
nobody has finished yet.

Two deviations from the shape that was proposed, both for consistency with the
file the block now lives in. It is `[[tile]]` with an `id` field rather than
`[tiles.5]`, matching the `[[clip]]` blocks beside it; and the timing is
`frame_ms` rather than `ms`, because it is the same quantity `[[clip]]` already
spells that way and two names for one thing is how a sidecar becomes guesswork.

**It does not move the state hash, and it is not in the scene.** The obvious
implementation is to step the map each tick. That would mean a rollback had to
undo a ripple, every snapshot carried a decoration, and the state hash of a
replay depended on the art — change a `.meta`, and a recorded run that verified
yesterday diverges today. So the cycle is declared on the sheet, converted to
whole ticks at import against the project's tick rate, and *asked* at draw
time: which slice to draw is a pure function of a tile id and a tick, and the
chunk still holds the id an author painted.

`examples/sorcerer`'s floor band shimmers as of this entry, and
`examples/sorcerer/tests/arena01.hashes` was **not** re-recorded — it still
passes unchanged. That is the proof, and it runs in CI.

The new door is `dimetric_render::extract_at`, which takes the tick. `extract`
and `extract_with_canvas` keep their signatures and pass `Tick::ZERO`: a still
of a scene, which is what an editor viewport wants. Whole ticks, never the
interpolation alpha — a cycle advancing on the host's accumulator remainder
would make `dim frame capture --tick N` depend on how busy the machine was.

### Fixed: a game can set its own volume

`docs/API.md` has said since M12 that a volume change reaches the mixer through
`event.emit`, and the mixer has had `set_bus_gain` for as long. Nothing joined
them: `dim-play` never called `Session::drain_events`, so a game's Options
screen emitted three events on every launch and all three went nowhere.

```lua
event.emit("audio.bus_volume", { bus = "Music", percent = 80 })
```

| Field | Meaning |
|---|---|
| `bus` | `"Music"`, `"Sfx"` or `"Ui"` |
| `percent` | A whole number, 0 to 100. `0` is **silent** |

`dim-play` drains each frame and applies the kinds the engine owns; the rest is
left for whoever drained it. Percent, and whole, because a setting a player
chose is a number of steps and the determinism lint rightly refuses a float
literal in Lua.

**The curve is decibels, not amplitude.** Loudness is logarithmic, so a linear
gain makes halfway up already most of the way loud and a stepped control feel
nothing like even. Percent maps onto a 40 dB range instead, which gives a
five-step slider five equal steps — 2.51× of gain apiece — and `0` is silent by
its own case, because a logarithmic scale has no bottom:

| percent | 0 | 20 | 40 | 60 | 80 | 100 |
|---|---|---|---|---|---|---|
| decibels | — | −32 | −24 | −16 | −8 | 0 |
| gain | 0.000 | 0.025 | 0.063 | 0.158 | 0.398 | 1.000 |

Applied over an 80 ms fade, so stepping a slider is heard as a change rather
than a click, and through `Speaker::set_bus_gain`, which tells both the voice
pool (what a *new* voice is worth) and the backend (what the ones already
sounding are). That second half is why this could not be done from script: a
`continuous` music voice carried over from the previous floor is already
playing, and no amount of `volume_db` on a new scene's `Sound` nodes reaches it.

`Session::apply_events` reads the drained list rather than consuming from it, so
a runtime that mirrors a volume to an OS mixer, or a Steam integration reading
the same list, still finds its own kinds in it. A kind the engine claims with a
payload it cannot read is the new **`DIM1103`** — the game asked for something
and did not get it — and a kind it does not claim is not a diagnostic at all. A
percent outside 0..100 is clamped rather than refused: a game that computed 120
meant loud, and a volume control is not a gain stage.

**Does not move the state hash.** A test runs the same twelve ticks with and
without applying the events and compares every hash: a run played with the
music off has to hash identically to one played with it on, or a recorded
session would replay differently depending on a setting in somebody's profile.
`dim run` still reports the events and applies nothing, because a headless run
has no device.

### Fixed: `DIM0508`'s remaining false positives, and the blind spot it had

Two more shapes, both false by the lint's own definition — *a file-scope local
written inside a hook is state in Lua* — and neither inside a hook.

**A file-scope loop filling a module table.** `rebound_file_local` took
"indented" to mean "inside a function". These lines are indented and run at
**load** time:

```lua
for i = 1, #M.ORDER do
  M.ELITE_OF[M.ORDER[i]] = "elite"
  BY_FAMILY[i] = {}
end
```

Each builds a constant from data every time the module is required, which is
the same thing that makes `local M = {}` safe — and `-- @transient` would have
been a lie about them, teaching readers that the mark means "ignore the lint".

**A constructor that opens mid-line.** Brace depth was measured at the start of
each line, so the `{ fade = 10 }` here did not count and the `=` inside it was
read as a write to `M`:

```lua
queue_fx(m, now, "prefabs/fx_sigil",
         M.world_of(cell_w, cell_h, g, a.cell), INK_MINE, 14, { fade = 10 })
```

Both are the same question asked twice — *what encloses this `=`* — so the two
heuristics are replaced by one walk that answers it: a stack of block openers
and table constructors, and the innermost of those that can hold an assignment.
A function body means a hook, a brace means a field, and anything else means
load time. It tracks short strings, line comments and long brackets — `[[ ]]`,
`--[[ ]]` and their `[==[` forms — because a commented-out `function` would
otherwise unbalance the stack for the rest of a file, and a lint that switches
itself off part-way through one is worse than no lint.

It is still not a Lua parser, and answers only that one question. Three things
fell out of it:

- the blind spot the previous round wrote down is closed: a function literal
  inside a constructor is a function body, so a write there is named;
- a `local` inside a file-scope `for` is no longer collected as a module-level
  name, which it never was;
- `f({ a = 1 }); M.x = 2` is seen, because a field's `=` is skipped rather than
  ending the search for a statement's.

**Does not move the state hash:** a lint reads scripts and changes nothing.

**A correction.** The previous entry said all eighteen warnings in that project
were the constructor-field shape. Eleven were; these seven were the rest. Both
the entry and `lint::scopes`' own notes now say so.

### Fixed: a fractional scale was dropping strokes out of text

The present pass — the last blit, which takes the finished
internal-resolution frame and lays it into the window — shared the atlas's
sampler, and that sampler is `Nearest` because this engine is for pixel art.
At a whole scale that is exactly right. At 0.9, which is what G44's default
window gives a 1920×1080 game on a 1080p screen, nearest never reads one
source row and one column in ten.

On a sprite that loses a pixel here and there. On a letter it loses a stroke: a
game's run menu at 1728×972 read *Aim en action* because the bowl of the `a`
sat on a row that went, *Act* lost the bar of its `t`, and *Choose* lost the top
of its `C`. Which strokes go moves with the window size, so it is not something
a font or a layout can be designed around.

**The last blit is now filtered when the scale is not a whole number, and only
then.** Sprites drawn *into* the frame keep nearest, which is where pixel art is
and where crisp is the point. Measured on that label at 1728×972: with nearest,
22 of the 103 columns the text occupies hold no ink at all and the lit-pixel
count is 179; with the new default every column is there and it is 484.

Reduced to its smallest form in `crates/dimetric-render/tests/present_filter.rs`:
a twenty-pixel-tall frame with one row of ink in it, presented at 0.9. Nearest
loses rows 4 and 14 of the twenty **completely** — the finished picture is
blank — and the default loses none of the twenty. At 1×, 2× and 3× the default
and nearest are byte-identical, so nothing that was already right has softened.

```toml
[render]
present_filter = "auto"     # the default, as described above
present_filter = "nearest"  # a frame with rows missing over a soft one
present_filter = "linear"   # for art that was never on a pixel grid
```

`nearest` is a real preference for some pixel art, which is why it is a setting
rather than assumed away. Area averaging would be better still for a downscale
and was not built: at 0.9 a mip chain still selects level 0, so it would cost
generating mips on the internal targets every frame and buy nothing at the scale
that matters.

**Does not move the state hash**, and cannot: the choice is read from the scale
`placement` already produced and feeds back into nothing. A test asserts that
every placement and every `window_to_internal` is identical under all three
settings, so a click still lands on what is drawn under it.

### Fixed: `DIM0508` named table fields as writes to a module

```lua
local route = require("scripts/route.lua")
local sfx = require("scripts/sfx.lua")

function on_ready(self)
  self.run = { route = {}, sfx = sfx.fresh(), floor_at = 1 }
end
```

Nothing there writes `route` or `sfx`: inside `{ … }` a `name = value` is a
**field**, and a field whose key matches a module's name is the commonest thing
in Lua. One project reported eighteen warnings, **eleven** of them this shape —
this entry first said all eighteen were, which was wrong; the other seven were
two further false shapes, fixed in the entry above. Every one of the eighteen
was false, which is worse than having no lint: false warnings bury a true one.

Braces are exact for this rather than a heuristic: Lua's blocks are `do … end`
and `function … end`, so `{` opens a table constructor and nothing else. The
scan already blanks strings and strips comments, so counting them is reliable.

The blind spot this left was a function literal *inside* a constructor —
`local M = { go = function() cached = build() end }` — on the grounds that
telling it apart needed matching every `end` to its opener. The entry above
built that matching for a different reason and closed this too.

A write *through* a file-scope local is now named as well — `M.count = 1`,
`seen[k] = true` — because a table's fields live in Lua exactly as the binding
does and are lost on a restore for the same reason. `-- @transient` still says
an author has checked.

**Does not move the state hash:** a lint reads scripts and changes nothing.

### Fixed: a window smaller than the game showed a cropped game

`RenderSettings::placement` clamped the integer-upscale factor to 1. For a
1920×1080 game in the old fixed 1440×810 window, `fit` was 0.75, the floor was
0, and the clamp made it 1 — so the frame was drawn at full size and centred,
and 240 pixels came off each side and 135 off the top and bottom. A menu
button, a right-hand rail and half a bottom bar were outside the window, and a
click aimed at any of them landed on nothing, because the cursor was
unprojected through the same wrong scale.

The clamp was the wrong shape rather than the wrong number. Whole multiples
exist to keep pixels square while scaling **up**; there is no whole multiple
below one, so below one the choice is a fractional scale or throwing part of the
frame away. `integer_upscale` now applies when the frame fits, and under that
the exact ratio is used. **Nothing a game draws is ever off screen.**

`RenderSettings::window_to_internal` is the inverse of `placement`, beside it so
the two cannot drift, and `dim-play` reads the cursor through it — which makes
"a click lands on what is drawn under it" a property of one pair of functions
rather than of two that happen to agree.

### Added: a project chooses its own window and scaling

`dim-play` opened a fixed 1440×810 whatever the project said, which for a 1080p
game is smaller than the game; a packaged game has no command line, so
`--window` could not help a player.

```toml
[window]
size = [1280, 720]       # exactly this, or
fit = "monitor"          # the largest the screen holds at the game's shape

[render]
integer_upscale = false  # for art that is not pixel-locked
```

Declaring neither opens the game at its own `[render] resolution`, scaled to the
largest whole multiple the screen has room for when the game is pixel-locked:

| game | screen | window | frame |
|---|---|---|---|
| 1920×1080 | 1920×1080 | 1728×972 | 0.900× |
| 1920×1080 | 2560×1440 | 1920×1080 | 1.000× |
| 480×270 | 1920×1080 | 1440×810 | 3.000× |
| 1920×1080, `fit = "monitor"` | 1920×1080 | 1920×1080 | 1.000× |
| 1920×1080 | 1366×768 | 1229×691 | 0.640× |

The default keeps a tenth of the monitor back, because a window opened at
exactly the screen's size has its title bar pushed off the top and there is no
portable way to ask a window manager how much room its decorations want. It is
also what makes the arithmetic land on the sizes a person would have picked: a
480×270 game opens in exactly the 1440×810 window the fixed default used to
give it. `size` and `fit` are honoured as written, with nothing kept back.

`--window` still overrides for one run, and `dim-play` now says what it opened
and what scale the frame is drawn at — the first thing worth knowing when a
frame looks soft or an edge looks missing.

`Project::render_settings` is one door, for the same reason `script_host` is:
three callers built a `RenderSettings` from the engine's defaults with only the
resolution patched in, so `[render] integer_upscale` would have reached none of
them.

**Neither change moves the state hash**, and neither can. `Settings::presentation`
holds both keys, beside `Settings::game`, and a test asserts that declaring them
changes no part of the simulation's configuration. That line runs *through*
`[render]`: `resolution` is the contract, because a script unprojects a click
through it, and `integer_upscale` reaches nothing but a viewport call.

### Added: a run can be suspended and continued

```lua
if ui.clicked(continue_row) then app.resume() end
if ui.clicked(quit_row)     then app.suspend() end
```

`savefile.rs` already wrote a run as a canonical `scene.dim` plus a
`state.toml`, versioned, and loading it back gave an identical state hash. Only
the CLI could reach it. `dim-play` never called it, `Session` had no way to
replace its state, and no script could ask — so a shipped game had no Continue
row, and the two things a game could do instead were both refused when this was
first filed: the profile cannot restore the RNG streams, and replaying the log
from tick zero costs time proportional to the run and breaks every save on a
balance change.

Four calls, each acted on **between ticks**, like `scene.request_load` and
`app.quit()`:

| Call | What it does |
|---|---|
| `app.suspend()` | Writes the run out and stops. Carries the quit with it |
| `app.suspended()` | Whether a run this build can continue is waiting |
| `app.resume()` | Replaces the run with the suspended one, and consumes it |
| `app.discard_suspended()` | Throws it away, for a player starting a new run |

- **Where it lives:** `suspended/` beside the game's `profile.toml` — for a
  packaged game, the directory the executable sits in. Never inside a
  single-file build; `Project::writable()` is false for an archive and this does
  not try.
- **Written in two renames**, through a scratch directory. A run is two files,
  so writing them in place means a crash between them leaves the new tree beside
  the old numbers — a state that never existed and would load without complaint.
  The previous save is moved aside rather than deleted, so the window in which
  neither exists is one syscall wide and recoverable: the set-aside copy is the
  previous run, and that is what is continued.
- **Consumed on resume**, and only once the state is installed. A resume that
  failed half way leaves the run where it was.
- **A stale save reads as no save.** `app.suspended()` is false for one written
  by a different engine or format version, so a game never offers a Continue that
  fails when pressed, and the reason is reported as `DIM1002` at startup.
- **A replay and a headless run do none of it** and answer `app.suspended()`
  false. A recorded run that depended on a file beside it would reproduce only
  on the machine that made one. A script that asks anyway is told so, as the new
  `DIM1003`.
- **`dim state suspended`** reports the slot without opening a window, which is
  the only way to check a title-screen Continue on a machine with no display.

**Does not move the state hash** for any run that never suspends, and
structurally cannot: the requests and the answer live beside the quit flag on
the script host, out of `SimState` entirely, so there is no field for a later
change to start hashing. A test asserts that a run calling all four hashes
identically to one that calls none, and every replay fixture reproduces its
recorded hashes unchanged.

### Added: a recorded session that resumed says what it resumed from

A recording made across `app.resume()` is the second half of a run, and
replaying its frames against a fresh scene would reproduce something nobody
played — silently, which is the one outcome this engine exists to prevent. So an
input log gained two optional header lines:

```
resumed 7f3c…      the state hash the save restores to
from_tick 21       the tick the first recorded frame belongs to
```

Tick numbers in such a log are the **run's**, so a probe that says `tick 40`
means tick 40 of the run. The frames before the resume are dropped. The save is
kept beside the log as `<log>.save`, because the slot it came out of is emptied
by the resume, and `dim replay --from-save` — or nothing at all, which looks
beside the log — replays it. A log that says it resumed is **refused** rather
than replayed without that save, or against one whose hash disagrees.

A log that did not resume is byte-for-byte what it always was.

### Added: `DIM0508`, for state a script keeps in Lua

`on_ready` does not fire again after a resume — those nodes are already readied,
and re-firing initialisation would re-roll a run's starting loadout — so a node
handle cached in a file-scope `local` is nil afterwards and the next use of it
raises. The same was already true of a hot reload and of a rollback, and had
gone unnamed in both.

`dim script check` now names it, with the two spellings that work and
`-- @transient` for a local that is rebuilt every tick anyway. Three of this
repository's own replay fixtures used the hazardous idiom and now do not; their
hashes are unchanged, because looking a node up per tick produces the same
state. An `on_resumed` hook was considered and declined — it fires only on the
resumed run, so anything it wrote to state would be the divergence it was meant
to prevent. The reasoning is in `docs/ENGINE-GAPS.md`.

### Fixed: the save-format checks are reachable without parsing a scene

`SaveFile::usable` is the three checks `restore` already made — format, format
version, engine version — split out so a caller can ask whether a save is worth
offering before parsing its tree, and `savefile::read_header` reads the header
alone. No behaviour changed; `restore` calls `usable`.

### Added: a game names its own window and gives it an icon

```toml
# project.toml
[game]
name = "Confluence"
icon = "icon.png"
```

The window is created before any of the project has been read and no script can
reach it, so neither of these could come from the game at runtime; with
`--single` the manifest is folded into the executable at build time, so no
post-build step could edit it either. A game was called "Dimetric" and showed
the window manager's default icon, and there was nothing it could do about it.

- `dim build` writes both into the manifest and stages the icon. `--name`
  overrides `[game] name` for a build that ships under a different name.
- `dim-play` reads them when it creates the window — from the manifest beside it
  or appended to it, or from `project.toml` when run against a project with
  `--project`, so a developer sees the real title too. The icon is read through
  whatever the project is read through, so a single-file game finds its icon
  inside itself.
- `dim inspect` prints what a packaged game calls itself, which is the only way
  to check a title on a machine with no display.
- `dim new --name Confluence` writes a `project.toml` that says so, instead of
  leaving a new project to find out where the setting is.
- An icon that is missing, is not a readable PNG, or is outside the project is a
  new `DIM1201` **warning at build time**, where the file is in front of
  whoever chose it. The game ships without an icon rather than failing to build.
- Declaring neither gives exactly what every game got before: "Dimetric", and
  the default icon.

`Settings::game` is a separate type rather than two fields beside the tick rate,
because everything else in `project.toml` is the replay contract and neither of
these is. A title and an icon reach a window manager and nothing else; nothing
below the runtime reads them, and a test asserts that declaring them changes no
part of the simulation's configuration.

**Does not move the state hash**, and structurally cannot: neither value is
reachable from the simulation.

**Declined: the executable's own icon in a Windows file manager.** `dim build`
is handed a `dim-play` somebody else linked, so there is nothing left to compile
a resource into — only a finished PE file, which would have to be edited in
place. The reasoning is in `docs/ENGINE-GAPS.md`. The window and the taskbar
both show the game's name and icon on all three platforms; what is missing is
how the file looks before it is launched.

### Fixed: a script's write to a property the node never authored is typed

`node:set` guards a write against the value the node already carries, because
that value came through the parser and the parser had the schema. The guard was
therefore a guard against *changing* a type, not a schema check — and said so.
The hole that left:

```lua
-- A Sprite2D authored without a `region`, because the whole texture is wanted.
hero:set("region", { 0, 0, 24, 3 })
```

`region` has **no default**, so a sprite that did not author one carries no
value for it, so there was nothing to check the write against. The list of
integers went into state exactly as the scripting boundary made it. The
renderer coped, so nothing on screen said so. The save wrote
`region = [0, 0, 24, 3]` — integers, where an authored rect is
`[0.0, 0.0, 24.0, 3.0]` — the loader typed it from the schema, and the reloaded
state hashed **differently from the live state it came from**. Written as the
string `"[0, 0, 24, 3]"` instead, the save did not load at all.

Now the kind's declared type is the type of record when the node has no value,
and the write is reshaped to it or refused with the same `DIM0505`. The
properties this covers are the ones with no default and not required: `region`
on a sprite, `limits` on a camera, `cone_angle` on a light, `tile_size` on a
tile layer, and any such property of a project-declared kind.

Two things only the schema knows are checked along with the type, because both
produce a save that will not *load* rather than one that merely hashes
differently: an enum takes only its own variants, and a reference property only
its own prefix. And a key the kind does not declare at all is now refused with
`DIM0301` — the same code the loader raises for it — rather than stored to
become a file nothing can open. That last one is a **behaviour change**: a
script that wrote a misspelled property used to be allowed to.

`PropertyType::reshape` is `Value::reshape` handed a witness value of the
declared type, so there is one set of rules and the two routes to it cannot
drift. The registry reaches the write path as Lua app data on the script host,
beside the fonts and the module cache — **not** in `SimState`: a registry is
unchanging project data the run does not own, and a field in the state is a
field a later change starts hashing by accident.

**Moves the state hash:** only for a run in which a script wrote a property its
node did not carry, which is exactly the state that was wrong. Every replay
fixture in the repository reproduces its recorded hashes unchanged.

### Fixed: `dim replay` and the editor's preview built the wrong script host

Found while giving the registry one door to come through. `Project::script_host`
now builds the host for every caller that has a project, and three things it
supplies had each been forgotten somewhere:

- **`dim replay` used a hardcoded 60Hz** while stepping the simulation at the
  project's own rate, so `tick.dt` inside a script disagreed with the tick it
  was in. A project that is not 60Hz did not replay the run it recorded.
- **`dim replay` and the editor's playback had no fonts**, so `ui.measure`
  answered from nothing. A control sized from a measured string is hashed, so
  this is the same class of defect.
- **Nobody had the node kinds**, which is what the entry above needed.

**Moves the state hash:** for a replay or a preview of a project that is not
60Hz, or whose scripts measure text — in both cases towards the run that was
actually recorded. The example project is 60Hz and its fixtures are unchanged.

### Added: `dim build --single`, a game that is one file

`package.rs` argues for staging files beside the runtime and the argument is
right: the files a game ships are the files it was developed against, byte for
byte, which is what makes "it worked on my machine" checkable. But "send me the
game" means a folder of ~360 files that breaks the moment somebody drags the
executable out of it, and every fix on the game's side — a self-extractor, an
unpacker writing to a temporary directory — breaks exactly the property the
module protects.

So this is a **read path**, not a bundler, in four parts:

- **`dimetric_core::Source`** — read a file, list a directory, ask if something
  is there. A directory is one implementation; an archive is the other. Writing
  is deliberately absent, and `Project::writable()` is false for an archive, so a
  shipped game declines to re-import or save beside itself rather than writing
  into a path that is not a directory.
- **`dim build --single`** appends the staged files to a copy of the runtime,
  unmodified and uncompressed, with a 64-byte footer carrying the index offset,
  the file count and a BLAKE3 of the payload. The staged directory is kept: it is
  the thing that was verified.
- **`dim-play` prefers its own tail.** An explicit `--project` still wins, so a
  developer pointing at a changed project is not silently reading a stale copy.
- **`dim inspect <game> --verify`** lists what is inside and checks the hash. A
  build nobody can look into is the objection to bundling in the first place, so
  the index is text and the payload hashes.

The import became a pure function of bytes and settings, which it always claimed
to be: the PNG, Aseprite, font and audio decoders are handed bytes instead of
opening paths. Only the cache *write* needs a filesystem, and a shipped game
skips it — it just imported from bytes that cannot have gone stale.

Verified end to end: the example game folded onto a real `dim-play`, copied
somewhere with nothing beside it, draws a **byte-identical frame** to the folder
build. And a test in the engine's suite replays an input log against both and
asserts the hashes match tick for tick, which is the thing that would break first
if the archive ever stopped being a read path.

**Breaking:** `DiskScenes` is replaced by `Project::scenes()`, which returns a
reader over whatever the project reads from. `Catalog::scan`,
`dimetric_assets::import`, `Settings::load` and `Project::open` all keep working
on a directory; each has a `*_from` sibling that takes a `Source`.

Does not move the state hash — proven by the replay-equivalence test rather than
asserted.

### Added: `Sound.continuous`, so music survives a scene load

A floor change is `scene.request_load`, which swaps the whole tree, and the
speaker stops voices whose node has left it — correctly, because a projectile's
loop must not outlive the projectile. So a region theme restarted from bar one on
every floor, and "music continues across a level transition" is the default
expectation of every game with levels.

A voice from a `continuous` node is keyed by `(stream, bus)` rather than by its
node. The next scene's own copy of the same track finds it sounding and continues
it; a different track on the same bus replaces it, with the outgoing one fading,
which is a cross-fade using the mechanism the mixer already had; `node:stop()`
still stops it, by track rather than by a handle some unloaded scene's node
started. It is deliberately not recorded in the map the destroyed-node sweep
walks, because outliving its node is the point.

**A floor with no node asking for the track loses it**, on the tick the swap
happens. The proposal was a sweep after N ticks of nobody re-requesting; this asks
the *scene* instead — is there a live node that would ask for this track? — which
needs no interval to tune and leaves no window in which music plays over a floor
that did not want it.

### Added: `DIM1102`, when a playing voice's node id becomes a different node

The trap underneath that sweep, which the game hit and fixed in its generator.
"Destroyed" was decided by whether the id was still in the tree, so two scene
files reusing an id — easy when ids come from a generator — left a voice attached
to whatever landed on that id after the swap, and two tracks played at once. A
voice now carries the clip it is playing, so a node that is still there but is no
longer the one that asked is reported and its voice stopped.

Neither moves the state hash. Audio is never in it, by construction.

### Added: `app.quit()`, and the pause key reaches the game

**Nothing a script could call ended the process.** A Quit row in a menu could
not be built honestly — the game's said "Alt+F4". `app.quit()` asks whatever is
running the game to stop, read **between ticks** like a scene load, so the tick
finishes over the state it started with (I8).

It is deliberately not simulation state. A run in which somebody chose Quit must
hash the same as one where they closed the window, or a recorded session would
replay differently depending on how it ended; and a rollback must not un-ask, so
the flag is not snapshotted either. It lives beside the log lines, the sounds and
the events — out of `SimState` entirely, not in a field the hasher skips. A
headless run and a replay ignore it: there is nothing for them to quit, and
honouring it would let a script cut a recorded run short.

**`input.pressed("pause")` was never true.** The windowed runtime toggled its own
freeze on the `pause` *action* and returned before the press reached `Held`. Even
delivering it would not have helped: the freeze stops the ticks, so the script
would never have run to see it.

So the action belongs to the game and the freeze belongs to whoever is debugging.
`pause` is now an ordinary action, and the runtime's freeze is on the keyboard's
**Pause/Break** key, which is not in the bindings table and so cannot be taken by
a project.

While there: the key routing is one function both the window and a scripted
`--capture` run call. The capture path resolved keys itself, under a comment
claiming it went "through the same `press` the window uses" — it did not, which is
exactly how `pause` came to photograph correctly and play wrongly.

Does not move the state hash — there is a test asserting a run that quits hashes
the same as one that does not.

### Added: `TextureRect`, a control that draws a texture

The UI walk emitted two things — filled quads for `Panel` and `Button`, and
glyphs for a `Label` under a control — so an icon, a portrait or a rune on the
card holding it had to be a world-space `Sprite2D`, read at a world position
under the camera's projection and zoom. That is not where a control is, so every
button in a game built on this was a word.

`TextureRect` behaves as a `Control`, which means anchors, offsets and hit
testing come from the machinery every other control already uses; what it adds
is `texture`, `region`, `modulate`, `flip_h`/`flip_v` and `blend`. It is drawn by
the same UI walk in tree order, as the same `DrawItem` a filled panel produces
with a real sub-rectangle instead of the solid-white pixel — so it batches with
the panels around it and costs no pass, no pipeline and no hit test of its own.

It **fills** its rectangle rather than keeping the texture's aspect: a control's
size is what its anchors and offsets say, and a node that quietly ignored them
would not be a control.

An imported animation works in one. `animation`, `playing` and the
engine-written `frame` behave as on an `AnimatedSprite2D`, the sheet comes off
`texture` because that is what a control calls it, and `anim.play` and
`anim.restart` reach it.

A golden reference, `hud-texture-rect.png`: a card with an icon, the same icon
tinted, and a caption.

Does not move the state hash for an existing project. A `TextureRect` that
animates puts an entry in `anim` like any other animated node, so a scene that
has one hashes differently from the same scene without it — which is a new node,
not a change to an old one.

### Fixed: `scale` is drawn, and a rect can be written from a script

**`scale` was never read by the renderer.** It is a key every node has,
`tween.rs` writes it into the transform, and `extract.rs` built the quad from
the texture region's size and nothing else. So a scale tween ran, was hashed,
was snapshotted, and changed nothing on screen: a health bar could not shrink
(the game shipped seven on/off segments instead), and nothing could squash on a
hit.

A `Sprite2D` and an `AnimatedSprite2D` now multiply the quad by their **world**
scale, so scaling a parent scales its children. A negative component mirrors the
quad — the shader builds it from `(corner - 0.5) * size` — which makes `flip_h`
sugar for `scale = [-1, 1]`, and doing both cancels out.

A `TileLayer` and a `Label` are deliberately left alone: a tile layer's sprite
size and its cell step are different numbers and scaling one breaks the lattice,
and a label's glyph advance is in screen pixels so scaling it would grow the
letters without spreading them. Both want their own answer.

**A rect could not be written.** `region` is the other way to draw part of a
sprite and `extract` honours it, but it reaches a script as a *table* — not a
string — so the round-four fix for colours, angles, references and enums did not
reach it, and `node:set` refused the table `get` had just handed over. The type
guard now takes the shape each type actually arrives in:

```lua
node:set("region", node:get("region"))     -- the table get returns
node:set("region", "[0, 0, 8, 4]")         -- the form Display writes
node:set("region", { 0, 0, 8, 4 })         -- the same, as an array
```

A refusal now distinguishes the right type spelled wrongly (the characters and
the parser's complaint) from an actual type change (the renderer no longer
reading it, and the save no longer round-tripping), which the single message it
used to print could not.

Does not move the state hash. The scale was already in it and already tweened;
what changed is that it is drawn. Writing a rect was refused before, so no
existing script's state can differ.

### Added: `DIM1101`, when the system audio device was asked for and not obtained

`Device::open` fell back to the mock backend both when the device would not open
and when the `kira` feature was not compiled in, indistinguishably, and said
nothing either way. `dimetric-player`'s `sound` feature is off by default, so
the build line in most of this repo's own documentation produces a game that
resolves every `Sound` node, plays nothing, and reports nothing — which is
indistinguishable from a scene with no sounds in it, and sends somebody looking
for the bug in their scene.

Two warnings now, worded differently, because one is a machine to fix and the
other is a build flag: *"the system audio device would not open (…)"*, and
*"asked for the system audio device, but this runtime was built without the
`sound` feature — rebuild with `--features gui,sound`"*. They go into the
speaker's diagnostics, which the session already drains and the player already
prints.

`Device::Silent` stays quiet. A headless run and a test suite ask for no device
on purpose, and a warning there would put a line in every CI log and teach
everyone to ignore the one that matters.

Does not move the state hash. Audio is never in it.

### Added: `DIM0507`, for a write into a table that is a copy of script state

`self.bag.b = 2` lands in a temporary and is dropped. A script variable holding
a map or a list is converted to a fresh Lua table on every read, so a field
written through one goes nowhere — and it did so with no error, no warning and
no lint. It was the only failure in the engine that carried nothing at all,
which is what I9 forbids, and it cost an afternoon every time it was hit.

**The runtime guard that was asked for cannot work, and both reasons are now
tests** (`crates/dimetric-sim/tests/copied_table_facts.rs`):

- A metatable's `__newindex` fires only for a key that is **absent**. This
  engine already knew that — it is why `require`'s freeze is a proxy rather than
  a metatable. So a guard would catch `self.bag.b = 2` and miss
  `self.run.pending.at = 2`, where `at` is already in the table. That is one of
  the two cases reported.
- An empty proxy *would* see every write, and would destroy the variable: the
  host converts a Lua table back to an engine value with `lua_next`, which
  ignores `__pairs`, so `self.pending = pending` would convert to an empty map.

What makes a text scan right is that the difference is **syntactic**, which is
the one thing the runtime cannot see. `self.bag.b = 2` writes through a chained
access; `local p = self.pending` / `p[#p+1] = v` / `self.pending = p` does not —
and that second form is the pattern that works, the one the proposed
diagnostic's own wording recommends, and the one `examples/sorcerer` uses in
three places. A guard would have refused the engine's own example game.

`dim script check --determinism` now reports the lost write as a warning naming
the file, the line, the variable and the spelling that works. A test asserts
the example game is clean under it.

Does not move the state hash. The write went nowhere before and goes nowhere
now; what changed is that the engine says so.

### Fixed: under `Isometric`, a world-space `Label` laid its glyphs diagonally

`Projection::Isometric` is `screen = (x - y, (x + y) / 2)`. A label put each
glyph at `anchor + advance` as a **world** position, so the advance went through
that shear and every glyph landed one step down and to the right of the last.
`"160"` came out as three digits on a descending diagonal, and every number a
game wants over an isometric board was unreadable.

A label's anchor is world geometry and should project; its glyph advance is
typography and should not. That is the split the engine already makes for a
sprite — the shader projects a centre and adds the quad in screen space, which
is why isometric artwork stays upright instead of shearing into parallelograms —
and the comment in `sprite.wgsl` has said so all along. `DrawItem` now carries a
`screen_offset` applied after the projection, and a glyph's advance goes there.
Zero for a sprite, a tile and a panel, whose positions are entirely world
geometry.

Canvas-space labels are unaffected, and provably so: the canvas projection is a
scale and a translate whose scale *is* `canvas_pixel_scale`, so adding the
advance before or after it is the same arithmetic.

Two golden references, `board-label-isometric.png` and
`board-label-topdown.png`, are **byte-identical** — that is the assertion, not
a coincidence. Typography does not depend on the camera.

`Label.in_world` was offered with this and is declined; see
`docs/ENGINE-GAPS.md`. What it would restore is upright glyphs on a sheared
baseline, which is the defect with a property name on it rather than text lying
on the ground.

Does not move the state hash. Nothing here reaches the simulation.

### Fixed: `anim.play` did nothing on an `AnimatedSprite2D`, and a finished clip could not replay

Two halves, both silent.

`anim::advance` reads every `AnimatedSprite2D`'s `animation` property each tick
and switches back to it when the running clip differs. So `anim.play(node,
"burst")` set the clip and the next `Advance` phase put it back — on the one
node kind that *has* clips, the documented script route did nothing and
reported nothing. `docs/API.md` listed both routes and did not say one
cancelled the other.

`anim.play` now writes the node's `animation` property too, so the two routes
are one route: the node carries what it is playing, the renderer and the scene
cannot disagree, and a save round-trips to the same clip. Only on an
`AnimatedSprite2D` — any other kind has no such property, and adding one would
write a scene its own parser refuses (`DIM0301`), which is a worse bug than the
one being fixed.

And nothing restarted a clip. Setting `animation` to the value it already holds
is rightly not a restart, and `anim.play` of the running clip is documented as
not one either — so a non-looping clip that had finished stayed on its last
frame for the life of the node. A pooled effect sprite played its burst once
and then showed the final frame forever. **`anim.restart(node)`** is the
restart: frame zero, not finished, playing. It restarts only something already
playing; inventing an entry for a node with no playback would put a phantom
record in `anim`, which is hashed.

Moves the state hash for a script that called `anim.play` on an
`AnimatedSprite2D`, which until now was a call that did nothing.

### Changed: replay fixtures import their project's assets

No fixture could cover animation. Frame advance is simulation state and comes
from clips the importer produced, and the harness never imported — so
`project.clips()` handed back an empty map, a script playing a clip held frame
zero, and a fixture would have passed having tested nothing. The harness now
imports first. Timing is resolved at import against the project's tick rate, so
it is the same on every machine; atlas packing moves UVs, which are render-only
and not hashed. `tests/replay/script-animation` is the first fixture with an
asset in it.

### Fixed: `self.thing = nil` stored `false`

`SimState.vars` holds a `Value` and `Value` has no nil variant, so the write
stored the nearest thing it had. The read-back was `false`, the standard
`if self.thing ~= nil` guard passed, and the next line indexed a boolean. The
engine reported that correctly and the per-tick log swallowed it, so the only
symptom was a caption that would not hide.

Nil now **removes** the variable, which is what it does to every other table in
Lua, and `self.thing` reads back as `nil`. That needed no new variant in a
value type the scene format also uses: the honest round-trip for "no value" is
no entry. A node's whole table goes when its last variable does, because
`vars.len()` is hashed and a node left holding an empty table would make
"cleared the only variable" hash differently from "never had one".

Nil anywhere else — `node:set`, `profile.put`, a tween target — is now refused
with a diagnostic naming the one place it means something, rather than becoming
a `false` nobody wrote. `profile.clear(key)` was already the way to clear a
profile key.

Moves the state hash only for a script that wrote a nil, which was already
doing something other than what it said. The example game is one: `arena.lua`
clears `self.offered` after an upgrade is taken, so its state used to carry an
`offered = false` nobody wrote. `examples/sorcerer/tests/arena01.hashes` is
re-recorded from tick 40 on. The run itself is unchanged — every probe beside
it still passes, same five rooms, same twenty kills, same `Forking Arc`
evolution — only the state is now honest about a variable that is not there.

### Fixed: a quoted string in a `--assert` probe never matched

The expected value was everything after the operator, quotes included, so
`== "none"` compared six characters against four and could not pass. Quoting is
the natural thing to write, since every other literal in this toolchain is
TOML-ish.

Quotes are now optional and are not part of the value. Three things follow:

* an **empty string** can be asserted (`== ""`), which bare was two quote
  characters against nothing — the earlier finding, closed by the same change;
* a value keeps its **own spacing**, where rejoining a whitespace split with
  single spaces used to rewrite `"two  words"`;
* a `#` **inside quotes is not a comment**, so a colour can be asserted on —
  relevant now that a script can set one.

An unterminated quote is reported rather than read raw.

Neither moves a recorded hash. Probes are assertions over a replay, not part
of it.

### Added: `TileLayer.tile_size`, so a dimetric board can be drawn

`Projection::Isometric` is `screen = (x - y, (x + y) / 2)`. Work out what
tessellates under it and you get one answer: the grid's step must be **square**
and the tile sprite must be **2:1**. That is the standard dimetric arrangement,
and it is the one this engine is named for — and the single node kind for
drawing a floor could not express it.

`cell` was three things at once: the slice out of the sheet, the size drawn,
and the step between neighbours. A square step therefore forced a square
sprite. `cell = [32, 16]` steps twice as far as a tessellating neighbour, so
every other tile gaps and the board reads as scattered tiles; `cell = [16, 16]`
cuts 16×16 out of a 32×16 tile and draws half of each, so the floor collapses.
There is no third value.

`tile_size` is the sprite — the slice and the size it draws at, in pixels.
`cell` stays the step, in world units:

```toml
[[node]]
kind = "TileLayer"
cell = [16, 16]        # world step: square, as Isometric requires
tile_size = [32, 16]   # the sprite: 2:1, as a dimetric tile is
```

Absent means `cell`, so a layer that says nothing draws exactly as it did and
top-down projects are untouched.

Does not move the state hash. `cell` is read only by the renderer and written
only by the LDtk importer; nothing in the simulation reads either, so picking,
collision and `tiles.*` are unchanged — `tile_size` cannot move a cell, only
cover more of one.

### Added: a script can set a colour, and read any property back into itself

`modulate` is on every `Sprite2D` and `AnimatedSprite2D`, `node:get` hands it
back as `#rrggbbaa`, and there was no way to write one. The sandbox had a
`vec2` constructor and no `color` one, so the read gave you a string the write
would not take — and the refusal was *correct*: a script changing a property's
type stops the renderer drawing that node and stops a save round-tripping.
The gap was that a colour could not be expressed in Lua at all, which put
"eight sheets plus a tint per variant" out of reach and left a per-colour node
and a `visible` toggle in its place.

There is a `color` global now, beside `vec2`:

```lua
node:set("modulate", color.rgba(255, 143, 74, 255))
node:set("modulate", color.rgb(255, 143, 74))   -- opaque, said aloud
node:set("modulate", "#ff8f4aff")               -- what `get` hands back
```

Channels are bytes, and are **refused** outside `0..255` rather than clamped: a
clamp invents a colour the author did not write and says nothing about the
arithmetic that produced the 300. Alpha stays explicit, here as in a `.dim`
file — `#ff8f4a` is refused in both, and `color.rgb` is how to mean opaque.

**And the same defect four types over.** A colour, an angle, a reference and an
enum all reach a script as a string, because a string is what they are written
as — so `node:set(k, node:get(k))` was a type change on all four and was
refused. A string over one of those is now re-read as the property's own type,
by the same parser the scene format uses. Text that does not parse is refused
with the reason and the text quoted back, rather than stored. This widens what
counts as writing the same type; it does not widen what counts as a type, and a
number into a colour is still refused as before.

### Fixed: `node:set` on a reserved key shadowed it rather than writing it

Found while closing the gap above, and the same silence one key over. `get` and
`set` address the kind properties; `pos`, `rot`, `visible` and the rest live on
the node itself. So a write went into the property map beside the real one:

* `node:set("visible", false)` left the node on screen and said nothing — the
  renderer reads the node, not the prop.
* `node:set("rot", "45")` wrote a value the scene writer emitted as
  `rot = "45"` and the scene parser then refused as *"rot should be a angle"* —
  **a save written without complaint that could not be loaded.**

Both are refused now with `DIM0104`, naming the spelling that works
(`self.visible = false`), which has been there all along.

Moves the state hash **only for a script that was already broken**: one writing
a reserved key through `node:set` (which did nothing, or corrupted a save), or
writing a string into a colour, angle, reference or enum (which was refused).
A script doing neither hashes exactly as before — every replay fixture is
unchanged, and there is a new one, `tests/replay/script-colours`, that tints
from a script and hashes the result.

### Fixed: a project's settings reach what actually draws

Two halves of one defect: a setting that exists, is documented, and does not
reach the thing it names.

**`dim frame capture` ran on the engine's defaults.** `SimConfig` carries
`tick_rate`, `canvas` and `resolution`, all three declared in `project.toml`,
and the capture path built `SimConfig::default()`. The runtime got it right and
said why — *"a session that ran on the defaults while the project asked for
something else would produce recordings the project itself could not
replay."* The capture path just did not do it, and so every golden image taken
through `frame capture` photographed a differently-configured game from the one
the runtime plays. It survived because `Canvas::default()` is 320×180 and the
example project declares 320×180: a project whose settings happen to equal the
defaults is photographed correctly either way. The editor's playback had the
same defect and is fixed with it.

All five places that start this engine's simulation now go through one
`Project::sim_config()` rather than four hand-rolled copies and two defaults.

**`[render] resolution` never reached the renderer.** It reached
`SimConfig.resolution`, which is what `camera.to_world` unprojects a click
through, and not `RenderSettings.internal_resolution`, which is what draws. So
the number the simulation picked against and the number the renderer drew at
were independent and nothing kept them in step. A packaged game has no command
line, so the only resolution a shipped game could have was the default.
`internal_resolution` now defaults from the project.

`--internal` still overrides it for a one-off capture, and is no longer silent:
overriding re-creates exactly that disagreement for as long as the flag is
there, so it now warns with **`DIM0904`** naming both numbers and what it costs.
What it deliberately does *not* do is move the simulation's resolution to
match — that is hashed, and a flag that quietly changed the run would make a
recording replayable only by someone who passed the same flag.

Does not move the state hash for a project that declared nothing, which is
every golden fixture. **It does move it for a project that declared a
`tick_rate`, `canvas` or `resolution` other than the default and has
recordings made through `dim frame capture` or the editor's playback** — those
were taken against the wrong configuration and have to be retaken. A recording
made by the runtime or by `dim replay` is unaffected; both already read the
project.

### Fixed: a save dropped `resolution`, so a resumed run was a different run

`SimState.resolution` is in the state hash — a script unprojects a click
through it, so it decides which cell was clicked — and `savefile.rs` did not
write it. On load it fell back to the default `(480, 270)`, and the restored
state hashed differently from the state that was saved. The load succeeded and
said nothing.

It only bit a project that asked for a resolution other than the default, which
is why it survived: every round-trip test in the suite ran on
`SimConfig::default()`, where a dropped field restores to the value it had
anyway. A field that equals its default round-trips whether it is written or
not. There are now two tests on a project's own settings — one on the hash, one
naming `canvas` and `resolution` individually so the next field forgotten here
fails on the field rather than on an opaque mismatch.

The save carries `resolution` beside `canvas` now. The field defaults to
`(480, 270)` when absent, so a save written before this loads exactly as it did
before — no format bump.

Does not move the state hash. It stops a save from moving it.

### Fixed: `dim script check` can resolve `require`, and checks a project

Two defects and an absence, all in the same command, all of which made the
check quieter than the truth.

**`require` could not resolve.** The command built a bare `LuaHost` and loaded
one file into it, while the runtime registers every script's source before
loading any of them. So the first `require` line raised `unknown module`, the
command exited non-zero, and everything after that line went unexamined. A
script that pulls in a module — which is every script worth linting — could not
be checked at all. It now seeds the host's module registry from
`project.load_scripts()`, the way the runtime does, so a module path resolves
in the checker exactly as it will at tick 0. Registering is not running: a
single-file check still loads only the file it was asked about.

**A load failure suppressed the lint.** The determinism scan is a text scan
with no parser behind it, so it never depended on the load succeeding — but it
sat behind a `?`, and a syntax error on line 3 hid a float on line 2. The lint
now runs either way, and a file that does not parse reports its syntax error
*and* its hazards. A file that does not parse still fails the command, so a
check in CI has an exit status to read; the hazards go out with the failure
rather than being swallowed by it.

**There was no way to ask about a project.** `dim script check --determinism`
now takes no path and checks every script under `scripts/`, in sorted order.
The per-file form still works. A caller who has to write the loop is the caller
who skips the file that mattered — which this engine's own example game
demonstrated: `examples/sorcerer/scripts/arena.lua` requires the spellbook, so
a per-file check on it had never got past line 12, and the `pairs()` on line 53
had never been reported in the project's life. It is a whole-table copy and
safe, and now says so with `-- @ordered`.

`dim script write` had the same bare host and is fixed with it: a script that
requires a module can now be written.

The command's JSON result changed shape. It was `{ ok, path, hazards }`; it is
now `{ checked, files, hazards }`, where `checked` is the paths examined and
`files` carries a `{ path, parses, error, hazards }` entry for each. `ok` is
gone from the body — the envelope has always carried one.

Does not move the state hash. `LuaHost::register` is new and public, but it
only writes into the module registry that `load` already wrote into; nothing
about stepping changed.

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
