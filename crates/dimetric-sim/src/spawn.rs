//! Creating nodes while the simulation runs.
//!
//! # Why this is not just `Scene::insert`
//!
//! Two things have to hold, and neither is automatic.
//!
//! A spawn is **deferred to a phase boundary**, like a destroy. A script that
//! inserted a node mid-tick would be inserting it into a tree another script
//! might be half-way through walking, and which of them saw it would depend on
//! the order the scene happened to be traversed.
//!
//! A spawn's **id is derived, not drawn**. Ids come from the template's id and
//! a counter that is part of the state, so the same run produces the same ids
//! on every machine and a rollback that re-runs a tick re-uses them rather than
//! inventing new ones. Drawing from the RNG would work too, but it would mean
//! that spawning one fewer projectile shifted every later gameplay roll.

use std::collections::BTreeMap;

use dimetric_core::{Diagnostic, NodeUid, Vec2Fx};
use dimetric_scene::Scene;

/// A prefab the simulation can stamp out, already resolved.
///
/// Resolved by the host at load, because flattening an instance needs the
/// project's other scenes and the simulation has no filesystem.
pub type Templates = BTreeMap<String, Scene>;

/// A request to create something, waiting for the end of the tick.
#[derive(Clone, PartialEq, Debug)]
pub struct Spawn {
    /// Template name, as `scene.spawn` was given it.
    pub template: String,
    /// Where to put the root, in the parent's space.
    pub at: Vec2Fx,
    /// Parent, or none for the scene root.
    pub parent: Option<NodeUid>,
    /// The id the root will get. Decided when the request is made, so the
    /// script can hold onto it and find the node next tick.
    pub id: NodeUid,
}

/// The id a spawn gets.
///
/// Derived from the template's root id and the spawn counter, so it is the same
/// on every machine and the same again after a rollback. Uses the generation
/// alphabet, so a derived id is indistinguishable from an authored one.
pub fn derive_uid(template_root: NodeUid, counter: u64) -> NodeUid {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"dimetric.spawn");
    hasher.update(template_root.body().as_bytes());
    hasher.update(&counter.to_le_bytes());
    let mut bits = u64::from_le_bytes(
        hasher.finalize().as_bytes()[..8]
            .try_into()
            .expect("blake3 is 32 bytes"),
    );
    let mut body = String::from("n_");
    for _ in 0..8 {
        body.push(ALPHABET[(bits & 31) as usize] as char);
        bits >>= 5;
    }
    NodeUid::parse(&body).expect("derived ids use the generation alphabet")
}

/// Graft a template's subtree into a scene.
///
/// Every node in the template gets an id derived from the root's new one, so a
/// prefab's children are as reproducible as its root and two copies of the same
/// prefab never collide.
pub fn graft(scene: &mut Scene, template: &Scene, request: &Spawn) -> Result<NodeUid, Diagnostic> {
    let Some(template_root) = template.root() else {
        return Err(Diagnostic::new(
            dimetric_core::Code::COMMAND_REJECTED,
            format!("template {:?} has no root", request.template),
        ));
    };

    let parent = request
        .parent
        .and_then(|uid| scene.by_uid(uid))
        .or_else(|| scene.root());

    // Depth first, parents before children, so a child's parent is always in
    // place by the time it is inserted.
    let mut mapping: BTreeMap<NodeUid, NodeUid> = BTreeMap::new();
    let mut name_counter = 0u32;
    for source_id in template.walk() {
        let Some(source) = template.get(source_id) else {
            continue;
        };
        let mut node = source.clone();

        let uid = if source_id == template_root {
            request.id
        } else {
            derive_uid(source.uid, hash_of(request.id, source.uid))
        };
        mapping.insert(source.uid, uid);
        node.uid = uid;

        let target_parent = match source.parent().and_then(|p| template.get(p)) {
            Some(p) => mapping.get(&p.uid).and_then(|u| scene.by_uid(*u)),
            None => parent,
        };

        if source_id == template_root {
            node.transform.pos = request.at;
            // A spawned root's name has to be unique among its siblings, and a
            // template's name is the same every time. Numbering keeps the tree
            // navigable by path without the insert failing on the second copy.
            let base = node.name.clone();
            while target_parent
                .and_then(|p| scene.child_named(p, &node.name))
                .is_some()
            {
                name_counter += 1;
                node.name = format!("{base}_{name_counter}");
            }
        }

        scene.insert(node, target_parent)?;
    }
    Ok(request.id)
}

/// A stable number from two ids, for deriving a child's id.
fn hash_of(root: NodeUid, source: NodeUid) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(root.body().as_bytes());
    hasher.update(source.body().as_bytes());
    u64::from_le_bytes(
        hasher.finalize().as_bytes()[..8]
            .try_into()
            .expect("blake3 is 32 bytes"),
    )
}
