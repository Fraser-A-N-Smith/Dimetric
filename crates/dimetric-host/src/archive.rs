//! A read-only archive appended to a runtime, so a game can be one file.
//!
//! `package.rs` argues for staging files beside the runtime rather than bundling
//! them, and the argument is right: the files a game ships are the files it was
//! developed against, byte for byte, which is what makes "it worked on my
//! machine" checkable. But "send me the game" means a folder of several hundred
//! files that breaks the moment somebody drags the executable out of it, and
//! every fix on the game's side — a self-extractor, an unpacker writing to a
//! temporary directory — breaks exactly the property the module protects.
//!
//! So this is a **read path**, not a bundler. The staged files are appended to a
//! copy of the runtime, unmodified and uncompressed, and the runtime reads them
//! where they lie. Nothing is unpacked, nothing is written at startup, and
//! because the bytes are the staged bytes a build stays inspectable: `dim
//! inspect` lists what is in one and checks its hash.
//!
//! # The layout
//!
//! ```text
//! [ the runtime executable, untouched ]
//! [ file 0 ][ file 1 ][ ... ]            payload, in index order
//! [ index ]                              one line per file, tab separated
//! [ footer ]                             64 bytes, fixed
//! ```
//!
//! The footer is last so it can be found by seeking to `len - 64` without
//! knowing anything else, and it carries the magic, where the index is, how many
//! files there are, and a BLAKE3 of the payload and index together. An ordinary
//! runtime does not end in the magic, so "is there a game in here" is one read.
//!
//! The index is text, for the same reason a scene and a save are text: a build
//! somebody cannot read is a build somebody cannot debug.

use std::io;
use std::path::{Path, PathBuf};

use dimetric_core::Source;

/// What the footer begins with. Bumped if the layout changes.
pub const MAGIC: &[u8; 8] = b"DIMPACK1";

/// The footer's size: magic, index offset, index length, count, hash.
pub const FOOTER_LEN: usize = 8 + 8 + 8 + 8 + 32;

/// One file in an archive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// Project-relative path, `/` separated.
    pub path: String,
    /// Where the bytes start, from the beginning of the file.
    pub offset: u64,
    /// How many bytes.
    pub len: u64,
}

/// What an archive says about itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// The files, in the order they were written.
    pub entries: Vec<Entry>,
    /// BLAKE3 of the payload and the index.
    pub hash: String,
    /// Where the runtime ends and the payload begins.
    pub payload_at: u64,
}

/// Append `files` to a copy of `runtime`, writing the result to `out`.
///
/// `files` is `(project-relative path, bytes)`. Order is the caller's and is
/// preserved, so a build is reproducible from the same staging.
pub fn write(runtime: &Path, out: &Path, files: &[(String, Vec<u8>)]) -> io::Result<Manifest> {
    for (path, _) in files {
        if path.contains('\t') || path.contains('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path:?} has a tab or a newline in it, and the index is text"),
            ));
        }
    }

    // A copy rather than a rewrite, so the runtime's own bytes — and on Unix its
    // executable bit — arrive untouched.
    if out.exists() {
        std::fs::remove_file(out)?;
    }
    std::fs::copy(runtime, out)?;
    let payload_at = std::fs::metadata(out)?.len();

    let mut payload = Vec::new();
    let mut entries = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        entries.push(Entry {
            path: path.clone(),
            offset: payload_at + payload.len() as u64,
            len: bytes.len() as u64,
        });
        payload.extend_from_slice(bytes);
    }

    let index = index_text(&entries);
    let mut tail = payload;
    let index_offset = payload_at + tail.len() as u64;
    tail.extend_from_slice(index.as_bytes());

    let hash = blake3::hash(&tail).to_hex().to_string();
    let mut footer = Vec::with_capacity(FOOTER_LEN);
    footer.extend_from_slice(MAGIC);
    footer.extend_from_slice(&index_offset.to_le_bytes());
    footer.extend_from_slice(&(index.len() as u64).to_le_bytes());
    footer.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    footer.extend_from_slice(blake3::hash(&tail).as_bytes());
    debug_assert_eq!(footer.len(), FOOTER_LEN);

    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new().append(true).open(out)?;
    file.write_all(&tail)?;
    file.write_all(&footer)?;
    file.flush()?;

    Ok(Manifest {
        entries,
        hash,
        payload_at,
    })
}

/// One line per file: path, offset, length.
fn index_text(entries: &[Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            entry.path, entry.offset, entry.len
        ));
    }
    out
}

/// Read an archive's manifest, or `None` when the file carries none.
///
/// Not an error: every ordinary executable is a file with no archive in it, and
/// "is there a game appended to me" is a question the runtime asks about itself
/// on every start.
pub fn read_manifest(path: &Path) -> io::Result<Option<Manifest>> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    if total < FOOTER_LEN as u64 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(total - FOOTER_LEN as u64))?;
    let mut footer = [0u8; FOOTER_LEN];
    file.read_exact(&mut footer)?;
    if &footer[..8] != MAGIC {
        return Ok(None);
    }
    let number = |at: usize| u64::from_le_bytes(footer[at..at + 8].try_into().expect("8 bytes"));
    let index_offset = number(8);
    let index_len = number(16);
    let count = number(24);
    let hash = blake3::Hash::from_bytes(footer[32..64].try_into().expect("32 bytes"))
        .to_hex()
        .to_string();

    let tail_at = total - FOOTER_LEN as u64 - index_len;
    if index_offset > total || index_offset + index_len > total || index_offset != tail_at {
        return Err(malformed(
            "the footer does not agree with the file's length",
        ));
    }
    file.seek(SeekFrom::Start(index_offset))?;
    let mut index = vec![0u8; index_len as usize];
    file.read_exact(&mut index)?;
    let index = String::from_utf8(index).map_err(|_| malformed("the index is not text"))?;

    let mut entries = Vec::new();
    for line in index.lines() {
        let mut fields = line.split('\t');
        let (Some(path), Some(offset), Some(len), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(malformed(&format!(
                "index line {line:?} is not three fields"
            )));
        };
        let parse = |text: &str| {
            text.parse::<u64>()
                .map_err(|_| malformed(&format!("{text:?} is not a number")))
        };
        entries.push(Entry {
            path: path.to_string(),
            offset: parse(offset)?,
            len: parse(len)?,
        });
    }
    if entries.len() as u64 != count {
        return Err(malformed(&format!(
            "the footer says {count} files and the index lists {}",
            entries.len()
        )));
    }
    let payload_at = entries
        .iter()
        .map(|e| e.offset)
        .min()
        .unwrap_or(index_offset);
    for entry in &entries {
        if entry.offset < payload_at || entry.offset + entry.len > index_offset {
            return Err(malformed(&format!(
                "{} runs outside the archive's payload",
                entry.path
            )));
        }
    }

    Ok(Some(Manifest {
        entries,
        hash,
        payload_at,
    }))
}

/// Read the payload and index and hash them, for `dim inspect`.
///
/// Separate from [`read_manifest`] because starting a game does not need it: a
/// runtime that hashed its whole payload on every start would pay for a check
/// nobody asked for. Verifying is a thing you *do*, deliberately.
pub fn verify(path: &Path, manifest: &Manifest) -> io::Result<bool> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    let tail_len = total - FOOTER_LEN as u64 - manifest.payload_at;
    file.seek(SeekFrom::Start(manifest.payload_at))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    Ok(blake3::hash(&tail).to_hex().to_string() == manifest.hash)
}

fn malformed(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("this file has a Dimetric footer but {what}"),
    )
}

/// A project read out of an archive appended to a file.
///
/// Holds the file's path and its index, and reads a range on demand. The whole
/// payload is not loaded: a game's assets are the large part of it and the
/// importer asks for them one at a time anyway.
#[derive(Debug)]
pub struct Archive {
    path: PathBuf,
    entries: std::collections::BTreeMap<String, Entry>,
}

impl Archive {
    /// Open the archive appended to `path`, or `None` if there is none.
    pub fn open(path: &Path) -> io::Result<Option<Archive>> {
        let Some(manifest) = read_manifest(path)? else {
            return Ok(None);
        };
        Ok(Some(Archive {
            path: path.to_path_buf(),
            entries: manifest
                .entries
                .into_iter()
                .map(|e| (e.path.clone(), e))
                .collect(),
        }))
    }

    /// How many files it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Source for Archive {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let Some(entry) = self.entries.get(path) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{path} is not in this game"),
            ));
        };
        let mut file = std::fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0u8; entry.len as usize];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn list(&self, dir: &str) -> Vec<String> {
        // A `BTreeMap`, so this is already sorted and the same everywhere.
        let prefix = match dir.is_empty() || dir == "." {
            true => String::new(),
            false => format!("{}/", dir.trim_end_matches('/')),
        };
        self.entries
            .keys()
            .filter(|path| path.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn exists(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    fn root(&self) -> Option<&Path> {
        // Deliberately none: there is nowhere to write, and a caller that gets
        // `None` is being told a shipped game does not author.
        None
    }
}
