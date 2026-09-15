//! The asset pipeline: source files in, cached artifacts out.
//!
//! # The shape of it
//!
//! Sources live in `assets/`. Import settings live in a sibling `.meta`, which
//! is TOML and therefore hand- and agent-editable — an import setting that can
//! only be changed through a GUI is one an agent cannot change at all. Imported
//! artifacts are cached in `.import/`, keyed by the content hash of the source
//! they came from, and never committed.
//!
//! Identity is the part worth being careful about. The id is stored in the
//! `.meta` rather than derived from the path, so renaming a file is free: the
//! id travels with the settings and every scene referencing it keeps working.
//!
//! # What happens at import rather than at runtime
//!
//! Two things, both because determinism depends on it.
//!
//! Frame durations are milliseconds in an Aseprite document and ticks in the
//! simulation. [`ms_to_ticks`] runs here, once, and the tick count is what gets
//! cached. Converting on load would make frame advance depend on whatever tick
//! rate happened to be configured that session, and an activation frame is
//! gameplay rather than decoration.
//!
//! Atlas packing runs here too. The layout is deterministic given the same
//! inputs, so caching it costs nothing and a golden image of a differently
//! packed atlas is a different image.
//!
//! # What is not here
//!
//! No filesystem watcher. [`Catalog::changed_since`] answers hot reload's
//! question — which assets differ — and the host decides when to ask it, which
//! is at a tick boundary and never inside one.

#![warn(missing_docs)]

pub mod aseprite;
pub mod cache;
pub mod clip;
pub mod font;
pub mod image;
pub mod ldtk;
pub mod meta;
pub mod sheet;

pub use aseprite::{clip_from_range, Aseprite, Playback};
pub use cache::{import, Artifact, Catalog, Entry, Imported, ASSETS_DIR, MAX_SHEET_WIDTH};
pub use clip::{ms_to_ticks, Clip, Frame};
pub use font::{bake, Font, FontError, Glyph, DEFAULT_CHARSET};
pub use image::{decode_png, encode_png, Image, ImageError};
pub use ldtk::{LdtkError, Level, Tile, TileLayer};
pub use meta::{content_hash, ImportSettings, MetaError, SourceKind, IMPORT_DIR, META_EXTENSION};
pub use sheet::{pack, Placement, Sheet};
