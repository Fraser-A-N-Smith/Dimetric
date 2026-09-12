//! The node tree.

use dimetric_core::{Code, Diagnostic, NodeId, NodeUid, StateHasher, Vec2Fx};
use indexmap::IndexMap;
use slotmap::SlotMap;

use crate::chunk::Chunk;
use crate::node::{Connection, Node, Override, Transform};

/// A loaded scene: its nodes, their hierarchy, and the connection, override
/// and tile-chunk blocks that go with them.
///
/// Handles are generational, so holding a [`NodeId`] across a delete gives a
/// clean `None` rather than whatever was allocated in its place.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    nodes: SlotMap<NodeId, Node>,
    by_uid: IndexMap<NodeUid, NodeId>,
    root: Option<NodeId>,
    /// Signal connections, sorted by `from` then `signal` in canonical form.
    pub connections: Vec<Connection>,
    /// Instance overrides, grouped by instance in canonical form.
    pub overrides: Vec<Override>,
    /// Tile chunks, sorted by layer then coordinate in canonical form.
    pub chunks: Vec<Chunk>,
}

impl Scene {
    /// An empty scene.
    pub fn new() -> Scene {
        Scene::default()
    }

    /// The root node, if the scene has one.
    #[inline]
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    /// How many nodes the scene holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when there are no nodes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Borrow a node.
    #[inline]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Borrow a node mutably.
    ///
    /// Touching [`Node::transform`] through this invalidates the cached world
    /// transform of the node and everything beneath it, so callers that only
    /// change a property should prefer [`Scene::node_mut_no_transform`].
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        if self.nodes.contains_key(id) {
            self.mark_subtree_dirty(id);
        }
        self.nodes.get_mut(id)
    }

    /// Borrow a node mutably without invalidating transforms.
    #[inline]
    pub fn node_mut_no_transform(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Find a node by its permanent id.
    #[inline]
    pub fn by_uid(&self, uid: NodeUid) -> Option<NodeId> {
        self.by_uid.get(&uid).copied()
    }

    /// True when the id is in use.
    #[inline]
    pub fn contains_uid(&self, uid: NodeUid) -> bool {
        self.by_uid.contains_key(&uid)
    }

    /// Every node, in insertion order. Deterministic, unlike slotmap order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.by_uid.values().map(|id| (*id, &self.nodes[*id]))
    }

    /// Insert a node under `parent`, or as the root when `parent` is `None`.
    ///
    /// Fails with `DIM0102` on a duplicate id and `DIM0105` on a duplicate
    /// sibling name.
    pub fn insert(&mut self, mut node: Node, parent: Option<NodeId>) -> Result<NodeId, Diagnostic> {
        // Hierarchy links belong to the scene that issued them. A node cloned
        // out of another scene — which is exactly what instancing does — still
        // carries that scene's handles, and they index nothing here.
        node.parent = None;
        node.first_child = None;
        node.last_child = None;
        node.next_sibling = None;
        node.prev_sibling = None;
        node.world_dirty = true;

        if self.by_uid.contains_key(&node.uid) {
            return Err(Diagnostic::new(
                Code::DUPLICATE_ID,
                format!("node id {} is already used in this scene", node.uid),
            )
            .with_field("id", node.uid.to_text()));
        }
        if let Some(p) = parent {
            if let Some(existing) = self.child_named(p, &node.name) {
                let path = self.path_of(existing).unwrap_or_default();
                return Err(Diagnostic::new(
                    Code::DUPLICATE_NAME,
                    format!("{:?} already exists under {}", node.name, path),
                )
                .with_field("name", node.name.clone())
                .with_node(path));
            }
        } else if self.root.is_some() {
            return Err(Diagnostic::new(
                Code::DANGLING_PARENT,
                format!(
                    "node {} has no parent, but the scene already has a root",
                    node.uid
                ),
            )
            .with_field("id", node.uid.to_text()));
        }

        let uid = node.uid;
        let id = self.nodes.insert(node);
        self.by_uid.insert(uid, id);
        match parent {
            Some(p) => self.link_child(p, id),
            None => self.root = Some(id),
        }
        self.mark_subtree_dirty(id);
        Ok(id)
    }

    /// The child of `parent` with this name.
    pub fn child_named(&self, parent: NodeId, name: &str) -> Option<NodeId> {
        self.children(parent).find(|c| self.nodes[*c].name == name)
    }

    /// The children of a node, in order.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            scene: self,
            next: self.nodes.get(id).and_then(|n| n.first_child),
        }
    }

    /// Every node beneath `id`, depth first, parents before children.
    ///
    /// This is canonical order: the order the file is written in, and the order
    /// transforms propagate in.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            // Reversed, so that popping yields siblings in declaration order.
            let kids: Vec<NodeId> = self.children(n).collect();
            stack.extend(kids.into_iter().rev());
        }
        out
    }

    /// Every node in the scene, depth first from the root.
    pub fn walk(&self) -> Vec<NodeId> {
        match self.root {
            Some(r) => self.descendants(r),
            None => Vec::new(),
        }
    }

    /// The scene path of a node, such as `/Arena01/Player/Sprite`.
    pub fn path_of(&self, id: NodeId) -> Option<String> {
        let mut parts = Vec::new();
        let mut cursor = Some(id);
        while let Some(c) = cursor {
            let node = self.nodes.get(c)?;
            parts.push(node.name.as_str());
            cursor = node.parent;
        }
        parts.reverse();
        Some(format!("/{}", parts.join("/")))
    }

    /// Resolve a scene path.
    ///
    /// Paths are derived from names and are never stored as references — a
    /// rename would silently break every one of them. They exist for scripts,
    /// the CLI and agents, where a readable address is worth more than a stable
    /// one.
    pub fn resolve_path(&self, path: &str) -> Option<NodeId> {
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        let root = self.root?;
        let first = segments.next()?;
        if self.nodes[root].name != first {
            return None;
        }
        let mut cursor = root;
        for seg in segments {
            cursor = self.child_named(cursor, seg)?;
        }
        Some(cursor)
    }

    /// Rename a node, keeping sibling names unique (`DIM0105`).
    pub fn rename(&mut self, id: NodeId, name: impl Into<String>) -> Result<String, Diagnostic> {
        let name = name.into();
        let parent = self.nodes.get(id).and_then(|n| n.parent);
        if let Some(p) = parent {
            if let Some(other) = self.child_named(p, &name) {
                if other != id {
                    return Err(Diagnostic::new(
                        Code::DUPLICATE_NAME,
                        format!("a sibling is already called {name:?}"),
                    )
                    .with_field("name", name));
                }
            }
        }
        let node = self.nodes.get_mut(id).ok_or_else(|| missing(id))?;
        Ok(std::mem::replace(&mut node.name, name))
    }

    /// Move a node under a new parent.
    ///
    /// Fails with `DIM0402` if the move would put a node inside its own
    /// subtree, and `DIM0105` on a name collision at the destination.
    pub fn reparent(
        &mut self,
        id: NodeId,
        new_parent: NodeId,
    ) -> Result<Option<NodeId>, Diagnostic> {
        if id == new_parent || self.is_ancestor(id, new_parent) {
            return Err(Diagnostic::new(
                Code::ILLEGAL_REPARENT,
                "a node cannot be moved inside its own subtree",
            )
            .with_node(self.path_of(id).unwrap_or_default()));
        }
        let name = self.nodes.get(id).ok_or_else(|| missing(id))?.name.clone();
        if let Some(other) = self.child_named(new_parent, &name) {
            if other != id {
                return Err(Diagnostic::new(
                    Code::DUPLICATE_NAME,
                    format!(
                        "{:?} already exists under {}",
                        name,
                        self.path_of(new_parent).unwrap_or_default()
                    ),
                )
                .with_field("name", name));
            }
        }
        let old = self.nodes[id].parent;
        self.unlink(id);
        self.link_child(new_parent, id);
        self.mark_subtree_dirty(id);
        Ok(old)
    }

    /// True when `ancestor` is at or above `id`.
    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cursor = Some(id);
        while let Some(c) = cursor {
            if c == ancestor {
                return true;
            }
            cursor = self.nodes.get(c).and_then(|n| n.parent);
        }
        false
    }

    /// Remove a node and everything beneath it, returning the removed nodes in
    /// depth-first order so the operation can be undone exactly.
    pub fn remove_subtree(&mut self, id: NodeId) -> Vec<Node> {
        let doomed = self.descendants(id);
        self.unlink(id);
        if self.root == Some(id) {
            self.root = None;
        }
        let mut removed = Vec::with_capacity(doomed.len());
        for n in doomed {
            if let Some(node) = self.nodes.remove(n) {
                self.by_uid.shift_remove(&node.uid);
                removed.push(node);
            }
        }
        removed
    }

    /// Recompute every stale world transform in one depth-first pass.
    ///
    /// Called once per tick rather than on every write, so a script moving a
    /// node a hundred times in one tick still costs one propagation.
    pub fn update_world_transforms(&mut self) {
        let Some(root) = self.root else { return };
        let order = self.descendants(root);
        for id in order {
            let (parent_world, dirty) = {
                let node = &self.nodes[id];
                let parent_world = node
                    .parent
                    .map(|p| self.nodes[p].world)
                    .unwrap_or(Transform::IDENTITY);
                (parent_world, node.world_dirty)
            };
            if dirty {
                let node = &mut self.nodes[id];
                node.world = parent_world.compose(node.transform);
                node.world_dirty = false;
            }
        }
    }

    /// The world position of a node, recomputing on the spot.
    ///
    /// Convenient for one-off queries; use
    /// [`update_world_transforms`](Scene::update_world_transforms) plus
    /// [`Node::world`] when touching many nodes.
    pub fn world_of(&self, id: NodeId) -> Option<Transform> {
        let node = self.nodes.get(id)?;
        match node.parent {
            Some(p) => Some(self.world_of(p)?.compose(node.transform)),
            None => Some(node.transform),
        }
    }

    /// Set a node's local position and invalidate its subtree.
    pub fn set_position(&mut self, id: NodeId, pos: Vec2Fx) -> Option<Vec2Fx> {
        let old = self.nodes.get(id)?.transform.pos;
        self.nodes.get_mut(id)?.transform.pos = pos;
        self.mark_subtree_dirty(id);
        Some(old)
    }

    /// Mark a node and its descendants as needing a world transform.
    pub fn mark_subtree_dirty(&mut self, id: NodeId) {
        for n in self.descendants(id) {
            if let Some(node) = self.nodes.get_mut(n) {
                node.world_dirty = true;
            }
        }
    }

    /// Feed the whole tree into a state hash.
    ///
    /// Walks in depth-first order, which is defined by the tree rather than by
    /// allocation, so the hash is reproducible across machines (I4). World
    /// transforms are excluded because they are derived.
    pub fn hash_state(&self, h: &mut StateHasher) {
        let order = self.walk();
        h.tag("nodes").len(order.len());
        for id in order {
            let node = &self.nodes[id];
            h.node_uid(node.uid);
            h.str(&node.kind);
            h.str(&node.name);
            h.vec2(node.transform.pos);
            h.angle(node.transform.rot);
            h.vec2(node.transform.scale);
            h.bool(node.visible);
            h.i32(node.z);
            h.i32(node.layer);
            h.len(node.tags.len());
            for t in &node.tags {
                h.str(t);
            }
            let mut keys: Vec<&String> = node.props.keys().collect();
            keys.sort();
            h.len(keys.len());
            for k in keys {
                h.str(k);
                node.props[k].hash_state(h);
            }
        }
        h.tag("connections").len(self.connections.len());
        for c in &self.connections {
            h.node_uid(c.from);
            h.str(&c.signal);
            h.node_uid(c.to);
            h.str(&c.method);
        }
        h.tag("chunks").len(self.chunks.len());
        for c in &self.chunks {
            c.hash_state(h);
        }
    }

    // -- hierarchy plumbing ------------------------------------------------

    fn link_child(&mut self, parent: NodeId, child: NodeId) {
        let last = self.nodes[parent].last_child;
        {
            let node = &mut self.nodes[child];
            node.parent = Some(parent);
            node.prev_sibling = last;
            node.next_sibling = None;
        }
        match last {
            Some(l) => self.nodes[l].next_sibling = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    fn unlink(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let (parent, prev, next) = (node.parent, node.prev_sibling, node.next_sibling);
        if let Some(p) = prev {
            self.nodes[p].next_sibling = next;
        }
        if let Some(n) = next {
            self.nodes[n].prev_sibling = prev;
        }
        if let Some(p) = parent {
            if self.nodes[p].first_child == Some(id) {
                self.nodes[p].first_child = next;
            }
            if self.nodes[p].last_child == Some(id) {
                self.nodes[p].last_child = prev;
            }
        }
        let node = &mut self.nodes[id];
        node.parent = None;
        node.prev_sibling = None;
        node.next_sibling = None;
    }
}

fn missing(id: NodeId) -> Diagnostic {
    Diagnostic::new(Code::NO_SUCH_NODE, format!("no such node: {id:?}"))
}

/// Iterator over a node's children.
pub struct Children<'a> {
    scene: &'a Scene,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.scene.nodes.get(current).and_then(|n| n.next_sibling);
        Some(current)
    }
}
