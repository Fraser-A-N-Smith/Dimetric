//! What a user did, as data.
//!
//! Every interaction a widget can have becomes one of these, and
//! [`crate::Editor::dispatch`] turns it into commands. Nothing in the editor
//! writes to a scene directly.
//!
//! The reason for the indirection is not tidiness. A widget that edits the
//! scene itself is a second mutation path, and a second mutation path is how
//! undo starts disagreeing with the file. Making the interaction a value means
//! the rule is checkable: a scripted session can be run with no GUI at all, and
//! the commands it produced compared against the diff it made.

use dimetric_core::{NodeUid, Vec2Fx};

/// One thing a user did.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    // -- view state: changes the sidecar, never the scene ----------------
    /// Replace the selection.
    Select(Vec<NodeUid>),
    /// Add to or remove from the selection.
    ToggleSelect(NodeUid),
    /// Clear the selection.
    SelectNone,
    /// Fold or unfold a node's children in the tree.
    ToggleFold(NodeUid),
    /// Move the viewport camera to a point.
    LookAt(Vec2Fx),
    /// Set the viewport zoom.
    Zoom(String),

    // -- scene edits: every one of these produces commands ---------------
    /// Add a node under a parent.
    Create {
        /// Registered node kind.
        kind: String,
        /// Name, unique among siblings.
        name: String,
        /// Parent, or none for the scene root.
        parent: Option<NodeUid>,
    },
    /// Remove a node and its subtree.
    Delete(NodeUid),
    /// Rename a node.
    Rename {
        /// Node to rename.
        node: NodeUid,
        /// New name.
        name: String,
    },
    /// Set or clear one property, from the literal text an inspector field
    /// holds.
    ///
    /// Text rather than a parsed value because that is what a text field has,
    /// and because parsing it here is what lets a bad value be a diagnostic in
    /// the console instead of a panic in a widget.
    SetProperty {
        /// Node to change.
        node: NodeUid,
        /// Property name.
        key: String,
        /// Literal text, or none to clear the property.
        literal: Option<String>,
    },
    /// Move a node in the tree.
    Reparent {
        /// Node to move.
        node: NodeUid,
        /// New parent.
        parent: NodeUid,
    },
    /// Drag a node's gizmo to a world position.
    Move {
        /// Node being dragged.
        node: NodeUid,
        /// Where it was dropped, in world space.
        to: Vec2Fx,
    },
    /// Instance a prefab under a parent.
    Instance {
        /// Scene reference, such as `prefabs/skeleton`.
        source: String,
        /// Name for the instance.
        name: String,
        /// Parent to hang it off.
        parent: NodeUid,
    },

    // -- history and files ----------------------------------------------
    /// Undo the last scene edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Write the scene and the sidecar.
    Save,

    // -- playback --------------------------------------------------------
    /// Start playing from the current state.
    Play,
    /// Stop and return to the edited scene.
    Stop,
    /// Advance one tick while paused.
    StepTick,
    /// Pause without leaving play mode.
    Pause,
    /// Move the replay scrubber to a tick.
    ScrubTo(u64),
}

impl Action {
    /// Whether this action can change the scene.
    ///
    /// The editor uses it to decide whether a save is needed; the acceptance
    /// test uses it to assert the converse — that an action which is *not*
    /// an edit leaves the `.dim` byte-identical.
    pub fn edits_scene(&self) -> bool {
        matches!(
            self,
            Action::Create { .. }
                | Action::Delete(_)
                | Action::Rename { .. }
                | Action::SetProperty { .. }
                | Action::Reparent { .. }
                | Action::Move { .. }
                | Action::Instance { .. }
                | Action::Undo
                | Action::Redo
        )
    }
}
