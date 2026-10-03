//! Where a project's files are read from.
//!
//! A project on disk is a directory, and that is what everything here was
//! written against. A *shipped* game is the same files, which is the property
//! `package.rs` protects — "the files a game ships are the files it was
//! developed against, byte for byte" — but "send me the game" means a folder of
//! several hundred files that breaks the moment somebody drags the executable
//! out of it.
//!
//! So reading is a trait with two implementations rather than a bundler. A
//! directory is one. A read-only archive appended to the runtime is the other,
//! and because it is a *read path* the shipped bytes are still the developed
//! bytes: nothing is rewritten, nothing is unpacked, and a build can be
//! inspected and hashed.
//!
//! Three methods, because three is what a game needs to start: read a file, list
//! a directory, ask whether something is there. Writing is deliberately absent —
//! a shipped game does not author, and a `Source` that could write would be a
//! `Source` somebody writes through.

use std::io;
use std::path::{Path, PathBuf};

/// Somewhere a project's files can be read from.
///
/// Paths are project-relative, use `/` as a separator, and never begin with one.
/// A directory implementation turns them into real paths; an archive looks them
/// up in its index.
pub trait Source: std::fmt::Debug {
    /// Read a file whole.
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;

    /// Every file under `dir`, recursively, as project-relative paths.
    ///
    /// Sorted, because a caller that walked them in whatever order the
    /// filesystem offered would behave differently on two machines — which for
    /// a script load order or an atlas pack is a divergence (I4).
    fn list(&self, dir: &str) -> Vec<String>;

    /// Whether a file is there.
    fn exists(&self, path: &str) -> bool;

    /// The directory this source reads from, when it is a directory.
    ///
    /// `None` for an archive. It is what a caller needs to *write* — a save, a
    /// re-imported cache, an edited scene — and a caller that gets `None` is
    /// being told it cannot, which is the honest answer for a shipped game.
    fn root(&self) -> Option<&Path>;
}

/// Read a file as text, decoding as UTF-8.
pub fn read_to_string(source: &dyn Source, path: &str) -> io::Result<String> {
    let bytes = source.read(path)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// A project directory.
#[derive(Clone, Debug)]
pub struct Directory {
    root: PathBuf,
}

impl Directory {
    /// Read from `root`.
    pub fn new(root: impl Into<PathBuf>) -> Directory {
        Directory { root: root.into() }
    }
}

impl Source for Directory {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.root.join(path))
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let mut out = Vec::new();
        collect(&self.root.join(dir), &self.root, &mut out);
        out.sort();
        out
    }

    fn exists(&self, path: &str) -> bool {
        self.root.join(path).is_file()
    }

    fn root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

/// Walk a directory, appending project-relative paths.
fn collect(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(&path, root, out);
            continue;
        }
        if let Ok(relative) = path.strip_prefix(root) {
            // `/` on every platform, so a path means the same thing in an
            // archive built on Windows and read on Linux.
            out.push(
                relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}
