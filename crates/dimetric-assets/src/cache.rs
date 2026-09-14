//! The import pipeline: what is in `assets/`, and what it became in `.import/`.
//!
//! # The shape of it
//!
//! A [`Catalog`] is the list of source files and the `.meta` beside each one.
//! Scanning is cheap — it reads bytes to hash them and nothing else — so it is
//! also what hot reload is built on: rescan, compare hashes, reimport what
//! moved.
//!
//! [`import`] turns sources into artifacts under `.import/`, keyed by the
//! content hash of the source they came from. The cache is derived and never
//! committed; deleting it costs a reimport and nothing else.
//!
//! # Why identity lives in the `.meta`
//!
//! An id derived from the path would break every scene the moment someone
//! renamed a folder. An id stored in the sidecar travels with the file. The
//! first import derives one from the asset's name so that a project whose
//! `.meta` files were never committed still comes back with the ids its scenes
//! expect, but once written the stored id wins, and renaming is free.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dimetric_core::AssetId;

use crate::clip::Clip;
use crate::image::{decode_png, encode_png, Image, ImageError};
use crate::meta::{content_hash, ImportSettings, SourceKind, IMPORT_DIR, META_EXTENSION};
use crate::sheet::{pack_framed, Framed, Sheet};

/// Directory sources are read from.
pub const ASSETS_DIR: &str = "assets";

/// Largest sheet the importer packs into.
///
/// Well inside the 2048 that downlevel GPU limits guarantee, so a project that
/// renders on a developer's machine also renders on a modest laptop.
pub const MAX_SHEET_WIDTH: u32 = 2048;

/// One source file and what is known about it.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    /// Project-relative path, e.g. `assets/sprites/hero.png`.
    pub path: String,
    /// What a scene calls it: the path under `assets/` without its extension.
    pub name: String,
    /// What kind of file it is.
    pub kind: SourceKind,
    /// Settings from the sidecar.
    pub settings: ImportSettings,
    /// Hash of the bytes currently on disk.
    pub hash: String,
}

impl Entry {
    /// Whether the cache is behind the source.
    pub fn is_stale(&self) -> bool {
        self.settings.source_hash.as_deref() != Some(self.hash.as_str())
    }
}

/// Every source file in a project, with its settings.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Catalog {
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl Catalog {
    /// Scan a project's `assets/` directory.
    ///
    /// Unreadable files are skipped rather than fatal: one bad file should not
    /// stop a project opening, and the missing asset reports itself where it is
    /// used.
    pub fn scan(root: &Path) -> Catalog {
        let mut paths = Vec::new();
        collect(&root.join(ASSETS_DIR), root, &mut paths);
        paths.sort();

        let mut entries = BTreeMap::new();
        for path in paths {
            let Some(kind) = SourceKind::of(Path::new(&path)) else {
                continue;
            };
            let full = root.join(&path);
            let Ok(bytes) = std::fs::read(&full) else {
                continue;
            };
            let name = asset_name(&path);
            let settings =
                read_meta(&full).unwrap_or_else(|| ImportSettings::new(derive_id(&name)));
            entries.insert(
                name.clone(),
                Entry {
                    path,
                    name,
                    kind,
                    settings,
                    hash: content_hash(&bytes),
                },
            );
        }
        Catalog {
            root: root.to_path_buf(),
            entries,
        }
    }

    /// The project this catalogue came from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every entry, in name order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// One entry by the name a scene refers to it by.
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// How many sources there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the project has no assets.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Names whose source bytes differ from this catalogue's, in name order.
    ///
    /// This is hot reload's question. It is deliberately about content rather
    /// than modification time: a build step that rewrites a file byte for byte
    /// should not trigger a reload, and a file restored from a backup should.
    pub fn changed_since(&self, previous: &Catalog) -> Vec<String> {
        let mut changed: Vec<String> = self
            .entries
            .iter()
            .filter(|(name, entry)| {
                previous.entries.get(*name).map(|p| &p.hash) != Some(&entry.hash)
            })
            .map(|(name, _)| name.clone())
            .collect();
        changed.extend(
            previous
                .entries
                .keys()
                .filter(|name| !self.entries.contains_key(*name))
                .cloned(),
        );
        changed.sort();
        changed.dedup();
        changed
    }
}

/// What one source imported to.
#[derive(Clone, PartialEq, Debug)]
pub enum Artifact {
    /// A still image.
    Image(Image),
    /// A sprite strip and the clips that index into it.
    Animation {
        /// Every frame, laid out in one horizontal strip.
        sheet: Image,
        /// Width of one frame, in pixels.
        frame_width: u32,
        /// Height of one frame, in pixels.
        frame_height: u32,
        /// How many frames the strip holds.
        frame_count: u32,
        /// Named clips from the document's tags.
        clips: Vec<Clip>,
    },
    /// An audio clip, left encoded.
    ///
    /// Decoding is the audio backend's job and happens when the sound is first
    /// played. Importing it here would mean holding every sound in the project
    /// as raw samples for no gain.
    Audio {
        /// Bytes as they are on disk.
        bytes: Vec<u8>,
    },
    /// An LDtk project, which is read when a level is baked rather than cached.
    Level,
}

/// The result of importing a project.
#[derive(Clone, PartialEq, Debug)]
pub struct Imported {
    /// Artifacts by asset name.
    pub artifacts: BTreeMap<String, Artifact>,
    /// Every atlas-eligible image, packed.
    pub sheet: Sheet,
    /// What went wrong, per asset.
    pub failures: Vec<(String, String)>,
}

impl Imported {
    /// The clips an animation imported to.
    pub fn clips(&self, name: &str) -> &[Clip] {
        match self.artifacts.get(name) {
            Some(Artifact::Animation { clips, .. }) => clips,
            _ => &[],
        }
    }
}

/// Import every source in a catalogue.
///
/// `tick_rate` is baked into animation clips here, at import, which is the
/// whole reason this function needs to know it.
pub fn import(catalog: &Catalog, tick_rate: u32) -> Imported {
    let mut artifacts = BTreeMap::new();
    let mut failures = Vec::new();
    let mut packable: Vec<Framed> = Vec::new();

    for entry in catalog.entries() {
        let full = catalog.root().join(&entry.path);
        match import_one(&full, entry, tick_rate) {
            Ok(artifact) => {
                if entry.settings.atlas {
                    match &artifact {
                        Artifact::Image(image) => {
                            let mut image = image.clone();
                            image.name = entry.name.clone();
                            packable.push(Framed {
                                image,
                                frames: entry.settings.frames.max(1),
                            });
                        }
                        Artifact::Animation {
                            sheet, frame_count, ..
                        } => {
                            let mut image = sheet.clone();
                            image.name = entry.name.clone();
                            packable.push(Framed {
                                image,
                                frames: *frame_count,
                            });
                        }
                        _ => {}
                    }
                }
                artifacts.insert(entry.name.clone(), artifact);
            }
            Err(e) => failures.push((entry.name.clone(), e.to_string())),
        }
    }

    Imported {
        artifacts,
        sheet: pack_framed(packable, MAX_SHEET_WIDTH),
        failures,
    }
}

fn import_one(full: &Path, entry: &Entry, tick_rate: u32) -> Result<Artifact, ImageError> {
    match entry.kind {
        SourceKind::Png => {
            let mut image = decode_png(full)?;
            image.name = entry.name.clone();
            let frames = entry.settings.frames.max(1);
            if frames == 1 {
                return Ok(Artifact::Image(image));
            }
            // A PNG that the `.meta` calls a strip animates like an Aseprite
            // document does, with one clip covering every frame. Nothing in a
            // PNG says it is a strip, so somebody has to.
            let width = image.width / frames;
            let height = image.height;
            let ticks = crate::clip::ms_to_ticks(entry.settings.frame_ms, tick_rate);
            Ok(Artifact::Animation {
                sheet: image,
                frame_width: width,
                frame_height: height,
                frame_count: frames,
                clips: vec![crate::clip::Clip {
                    name: "default".to_string(),
                    frames: (0..frames)
                        .map(|index| crate::clip::Frame {
                            index,
                            ticks,
                            event: None,
                        })
                        .collect(),
                    looping: true,
                }],
            })
        }
        SourceKind::Aseprite => {
            let ase = crate::aseprite::import(full, tick_rate)?;
            Ok(Artifact::Animation {
                sheet: ase.sheet,
                frame_width: ase.frame_width,
                frame_height: ase.frame_height,
                frame_count: ase.frame_count,
                clips: ase.clips,
            })
        }
        SourceKind::Ogg => Ok(Artifact::Audio {
            bytes: std::fs::read(full).map_err(|e| ImageError::io(full, e))?,
        }),
        SourceKind::Ldtk => Ok(Artifact::Level),
    }
}

/// Write the `.meta` files a scan invented, and record what was imported.
///
/// Called after a successful import so the sidecars carry the hash the cache
/// was built from. A source whose import failed keeps its old hash, so the next
/// run tries again rather than treating the failure as done.
pub fn write_metas(catalog: &Catalog, imported: &Imported) -> std::io::Result<usize> {
    let failed: std::collections::BTreeSet<&str> =
        imported.failures.iter().map(|(n, _)| n.as_str()).collect();
    let mut written = 0;
    for entry in catalog.entries() {
        if failed.contains(entry.name.as_str()) {
            continue;
        }
        let mut settings = entry.settings.clone();
        settings.source_hash = Some(entry.hash.clone());
        let path = meta_path(&catalog.root().join(&entry.path));
        let text = settings.to_text();
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text)?;
            written += 1;
        }
    }
    Ok(written)
}

/// Where the cached sheet is written.
pub fn sheet_path(root: &Path) -> PathBuf {
    root.join(IMPORT_DIR).join("atlas.png")
}

/// Write the packed sheet and its manifest into `.import/`.
pub fn write_cache(root: &Path, imported: &Imported) -> std::io::Result<()> {
    let dir = root.join(IMPORT_DIR);
    std::fs::create_dir_all(&dir)?;
    // A cache directory that is not ignored is a cache directory somebody
    // commits, so it ignores itself rather than relying on the project's
    // `.gitignore` being right.
    std::fs::write(dir.join(".gitignore"), "*\n")?;
    encode_png(
        &sheet_path(root),
        &imported.sheet.pixels,
        imported.sheet.width,
        imported.sheet.height,
    )?;
    std::fs::write(dir.join("atlas.json"), manifest(imported))?;
    Ok(())
}

/// The manifest describing what is in the cached sheet.
fn manifest(imported: &Imported) -> String {
    let clips: BTreeMap<&String, &Vec<Clip>> = imported
        .artifacts
        .iter()
        .filter_map(|(name, a)| match a {
            Artifact::Animation { clips, .. } => Some((name, clips)),
            _ => None,
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "width": imported.sheet.width,
        "height": imported.sheet.height,
        "placements": imported.sheet.placements,
        "clips": clips,
    }))
    .unwrap_or_default()
        + "\n"
}

/// Where a source file's sidecar lives.
pub fn meta_path(source: &Path) -> PathBuf {
    let mut name = source.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(META_EXTENSION);
    source.with_file_name(name)
}

fn read_meta(source: &Path) -> Option<ImportSettings> {
    let text = std::fs::read_to_string(meta_path(source)).ok()?;
    ImportSettings::parse(&text).ok()
}

/// The name a scene refers to `assets/sprites/hero.png` by: `sprites/hero`.
pub fn asset_name(path: &str) -> String {
    let trimmed = path
        .strip_prefix(ASSETS_DIR)
        .and_then(|p| p.strip_prefix('/'))
        .unwrap_or(path);
    match trimmed.rfind('.') {
        Some(dot) => trimmed[..dot].to_string(),
        None => trimmed.to_string(),
    }
}

/// The id a never-before-imported asset starts with.
///
/// Derived from the name so a project whose `.meta` files were lost comes back
/// with the ids its scenes already reference. Once the sidecar exists, the
/// stored id wins and the derivation stops mattering — which is what makes a
/// rename free.
pub fn derive_id(name: &str) -> AssetId {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    let hash = blake3::hash(name.as_bytes());
    let bits = u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("blake3 is 32 bytes"));
    let body: String = (0..8)
        .map(|i| ALPHABET[((bits >> (i * 5)) & 31) as usize] as char)
        .collect();
    AssetId::parse(&format!("{}{body}", AssetId::PREFIX)).expect("the alphabet is valid")
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
