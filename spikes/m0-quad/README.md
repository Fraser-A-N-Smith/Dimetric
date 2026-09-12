# M0 — wgpu de-risking spike

**Throwaway code.** The design document is explicit that this gets deleted once
it has done its job. Nothing here should be copied into `dimetric-render`; the
findings below should.

Its own cargo workspace, excluded from the engine's, so `cargo test --workspace`
at the repository root never builds wgpu or winit.

```sh
cargo run -- --headless --out frame.png            # what CI runs
cargo run -- --headless --projection isometric     # the 2:1 shear
cargo run                                          # a window, for a person
WGPU_BACKEND=gl cargo run -- --headless            # force a backend
```

## What it draws

One textured quad, 64 world units across, sampling a generated checkerboard —
generated rather than loaded, because M0 is about the graphics stack and an
image decoder would only add a second thing that can fail. It exercises a vertex
buffer, an index buffer, a uniform buffer, a sampled texture, a sampler, a bind
group and a render pass, which is every piece the real sprite batcher will need.

The headless and windowed paths share one `draw` call. That was the point: if
they diverge, an agent's screenshot stops being evidence about what a human
sees.

## Findings

**wgpu comes up on a machine with no GPU.** Linux needs Mesa's lavapipe
(`mesa-vulkan-drivers`) installed; the adapter then reports as
`llvmpipe … via Vulkan (Cpu)`. macOS has Metal and Windows has WARP without any
extra installation. A headless renderer is therefore testable in CI, which is
what makes the golden-image job planned for M3 possible at all.

**The engine's projection matrix was wrong, and every unit test passed.**
`Projection::matrix()` is row-major and WGSL matrices are column-major, so the
spike transposed it on the way to the GPU. The transpose of a 2:1 shear is
another shear with the same determinant, so the picture stayed plausible — a
diamond of exactly the right area, just the wrong diamond. Nothing had ever
checked that `matrix()` agreed with `to_screen()`, because the tests only
exercised the latter. `dimetric-render` now has that test.

The lesson for M3 is narrower than "test the matrix": **a renderer check that
only asserts something was drawn will pass on a wrong transform.** The spike
asserts the quad's bounding box, which is 128×128 top-down and 256×128 under
the shear, and that is what caught it.

**Clear colours and texture bytes are treated differently.** With an sRGB
target format, a clear colour is given in linear space and stored encoded —
`Color { r: 0.05, .. }` reads back as 63, not 13 — while bytes written through
`write_texture` are stored verbatim. The first draft of the spike's own check
hard-coded the wrong background and reported 100% coverage of a frame that was
mostly background.

**Output is byte-identical between runs on one adapter.** Whether it is
identical *across* platforms could not be settled here: this machine has only
lavapipe, and the GL backend has no loadable driver. The CI job uploads each
platform's frames as artifacts so that question gets a real answer the first
time it runs. Until then, assume the M3 golden-image job needs a per-platform
baseline or a tolerance rather than exact equality.

**wgpu's API moves between releases.** Written against a recent-but-not-current
memory of the API, this needed fixes in nine places for wgpu 30: `SurfaceError`
became `CurrentSurfaceTexture`, `bind_group_layouts` takes `Option`s,
`push_constant_ranges` became `immediate_size`, `multiview` became
`multiview_mask`, `present` moved to `Queue`, and both `RequestAdapterOptions`
and `DeviceDescriptor` gained fields. The descriptors that implement `Default`
are now built with struct-update syntax so field additions do not break the
build. §2's argument against Bevy — that quarterly API churn is hostile to agent
operation — applies to wgpu too, just more slowly. Pin the version and budget
for the upgrades.

**The fixed-timestep accumulator works against a real display.** The windowed
loop uses `dimetric_host::run::Accumulator` rather than a copy, so this is a
real integration check. It is the one place allowed to read a clock (I5), and
what crosses into the simulation is a tick count and nothing else.

## Not covered

The windowed path is built and type-checked on every platform in CI but only
*run* by hand — CI runners have no display, and the spike exits with a clear
message saying so rather than hanging. Someone should open the window on each
of the three platforms once before M3 starts.

Nothing here touches vsync behaviour, multiple windows, HiDPI scaling, or
surface recreation on a GPU reset. Those are M3's problems, and the design
document is right that they are cheaper to meet with a working reference than
without one.
