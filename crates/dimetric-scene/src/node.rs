//! Nodes and transforms.

use dimetric_core::{Angle, NodeId, NodeUid, Vec2Fx};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::value::{Reference, Value};

/// A position, rotation and scale.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Transform {
    /// Translation.
    pub pos: Vec2Fx,
    /// Rotation.
    pub rot: Angle,
    /// Scale, per axis.
    pub scale: Vec2Fx,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform::IDENTITY
    }
}

impl Transform {
    /// No translation, no rotation, unit scale.
    pub const IDENTITY: Transform = Transform {
        pos: Vec2Fx::ZERO,
        rot: Angle::ZERO,
        scale: Vec2Fx::ONE,
    };

    /// Compose: apply `self` as the parent of `child`.
    ///
    /// Scale is applied before rotation, which is the only order that keeps a
    /// rotated child rigid under a non-uniform parent scale.
    pub fn compose(self, child: Transform) -> Transform {
        let scaled = child.pos.mul_components(self.scale);
        Transform {
            pos: self.pos + scaled.rotated(self.rot),
            rot: self.rot + child.rot,
            scale: self.scale.mul_components(child.scale),
        }
    }

    /// Map a point from this transform's local space into its parent's.
    pub fn apply(self, point: Vec2Fx) -> Vec2Fx {
        self.pos + point.mul_components(self.scale).rotated(self.rot)
    }

    /// Map a point from the parent's space into this transform's local space.
    pub fn inverse_apply(self, point: Vec2Fx) -> Vec2Fx {
        let local = (point - self.pos).rotated(-self.rot);
        Vec2Fx::new(
            if self.scale.x.is_zero() {
                dimetric_core::Fx::ZERO
            } else {
                local.x / self.scale.x
            },
            if self.scale.y.is_zero() {
                dimetric_core::Fx::ZERO
            } else {
                local.y / self.scale.y
            },
        )
    }
}

/// How a node names its parent.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ParentRef {
    /// The scene root.
    #[default]
    None,
    /// A node in this file.
    Node(NodeUid),
    /// A node inside an instance, written `<instance>/<inner>`.
    ///
    /// This is how a child is added inside a prefab instance without editing
    /// the prefab.
    Inner {
        /// The `Instance` node in this file.
        instance: NodeUid,
        /// The node id in the source scene to attach under.
        inner: NodeUid,
    },
}

impl ParentRef {
    /// Parse the `parent` key.
    pub fn parse(s: &str) -> Result<ParentRef, dimetric_core::id::UidError> {
        match s.split_once('/') {
            Some((outer, inner)) => Ok(ParentRef::Inner {
                instance: NodeUid::parse(outer)?,
                inner: NodeUid::parse(inner)?,
            }),
            None => Ok(ParentRef::Node(NodeUid::parse(s)?)),
        }
    }

    /// Render for the `parent` key. `None` has no text form.
    pub fn to_text(&self) -> Option<String> {
        match self {
            ParentRef::None => None,
            ParentRef::Node(u) => Some(u.to_text()),
            ParentRef::Inner { instance, inner } => {
                Some(format!("{}/{}", instance.to_text(), inner.to_text()))
            }
        }
    }

    /// The node in this file that ultimately owns the child.
    pub fn owner(&self) -> Option<NodeUid> {
        match self {
            ParentRef::None => None,
            ParentRef::Node(u) => Some(*u),
            ParentRef::Inner { instance, .. } => Some(*instance),
        }
    }
}

/// A node in a scene.
///
/// Hierarchy lives in the sibling links rather than a `Vec<NodeId>` of
/// children. A vector of children reallocates and reorders as the tree is
/// edited, and reordering is exactly the thing a deterministic engine must not
/// do casually. Links also make reparent an O(1) pointer rewrite.
///
/// `prev_sibling` and `last_child` are not in the design document's list. They
/// cost two words per node and turn unlink and append from O(n) walks into
/// O(1), which matters once a tile layer has a few thousand siblings.
#[derive(Clone, Debug)]
pub struct Node {
    /// Permanent identity, as written in the file.
    pub uid: NodeUid,
    /// Registered node kind.
    pub kind: String,
    /// Human-facing name, unique among siblings.
    pub name: String,
    /// Local transform.
    pub transform: Transform,
    /// Drawn and ticked when true.
    pub visible: bool,
    /// Sort key within a layer.
    pub z: i32,
    /// Render layer.
    pub layer: i32,
    /// Free-form tags, used by scripts and collision filters.
    pub tags: Vec<String>,
    /// Attached script.
    pub script: Option<Reference>,
    /// Source scene, for an `Instance`.
    pub scene: Option<Reference>,
    /// When this node was declared with a `<instance>/<inner>` parent, the
    /// source-scene id it attaches under.
    ///
    /// Kept out of [`Node::props`] so it can never collide with a kind
    /// property or leak into a state hash as one.
    pub inner_parent: Option<NodeUid>,
    /// Kind-specific properties. Ordered, so writes are stable.
    pub props: IndexMap<String, Value>,

    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub(crate) prev_sibling: Option<NodeId>,

    pub(crate) world: Transform,
    pub(crate) world_dirty: bool,
}

impl Node {
    /// A node with default reserved values and no properties.
    pub fn new(uid: NodeUid, kind: impl Into<String>, name: impl Into<String>) -> Node {
        Node {
            uid,
            kind: kind.into(),
            name: name.into(),
            transform: Transform::IDENTITY,
            visible: true,
            z: 0,
            layer: 0,
            tags: Vec::new(),
            script: None,
            scene: None,
            inner_parent: None,
            props: IndexMap::new(),
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
            world: Transform::IDENTITY,
            world_dirty: true,
        }
    }

    /// The parent handle, if any.
    #[inline]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// The first child handle, if any.
    #[inline]
    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    /// The next sibling handle, if any.
    #[inline]
    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling
    }

    /// The cached world transform.
    ///
    /// Valid only after [`Scene::update_world_transforms`](crate::Scene::update_world_transforms).
    /// World transforms are derived and never serialized — two sources of truth
    /// for the same position eventually disagree.
    #[inline]
    pub fn world(&self) -> Transform {
        self.world
    }

    /// True when a tag is present.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Read a kind property.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    /// Write a kind property, returning what was there.
    pub fn set(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        self.props.insert(key.into(), value)
    }
}

/// A signal connection.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Connection {
    /// Node emitting the signal.
    pub from: NodeUid,
    /// Signal name.
    pub signal: String,
    /// Node receiving it.
    pub to: NodeUid,
    /// Method called on the receiving node's script.
    pub method: String,
}

/// A sparse override applied to one node inside one instance.
#[derive(Clone, PartialEq, Debug)]
pub struct Override {
    /// The `Instance` node in this file.
    pub instance: NodeUid,
    /// The node id **in the source scene**.
    ///
    /// A source id rather than a name path on purpose. Name paths read better
    /// and shatter the moment someone renames a child in the prefab — silently,
    /// across every scene that instances it. Source ids are permanent, so
    /// renames cost nothing.
    pub target: NodeUid,
    /// When true, the inherited node is dropped from this instance.
    pub removed: bool,
    /// Properties to replace on the target.
    pub props: IndexMap<String, Value>,
}

impl Override {
    /// An empty override block.
    pub fn new(instance: NodeUid, target: NodeUid) -> Override {
        Override {
            instance,
            target,
            removed: false,
            props: IndexMap::new(),
        }
    }
}
