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

/// A named range of frames over a plain strip.
///
/// Aseprite documents carry tags, and those become clips. A PNG carries
/// nothing, so a strip exported from anything else — a generated placeholder
/// sheet, a procedural atlas, art assembled by a script — imported as a single
/// unnamed clip and `anim.play(node, "walk_se")` could not reach it.
///
/// This is the `.meta` doing the job its own doc comment already describes:
/// saying the things the file itself does not. It can say "this PNG is a strip
/// of 70 frames"; now it can say "frames 0 to 3 are called idle_ne", which is
/// exactly as much a fact the PNG does not carry.
///
/// Ranges rather than counts, so an off-by-one is a diagnostic naming the
/// clip rather than a silent shift of every frame after it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ClipRange {
    /// What `anim.play` calls it.
    pub name: String,
    /// First frame, inclusive.
    pub from: u32,
    /// Last frame, inclusive.
    pub to: u32,
    /// Whether it repeats. One-shot for an attack, looping for an idle.
    #[serde(default = "yes")]
    pub looping: bool,
    /// How long each frame is held, overriding the sheet's `frame_ms`.
    ///
    /// An attack is usually faster than an idle, and the alternative is
    /// importing the same sheet twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ms: Option<u32>,
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
    /// Named clips over the strip, if the sheet declares any.
    ///
    /// Empty means the old behaviour: one clip called `default` covering every
    /// frame, which is what a strip with no names can usefully be.
    #[serde(default, rename = "clip", skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<ClipRange>,
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
            clips: Vec::new(),
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
        // Clips last, because an array of tables swallows every key after it
        // in TOML — a scalar written below one would silently become part of
        // the clip rather than of the sheet.
        for clip in &self.clips {
            out.push_str("\n[[clip]]\n");
            out.push_str(&format!("name = {:?}\n", clip.name));
            out.push_str(&format!("from = {}\n", clip.from));
            out.push_str(&format!("to = {}\n", clip.to));
            if !clip.looping {
                out.push_str("looping = false\n");
            }
            if let Some(ms) = clip.frame_ms {
                out.push_str(&format!("frame_ms = {ms}\n"));
            }
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
        let frames = get_int("frames", 1);
        let clips = parse_clips(&doc, frames)?;
        Ok(ImportSettings {
            id: AssetId::parse(&id_text).map_err(|e| MetaError::BadId(e.to_string()))?,
            source_hash: get_str("source_hash"),
            nearest: get_bool("nearest", true),
            atlas: get_bool("atlas", true),
            frame_ms: get_int("frame_ms", default_frame_ms()),
            font_size: get_int("font_size", default_font_size()),
            charset: get_str("charset").unwrap_or_else(default_charset),
            frames,
            clips,
        })
    }

    /// Anything about the declared clips worth saying that is not fatal.
    ///
    /// Overlap is the only one, and it is a warning rather than an error on
    /// purpose. Reusing frames across clips is a real technique — an idle and
    /// a breathe sharing a couple of frames — and Aseprite tags may overlap
    /// too, so refusing it would be stricter than the tool the rest of this
    /// pipeline mirrors. But writing `0..3` then `3..7` when `4` was meant is
    /// exactly the off-by-one ranges are here to catch, so it is worth saying
    /// out loud.
    pub fn clip_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, a) in self.clips.iter().enumerate() {
            for b in self.clips.iter().skip(i + 1) {
                if a.from <= b.to && b.from <= a.to {
                    out.push(format!(
                        "clips {:?} ({}..={}) and {:?} ({}..={}) share frames; \
                         intended if they reuse art, an off-by-one if not",
                        a.name, a.from, a.to, b.name, b.from, b.to
                    ));
                }
            }
        }
        out
    }
}

/// Read `[[clip]]` blocks, checking them against the strip they describe.
fn parse_clips(doc: &toml_edit::DocumentMut, frames: u32) -> Result<Vec<ClipRange>, MetaError> {
    let Some(array) = doc.get("clip").and_then(|i| i.as_array_of_tables()) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<ClipRange> = Vec::new();
    for table in array {
        let name = table
            .get("name")
            .and_then(|i| i.as_value())
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            return Err(MetaError::BadClip(
                "a clip needs a `name`; it is what `anim.play` asks for".to_string(),
            ));
        }
        let int = |key: &str| {
            table
                .get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_integer())
                .and_then(|v| u32::try_from(v).ok())
        };
        let (Some(from), Some(to)) = (int("from"), int("to")) else {
            return Err(MetaError::BadClip(format!(
                "clip {name:?} needs `from` and `to`, as whole frame numbers"
            )));
        };
        if to < from {
            return Err(MetaError::BadClip(format!(
                "clip {name:?} runs from {from} to {to}, which is backwards"
            )));
        }
        // Inclusive, so the last usable index is `frames - 1`. Stated in the
        // message because off-by-one is precisely what this catches.
        if to >= frames {
            return Err(MetaError::BadClip(format!(
                "clip {name:?} ends at frame {to} and the strip declares {frames} \
                 frames, so the last one is {}",
                frames.saturating_sub(1)
            )));
        }
        if out.iter().any(|c| c.name == name) {
            return Err(MetaError::BadClip(format!(
                "two clips are both called {name:?}; `anim.play` could not tell them apart"
            )));
        }
        out.push(ClipRange {
            name,
            from,
            to,
            looping: table
                .get("looping")
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            frame_ms: int("frame_ms"),
        });
    }
    Ok(out)
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
    /// A `[[clip]]` block does not describe a usable range.
    #[error("{0}")]
    BadClip(String),
    /// The file is there and could not be read at all.
    #[error("import settings could not be read: {0}")]
    Unreadable(String),
}

/// Hash a source file's contents.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
