//! Projection, draw ordering and sprite batching.
//!
//! # What is here and what is not
//!
//! The parts of a 2D renderer that are pure computation — the projection, the
//! sort key, the batcher — are implemented and tested. The `wgpu` backend that
//! turns a batch list into draw calls is **not in this build**; see the
//! milestone table in the README.
//!
//! The split is deliberate rather than incidental. Everything in this crate can
//! be checked on a machine with no GPU, which is what makes a golden-image test
//! meaningful later: when the backend arrives, any difference it produces is
//! the backend's, because the ordering and grouping are already pinned by
//! tests.
//!
//! Nothing here writes simulation state (I7). Floats are allowed below this
//! line for exactly that reason.

#![warn(missing_docs)]

pub mod batch;
pub mod projection;
pub mod sort;

pub use batch::{build, Batch, Blend, DrawItem};
pub use projection::{Camera, Projection};
pub use sort::SortKey;
