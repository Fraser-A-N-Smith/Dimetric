//! Panel models: engine state, shaped for a client to draw.
//!
//! Each of these turns the project into rows. None of them draws anything, and
//! none of them can change anything — a panel that could write to the scene
//! would be a second mutation path, and the whole point of the split is that
//! there is exactly one.
//!
//! Keeping them here rather than in a client also means they are testable. "The
//! tree hides a folded node's children" and "the inspector shows a property's
//! default in grey" are assertions about data, and they should not need a
//! window to make.

pub mod assets;
pub mod inspector;
pub mod tree;
pub mod viewport;

pub use assets::{asset_rows, AssetRow};
pub use inspector::{inspector_rows, InspectorRow};
pub use tree::{tree_rows, TreeRow};
pub use viewport::{gizmos, Gizmo, Viewport};
