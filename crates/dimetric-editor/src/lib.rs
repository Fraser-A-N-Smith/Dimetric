//! Editor view state.
//!
//! # What is here and what is not
//!
//! The `.dim.editor` sidecar — the view state an editor holds — is implemented.
//! The `egui` client is **not in this build**.
//!
//! # The contract the client has to keep
//!
//! The editor holds no authoritative state. Every widget reads engine state and
//! emits a [`Command`](dimetric_host::Command); none of them touches the scene
//! directly (invariant I1). That is checkable rather than aspirational: a test
//! records the commands a scripted editing session produces, and a session that
//! changes the scene without producing commands has gone around the bus.
//!
//! The other rule is that opening and closing a scene must produce **zero**
//! diff in the `.dim`. Everything that changes when you merely look at a
//! scene — where the camera is, what is selected, which branches are folded —
//! belongs in the sidecar, which is committed so a team shares it.

#![warn(missing_docs)]

use std::collections::BTreeSet;

use dimetric_core::NodeUid;
use serde::{Deserialize, Serialize};

/// View state for one scene, stored beside it as `<scene>.dim.editor`.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Sidecar {
    /// Camera centre, as exact decimals.
    #[serde(default)]
    pub camera: [String; 2],
    /// Zoom level, as an exact decimal.
    #[serde(default)]
    pub zoom: String,
    /// Selected nodes.
    #[serde(default)]
    pub selection: BTreeSet<NodeUid>,
    /// Nodes whose children are folded away in the tree view.
    #[serde(default)]
    pub folded: BTreeSet<NodeUid>,
}

impl Sidecar {
    /// Render as TOML.
    ///
    /// Sets are written in sorted order, so two people with the same selection
    /// produce the same file and the sidecar does not churn in every diff.
    pub fn to_text(&self) -> String {
        let mut out = String::from("# Editor view state. Committed on purpose: camera position,\n\
                                    # selection and fold state are worth sharing across a team.\n\n");
        out.push_str(&format!(
            "camera = [{}, {}]\n",
            quote(&self.camera[0]),
            quote(&self.camera[1])
        ));
        out.push_str(&format!("zoom = {}\n", quote(&self.zoom)));
        out.push_str(&format!("selection = [{}]\n", join(&self.selection)));
        out.push_str(&format!("folded = [{}]\n", join(&self.folded)));
        out
    }
}

fn quote(s: &str) -> String {
    let value = if s.is_empty() { "0.0" } else { s };
    format!("\"{value}\"")
}

fn join(ids: &BTreeSet<NodeUid>) -> String {
    ids.iter()
        .map(|u| format!("\"{}\"", u.to_text()))
        .collect::<Vec<_>>()
        .join(", ")
}
