//! Editor view state, stored beside a scene as `<scene>.dim.editor`.
//!
//! Everything that changes when you merely *look* at a scene lives here:
//! where the camera is, what is selected, which branches are folded. None of it
//! belongs in the `.dim`, because opening and closing a scene must produce zero
//! diff in the scene itself.
//!
//! The sidecar is committed on purpose. Camera position and fold state are
//! worth sharing across a team, and a file that is regenerated per machine is a
//! file that conflicts on every pull.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dimetric_core::NodeUid;
use serde::{Deserialize, Serialize};

/// Extension appended to a scene path.
pub const SIDECAR_EXTENSION: &str = "dim.editor";

/// View state for one scene.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
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

impl Default for Sidecar {
    fn default() -> Sidecar {
        Sidecar {
            camera: ["0.0".to_string(), "0.0".to_string()],
            zoom: "1.0".to_string(),
            selection: BTreeSet::new(),
            folded: BTreeSet::new(),
        }
    }
}

impl Sidecar {
    /// Render as TOML.
    ///
    /// Sets are written in sorted order, so two people with the same selection
    /// produce the same file and the sidecar does not churn in every diff.
    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# Editor view state. Committed on purpose: camera position,\n\
             # selection and fold state are worth sharing across a team.\n\n",
        );
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

    /// Parse from TOML.
    ///
    /// Anything unreadable falls back to the default rather than failing: a
    /// corrupt sidecar should cost you your camera position, not your scene.
    pub fn parse(text: &str) -> Sidecar {
        let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
            return Sidecar::default();
        };
        let string = |key: &str| {
            doc.get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let ids = |key: &str| -> BTreeSet<NodeUid> {
            doc.get(key)
                .and_then(|i| i.as_value())
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| NodeUid::parse(s).ok())
                        .collect()
                })
                .unwrap_or_default()
        };
        let camera = doc
            .get("camera")
            .and_then(|i| i.as_value())
            .and_then(|v| v.as_array())
            .map(|a| {
                let mut it = a.iter().filter_map(|v| v.as_str());
                [
                    it.next().unwrap_or("0.0").to_string(),
                    it.next().unwrap_or("0.0").to_string(),
                ]
            })
            .unwrap_or_else(|| ["0.0".to_string(), "0.0".to_string()]);

        Sidecar {
            camera,
            zoom: string("zoom").unwrap_or_else(|| "1.0".to_string()),
            selection: ids("selection"),
            folded: ids("folded"),
        }
    }

    /// Where the sidecar for a scene lives.
    pub fn path_for(scene: &Path) -> PathBuf {
        let stem = scene
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        scene.with_file_name(format!("{stem}.{SIDECAR_EXTENSION}"))
    }

    /// Read the sidecar beside a scene, or the default when there is none.
    pub fn load(scene: &Path) -> Sidecar {
        std::fs::read_to_string(Sidecar::path_for(scene))
            .map(|text| Sidecar::parse(&text))
            .unwrap_or_default()
    }

    /// Write the sidecar beside a scene, if it differs from what is there.
    ///
    /// Skipping an identical write is what keeps a session that changed nothing
    /// from touching the file's timestamp and showing up in a watcher.
    pub fn save(&self, scene: &Path) -> std::io::Result<bool> {
        let path = Sidecar::path_for(scene);
        let text = self.to_text();
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(false);
        }
        std::fs::write(path, text)?;
        Ok(true)
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
