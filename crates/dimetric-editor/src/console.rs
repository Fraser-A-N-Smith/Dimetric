//! The console: structured diagnostics, kept in order.
//!
//! The engine's errors already carry a code, a span and named fields, so the
//! console shows those rather than a line of prose. A designer who can read
//! `DIM0301` and the property it names can find the typo without asking
//! anybody; a console that flattens it to "error: invalid scene" cannot.

use dimetric_core::{Diagnostic, Diagnostics, Severity};

/// How many entries the console keeps.
///
/// A bounded log, because an editor left open for a day with a script erroring
/// every tick should not eventually be a memory leak with a scrollbar.
pub const CAPACITY: usize = 512;

/// A diagnostic log.
#[derive(Clone, Debug, Default)]
pub struct Console {
    entries: Vec<Diagnostic>,
    dropped: usize,
}

impl Console {
    /// An empty console.
    pub fn new() -> Console {
        Console::default()
    }

    /// Add one diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.entries.push(diagnostic);
        if self.entries.len() > CAPACITY {
            let excess = self.entries.len() - CAPACITY;
            self.entries.drain(..excess);
            self.dropped += excess;
        }
    }

    /// Add several.
    pub fn extend(&mut self, diagnostics: Diagnostics) {
        for diagnostic in diagnostics.0 {
            self.push(diagnostic);
        }
    }

    /// Everything kept, oldest first.
    pub fn entries(&self) -> &[Diagnostic] {
        &self.entries
    }

    /// Entries at or above a severity.
    pub fn filtered(&self, at_least: Severity) -> impl Iterator<Item = &Diagnostic> {
        self.entries
            .iter()
            .filter(move |d| rank(d.severity) >= rank(at_least))
    }

    /// How many entries were dropped to stay within capacity.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// How many errors are in the log.
    pub fn errors(&self) -> usize {
        self.entries
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// Empty it.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }

    /// True when nothing has been logged.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Severity order, worst highest.
fn rank(severity: Severity) -> u8 {
    match severity {
        Severity::Note => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}
