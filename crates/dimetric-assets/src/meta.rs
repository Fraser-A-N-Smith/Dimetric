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
