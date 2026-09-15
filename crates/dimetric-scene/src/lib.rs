//! The node tree, the `.dim` scene format, and prefab instancing.
//!
//! # The two representations
//!
//! A [`SceneDoc`] holds both the parsed [`Scene`] and the `toml_edit` document
//! it came from. Ordinary edits patch the document, so comments, key order and
//! whitespace survive and load-then-save is byte-identical (invariant I2).
//! [`write::format_in_place`] canonicalises that document — key order, block
//! order, defaults — without rewriting the parts of it nobody asked about, so
//! comments survive formatting too. That is what `dim scene fmt` does: a
//! deliberate, reviewable step, not something firing on every save.
//! [`write::to_canonical_text`] renders a scene that has no document behind it.
//!
//! # The format in one paragraph
//!
//! Nodes are a flat list of `[[node]]` tables, each naming its parent by id.
//! The tree is rebuilt on load. Reparenting a subtree is a one-line change
//! rather than a fifty-line re-indentation, two branches adding nodes in
//! different places touch different regions of the file, and a patch tool
//! rewrites exactly one block. Canonical form orders nodes depth-first, so the
//! file still reads as a tree without being nested as one.

#![warn(missing_docs)]

pub mod chunk;
pub mod instance;
pub mod kinds;
pub mod node;
pub mod parse;
pub mod project_kinds;
pub mod schema;
pub mod tree;
pub mod ui;
pub mod value;
pub mod write;

pub use chunk::{Chunk, ChunkData, CHUNK_CELLS, CHUNK_SIZE};
pub use instance::{resolve, SceneSource};
pub use node::{Connection, Node, Override, ParentRef, Transform};
pub use parse::{
    parse, parse_property_literal, parse_value_literal, property_type_of, ParseOutput, SceneDoc,
    FORMAT_TAG, FORMAT_VERSION,
};
pub use schema::{KindRegistry, NodeKindSchema, PropertySchema, PropertyType, RESERVED_KEYS};
pub use tree::Scene;
pub use value::{Color, Reference, Value};
