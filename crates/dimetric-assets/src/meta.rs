//! Asset identity: what a source file is, and the `.meta` beside it.
//!
//! Identity lives in the sidecar rather than being derived from the path. That
//! is the whole reason renaming a file is free: the id travels with the
//! settings, and every scene referencing it keeps working.

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
    /// An audio clip, in any container the backend decodes.
    Audio,
    /// An LDtk level.
    Ldtk,
    /// A font, baked to a glyph page and a table of integer metrics.
    Font,
}

impl SourceKind {
    /// Classify a path by extension.
    pub fn of(path: &Path) -> Option<SourceKind> {
        Some(
            match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "png" => SourceKind::Png,
                "ase" | "aseprite" => SourceKind::Aseprite,
                "ogg" | "wav" => SourceKind::Audio,
                "ldtk" => SourceKind::Ldtk,
                "ttf" | "otf" => SourceKind::Font,
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
    /// Animation frames the image holds, side by side.
    ///
    /// One for a still. An Aseprite document says how many it has, so this is
    /// for the other common case: a sprite strip exported as a plain PNG, which
    /// nothing in the file itself identifies as a strip.
    #[serde(default = "one")]
    pub frames: u32,
    /// Pixel size a font is baked at.
    ///
    /// A font is rasterised once, at import, at this size — see
    /// [`crate::font`] for why measurement cannot happen at runtime. Drawing
    /// it larger scales the bitmap, which is what a pixel-art engine wants;
    /// import the file twice if you need two sizes.
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    /// Characters to bake.
    ///
    /// Printable ASCII by default. Every glyph baked is atlas space spent
    /// whether the game draws it or not, so a project wanting more says so
    /// rather than the engine guessing.
    #[serde(default = "default_charset")]
    pub charset: String,
    /// How long each frame of a declared strip is held, in milliseconds.
    ///
    /// Milliseconds here and ticks in the cache: the conversion happens once,
    /// at import, against the project's tick rate, for the same reason an
    /// Aseprite duration does.
    #[serde(default = "default_frame_ms")]
    pub frame_ms: u32,
}

fn default_frame_ms() -> u32 {
    100
}

fn default_font_size() -> u32 {
    16
}

fn default_charset() -> String {
    crate::font::DEFAULT_CHARSET.to_string()
}

fn one() -> u32 {
    1
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
            frames: 1,
            frame_ms: default_frame_ms(),
            font_size: default_font_size(),
            charset: default_charset(),
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
        if self.frames > 1 {
            out.push_str(&format!("frames = {}\n", self.frames));
            out.push_str(&format!("frame_ms = {}\n", self.frame_ms));
        }
        // Only for fonts: writing a size and a charset into every PNG's
        // sidecar would be noise in a file people are meant to read.
        if self.font_size != default_font_size() || self.charset != default_charset() {
            out.push_str(&format!("font_size = {}\n", self.font_size));
            out.push_str(&format!("charset = {:?}\n", self.charset));
        }
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
        let get_int = |key: &str, default: u32| {
            doc.get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_integer())
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };
        let id_text = get_str("id").ok_or(MetaError::MissingId)?;
        Ok(ImportSettings {
            id: AssetId::parse(&id_text).map_err(|e| MetaError::BadId(e.to_string()))?,
            source_hash: get_str("source_hash"),
            nearest: get_bool("nearest", true),
            atlas: get_bool("atlas", true),
            frames: get_int("frames", 1),
            frame_ms: get_int("frame_ms", default_frame_ms()),
            font_size: get_int("font_size", default_font_size()),
            charset: get_str("charset").unwrap_or_else(default_charset),
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
