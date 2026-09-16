//! Projection, draw ordering and sprite batching.
//!
//! # Shape
//!
//! Two halves, on purpose. [`extract`] turns a scene into a sorted, batched
//! frame and is pure computation: it can be tested on a machine with no GPU, so
//! ordering and grouping are pinned before a driver is involved. [`gpu`] turns
//! that frame into draw calls.
//!
//! Windowed and headless rendering run the same passes and differ only in what
//! they are handed as a target. If they diverged, an agent's screenshot would
//! stop being evidence about what a person sees.
//!
//! Nothing here writes simulation state (I7). Floats are allowed below this
//! line for exactly that reason, and the conversions in and out are marked.

#![warn(missing_docs)]

pub mod atlas;
pub mod batch;
pub mod capture;
pub mod extract;
pub mod gpu;
pub mod projection;
pub mod settings;
pub mod sort;
pub mod text;

pub use atlas::{Atlas, Region, Source};
pub use batch::{batch_group, build, Batch, Blend, DrawItem};
pub use capture::{headless_instance, read_png, write_png, Capture};
pub use extract::{extract, extract_with_canvas, Frame, Interpolation, LightItem};
pub use gpu::{GpuError, Renderer, Target};
pub use projection::{Camera, Projection};
pub use settings::RenderSettings;
pub use sort::SortKey;
