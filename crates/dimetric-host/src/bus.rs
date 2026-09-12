//! Undo and redo.
//!
//! A stack of applied commands and the inverses they produced. Because the bus
//! is the only path that mutates a scene, the stack cannot fall out of step
//! with the thing it describes — there is no second way in for a change to
//! arrive by.

use dimetric_core::{Code, Diagnostic};
use dimetric_scene::{KindRegistry, SceneDoc};

use crate::command::{apply, Applied, Command};

/// How many edits the stack remembers.
pub const DEFAULT_LIMIT: usize = 512;

/// The undo and redo stacks.
#[derive(Debug, Default)]
pub struct CommandBus {
    undo: Vec<Applied>,
    redo: Vec<Applied>,
    limit: usize,
    history: Vec<Command>,
}

impl CommandBus {
    /// A bus with the default history limit.
    pub fn new() -> CommandBus {
        CommandBus {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: DEFAULT_LIMIT,
            history: Vec::new(),
        }
    }

    /// A bus remembering at most `limit` edits.
    pub fn with_limit(limit: usize) -> CommandBus {
        CommandBus {
            limit,
            ..CommandBus::new()
        }
    }

    /// Apply a command and record how to undo it.
    pub fn apply(
        &mut self,
        doc: &mut SceneDoc,
        registry: &KindRegistry,
        command: Command,
    ) -> Result<(), Diagnostic> {
        let inverse = apply(doc, registry, &command)?;
        self.history.push(command.clone());
        // A fresh edit invalidates the redo branch, the way every editor does
        // it: you cannot redo forward into a future you have diverged from.
        self.redo.clear();
        self.undo.push(Applied { command, inverse });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        Ok(())
    }

    /// Undo the last command, returning what was undone.
    pub fn undo(
        &mut self,
        doc: &mut SceneDoc,
        registry: &KindRegistry,
    ) -> Result<Command, Diagnostic> {
        let entry = self
            .undo
            .pop()
            .ok_or_else(|| Diagnostic::new(Code::NOTHING_TO_UNDO, "there is nothing to undo"))?;
        for step in &entry.inverse {
            apply(doc, registry, step)?;
        }
        let command = entry.command.clone();
        self.redo.push(entry);
        Ok(command)
    }

    /// Redo the last undone command.
    pub fn redo(
        &mut self,
        doc: &mut SceneDoc,
        registry: &KindRegistry,
    ) -> Result<Command, Diagnostic> {
        let entry = self
            .redo
            .pop()
            .ok_or_else(|| Diagnostic::new(Code::NOTHING_TO_UNDO, "there is nothing to redo"))?;
        // Re-apply the original and recompute its inverse rather than trusting
        // the stored one: the scene may have been rebuilt since, and a stale
        // inverse is worse than no inverse.
        let inverse = apply(doc, registry, &entry.command)?;
        let command = entry.command.clone();
        self.undo.push(Applied {
            command: entry.command,
            inverse,
        });
        Ok(command)
    }

    /// How many edits can be undone.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// How many edits can be redone.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Every command applied through this bus, in order.
    ///
    /// This is what a test recording an editing session asserts against, and
    /// what proves a GUI action went through the bus rather than around it
    /// (I1).
    pub fn history(&self) -> &[Command] {
        &self.history
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.history.clear();
    }
}
