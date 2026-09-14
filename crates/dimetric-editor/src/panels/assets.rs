//! The asset browser.
//!
//! A list over the project's catalogue rather than the filesystem, so what it
//! shows is what the engine will actually load — including whether the cache is
//! behind the source, which is the question an artist wanting to know why their
//! change has not appeared is really asking.

use crate::editor::Editor;

/// One row in the asset browser.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AssetRow {
    /// Name a scene refers to it by, such as `sprites/hero`.
    pub name: String,
    /// Project-relative source path.
    pub path: String,
    /// What kind of file it is, lowercase.
    pub kind: String,
    /// Permanent id.
    pub id: String,
    /// Whether the cache is behind the source.
    pub stale: bool,
    /// Animation clips it imported to, by name.
    pub clips: Vec<String>,
}

/// Every asset in the project, in name order.
///
/// Reads the catalogue as it stands; a client calls
/// [`dimetric_host::Project::scan_assets`] when it wants it refreshed, which is
/// a decision about when to touch the disk and so belongs to the client.
pub fn asset_rows(editor: &Editor) -> Vec<AssetRow> {
    let imported = editor.project.imported();
    editor
        .project
        .catalog()
        .entries()
        .map(|entry| AssetRow {
            name: entry.name.clone(),
            path: entry.path.clone(),
            kind: format!("{:?}", entry.kind).to_lowercase(),
            id: entry.settings.id.to_string(),
            stale: entry.is_stale(),
            clips: imported
                .map(|i| {
                    i.clips(&entry.name)
                        .iter()
                        .map(|c| c.name.clone())
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}
