# Golden-image fixtures

Each scene under `scenes/` is rendered headless and compared against the PNG of
the same name under `references/`. CI does this on Windows, macOS and Linux.

```sh
cargo test -p dimetric-host --test golden            # compare
DIMETRIC_BLESS=1 cargo test -p dimetric-host --test golden   # regenerate
```

Read the diff before committing a regenerated reference. A changed image means
either you changed rendering on purpose, or you broke it — and the point of the
fixture is that you have to decide which.

## What each scene covers

**`room.dim`** — top-down. Tiles, two overlapping sprites so y-sorting is
visible, a rotated sprite, and an additive one. One frame that exercises most of
what can silently go wrong.

**`room-lit.dim`** — the same room under the 2:1 shear, in the dark, with a
radial torch and a blue cone light.

## Why the isometric fixture uses diamond tiles

The projection moves a sprite's **position** and draws its quad upright. That is
how the genre works: the artwork is already drawn in projection, and shearing
the quad as well turns every character into a parallelogram.

So isometric tile art is diamond-shaped, at the 2:1 ratio the projection places
it on — which is what `assets/tilesets/iso.png` is. Square tiles under the shear
tessellate into columns with gaps. That is the artwork being wrong, not the
engine.

## Why the comparison has a tolerance

The M0 spike established that output is byte-identical between runs on one
adapter, and could **not** establish that it matches across platforms — that
machine had only a software rasteriser. GPUs agree on a triangle's interior and
differ at its edges, so an exact comparison would pass for whoever generated the
references and fail for everyone else.

The thresholds in `crates/dimetric-host/tests/golden.rs` are provisional. Once
CI has reported what three platforms actually produce, tighten them to what the
evidence supports.

## A caveat about these files

`dim scene fmt` does not preserve hand-written comments — it regenerates the
file from the parsed model. That is why the explanation above lives here rather
than in the scenes, and it is a bug: §6.1 of the design document lists comment
preservation as one of the four reasons TOML was chosen. Until it is fixed,
anything you want to say about a scene has to live outside it.
