//! The scene tree.

use dimetric_core::NodeUid;

use crate::editor::Editor;

/// One line in the tree view.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TreeRow {
    /// The node.
    pub uid: NodeUid,
    /// Its name.
    pub name: String,
    /// Its kind.
    pub kind: String,
    /// How deep it sits, with the root at zero.
    pub depth: usize,
    /// Whether it has children at all, so a client knows to draw a twisty.
    pub has_children: bool,
    /// Whether its children are hidden.
    pub folded: bool,
    /// Whether it is selected.
    pub selected: bool,
    /// Whether it is an instanced prefab, whose contents are not editable here.
    pub instance: bool,
}

/// The rows a tree view shows, in depth-first order.
///
/// A folded node's descendants are left out entirely rather than marked hidden,
/// so a client draws the list it is given and does not reimplement folding.
pub fn tree_rows(editor: &Editor) -> Vec<TreeRow> {
    let Some(doc) = editor.project.open.as_ref() else {
        return Vec::new();
    };
    let scene = &doc.scene;

    let mut rows = Vec::new();
    let mut hidden_under: Option<usize> = None;
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        let depth = scene
            .path_of(id)
            .map(|p| p.matches('/').count().saturating_sub(1))
            .unwrap_or(0);

        // Everything deeper than a folded node is skipped until the walk comes
        // back up to its level.
        if let Some(folded_depth) = hidden_under {
            if depth > folded_depth {
                continue;
            }
            hidden_under = None;
        }

        let folded = editor.sidecar.folded.contains(&node.uid);
        let has_children = scene.children(id).next().is_some();
        if folded && has_children {
            hidden_under = Some(depth);
        }

        rows.push(TreeRow {
            uid: node.uid,
            name: node.name.clone(),
            kind: node.kind.clone(),
            depth,
            has_children,
            folded,
            selected: editor.sidecar.selection.contains(&node.uid),
            instance: node.scene.is_some(),
        });
    }
    rows
}
