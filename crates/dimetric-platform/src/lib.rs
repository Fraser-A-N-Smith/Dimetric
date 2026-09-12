//! Input sources and project paths.
//!
//! Thin on purpose: this crate isolates every operating-system concern so that
//! nothing above it touches a platform API.
//!
//! # What is here and what is not
//!
//! The input-source abstraction and path resolution are implemented. The
//! `winit` window, the event loop and filesystem watching are **not in this
//! build**.
//!
//! The abstraction is the part that matters for determinism. A device, a log
//! file and — later — a socket are indistinguishable from inside a tick, which
//! is invariant I8's practical form and most of what rollback netcode needs.

#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use dimetric_sim::{InputFrame, InputLog};

/// Anything that can supply a tick's input.
pub trait InputSource {
    /// Input for `tick`.
    fn frame(&mut self, tick: u64) -> InputFrame;

    /// How many ticks are available, when that is known.
    ///
    /// A device never knows; a log always does. Replay uses it to decide when
    /// the recording has run out.
    fn len(&self) -> Option<u64> {
        None
    }

    /// True when the source is known to be exhausted.
    fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
}

/// Input read from a recorded log.
pub struct LogSource {
    log: InputLog,
}

impl LogSource {
    /// Wrap a log.
    pub fn new(log: InputLog) -> LogSource {
        LogSource { log }
    }

    /// The log being replayed.
    pub fn log(&self) -> &InputLog {
        &self.log
    }
}

impl InputSource for LogSource {
    fn frame(&mut self, tick: u64) -> InputFrame {
        self.log.frame(tick)
    }

    fn len(&self) -> Option<u64> {
        Some(self.log.frames.len() as u64)
    }
}

/// Input that records what it supplies, so a session can be replayed later.
pub struct RecordingSource<S: InputSource> {
    inner: S,
    /// The log being built.
    pub log: InputLog,
}

impl<S: InputSource> RecordingSource<S> {
    /// Wrap a source and record from it.
    pub fn new(inner: S, log: InputLog) -> RecordingSource<S> {
        RecordingSource { inner, log }
    }
}

impl<S: InputSource> InputSource for RecordingSource<S> {
    fn frame(&mut self, tick: u64) -> InputFrame {
        let frame = self.inner.frame(tick);
        self.log.push(frame.clone());
        frame
    }

    fn len(&self) -> Option<u64> {
        self.inner.len()
    }
}

/// Input that is always idle. Headless runs with no log use this.
#[derive(Clone, Copy, Debug)]
pub struct IdleSource {
    /// Players to report.
    pub players: usize,
}

impl InputSource for IdleSource {
    fn frame(&mut self, _tick: u64) -> InputFrame {
        InputFrame::idle(self.players)
    }
}

/// Where a project's files live.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Project root.
    pub root: PathBuf,
}

impl Paths {
    /// Resolve paths against a root.
    pub fn new(root: impl Into<PathBuf>) -> Paths {
        Paths { root: root.into() }
    }

    /// Find the project root by walking up from `start` looking for a scene.
    pub fn discover(start: &Path) -> Option<Paths> {
        let mut cursor = Some(start);
        while let Some(dir) = cursor {
            let has_scene = std::fs::read_dir(dir).ok().is_some_and(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("dim"))
            });
            if has_scene {
                return Some(Paths::new(dir));
            }
            cursor = dir.parent();
        }
        None
    }

    /// Where source assets live.
    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    /// Where scripts live.
    pub fn scripts(&self) -> PathBuf {
        self.root.join("scripts")
    }

    /// Where imported artifacts are cached.
    pub fn import_cache(&self) -> PathBuf {
        self.root.join(".import")
    }

    /// The editor's sidecar for a scene.
    ///
    /// Committed to version control, so camera position, selection and fold
    /// state are shared across a team rather than being one more thing each
    /// person sets up by hand.
    pub fn sidecar(&self, scene: &Path) -> PathBuf {
        let mut name = scene.file_name().unwrap_or_default().to_os_string();
        name.push(".editor");
        scene.with_file_name(name)
    }
}
