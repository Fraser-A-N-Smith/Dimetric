//! Asset identity, import settings, and the conversions that have to happen at
//! import time rather than at runtime.
//!
//! # What is here and what is not
//!
//! Asset identity, content hashing, `.meta` files and the millisecond-to-tick
//! conversion are implemented. Decoding PNG and Aseprite files, packing
//! atlases and watching the filesystem are **not in this build**.
//!
//! The conversion is here rather than in the decoder on purpose: it is the part
//! that determinism depends on, and it is testable without a single byte of
//! image data.

#![warn(missing_docs)]

use std::path::Path;

use dimetric_core::AssetId;
use serde::{Deserialize, Serialize};

/// Where imported artifacts are cached.
///
/// Content-addressed and never committed: the cache is reproducible from the
/// sources, and checking in derived files is how a repository starts
/// disagreeing with itself.
pub const IMPORT_DIR: &str = ".import";
/// Extension of the sidecar carrying import settings.
pub const META_EXTENSION: &str = "meta";

/// A source file the project can import.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    /// A still image.
    Png,
    /// An Aseprite document, whose tags become animation clips.
    Aseprite,
    /// An audio clip.
    Ogg,
    /// An LDtk level.
    Ldtk,
}

impl SourceKind {
    /// Classify a path by extension.
    pub fn of(path: &Path) -> Option<SourceKind> {
        Some(
            match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "png" => SourceKind::Png,
                "ase" | "aseprite" => SourceKind::Aseprite,
                "ogg" => SourceKind::Ogg,
                "ldtk" => SourceKind::Ldtk,
                _ => return None,
            },
        )
    }
}

/// Settings for one source file, stored in a sibling `.meta`.
///
/// Text, and hand- and agent-editable. An import setting that can only be
/// changed through a GUI is one an agent cannot change at all.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ImportSettings {
    /// Permanent id for this asset.
    ///
    /// Lives in the `.meta` rather than being derived from the path, which is
    /// what makes renaming a file free: the id travels with the settings, and
    /// every scene referencing it keeps working.
    pub id: AssetId,
    /// Hash of the source the cache was built from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// Sample with nearest-neighbour rather than linear filtering.
    #[serde(default = "yes")]
    pub nearest: bool,
    /// Pack into a shared atlas.
    #[serde(default = "yes")]
    pub atlas: bool,
}

fn yes() -> bool {
    true
}

impl ImportSettings {
    /// Default settings for a new asset.
    pub fn new(id: AssetId) -> ImportSettings {
        ImportSettings {
            id,
            source_hash: None,
            nearest: true,
            atlas: true,
        }
    }

    /// Render as TOML.
    pub fn to_text(&self) -> String {
        let mut out = format!("id = \"{}\"\n", self.id);
        if let Some(hash) = &self.source_hash {
            out.push_str(&format!("source_hash = \"{hash}\"\n"));
        }
        out.push_str(&format!("nearest = {}\n", self.nearest));
        out.push_str(&format!("atlas = {}\n", self.atlas));
        out
    }

    /// Parse from TOML.
    pub fn parse(text: &str) -> Result<ImportSettings, MetaError> {
        let doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| MetaError::Malformed(e.to_string()))?;
        let get_str = |key: &str| {
            doc.get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let get_bool = |key: &str, default: bool| {
            doc.get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_bool())
                .unwrap_or(default)
        };
        let id_text = get_str("id").ok_or(MetaError::MissingId)?;
        Ok(ImportSettings {
            id: AssetId::parse(&id_text).map_err(|e| MetaError::BadId(e.to_string()))?,
            source_hash: get_str("source_hash"),
            nearest: get_bool("nearest", true),
            atlas: get_bool("atlas", true),
        })
    }
}

/// Why a `.meta` file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetaError {
    /// Not well-formed TOML.
    #[error("import settings are not valid TOML: {0}")]
    Malformed(String),
    /// No `id` key.
    #[error("import settings need an `id`, so the asset keeps its identity across renames")]
    MissingId,
    /// The id was not a valid asset id.
    #[error("{0}")]
    BadId(String),
}

/// Hash a source file's contents.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Convert a frame duration in milliseconds to whole ticks.
///
/// **This runs at import time, never at runtime.** The artist's timings are in
/// milliseconds and the simulation counts ticks; converting on load would make
/// frame advance depend on whatever tick rate happened to be configured that
/// session, and a hitbox activation frame is gameplay, not decoration.
///
/// Rounds half away from zero and never returns zero, because a clip with a
/// zero-length frame advances infinitely fast and hangs the frame walker.
pub fn ms_to_ticks(milliseconds: u32, tick_rate: u32) -> u32 {
    debug_assert!(tick_rate > 0, "a tick rate of zero has no meaning");
    let numerator = milliseconds as u64 * tick_rate as u64;
    let ticks = (numerator * 2 + 1000) / 2000;
    ticks.max(1) as u32
}

/// One frame of an imported clip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Index into the sheet.
    pub index: u32,
    /// How long the frame is held, in ticks.
    pub ticks: u32,
    /// Event dispatched to scripts when this frame begins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// A named animation clip, imported from an Aseprite tag.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Clip {
    /// Tag name from the source document.
    pub name: String,
    /// Frames, in order.
    pub frames: Vec<Frame>,
    /// Whether the clip repeats.
    pub looping: bool,
}

impl Clip {
    /// How many ticks one pass through the clip takes.
    pub fn duration_ticks(&self) -> u32 {
        self.frames.iter().map(|f| f.ticks).sum()
    }

    /// Which frame is showing at `tick` into the clip.
    pub fn frame_at(&self, tick: u32) -> Option<&Frame> {
        if self.frames.is_empty() {
            return None;
        }
        let total = self.duration_ticks();
        let t = if self.looping && total > 0 {
            tick % total
        } else {
            tick.min(total.saturating_sub(1))
        };
        let mut elapsed = 0;
        for frame in &self.frames {
            elapsed += frame.ticks;
            if t < elapsed {
                return Some(frame);
            }
        }
        self.frames.last()
    }
}
