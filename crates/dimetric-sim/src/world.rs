//! The physics world: broadphase, sweeping and contact resolution.
//!
//! Rebuilt from the scene at the start of every physics phase rather than kept
//! incrementally in sync. A cache that can disagree with the scene eventually
//! does, and the rebuild is a linear walk of the tree — cheap next to the
//! collision work itself.

use std::collections::BTreeMap;

use dimetric_core::{Fx, NodeId, NodeUid, StateHasher, Vec2Fx};
use dimetric_scene::{Scene, Value};

use crate::shape::{overlap, Shape};
use crate::sweep::{sweep_aabb, Hit, SKIN};

/// Width of one broadphase cell, in world units.
///
/// Tuned to roughly the size of a large character. Too small and a body spans
/// many cells; too large and every query returns the whole room.
pub const CELL_SIZE: i32 = 64;

/// Slide iterations per move.
///
/// Three is enough to handle an inside corner — hit a wall, slide, hit the
/// perpendicular wall, stop. A fourth almost never fires and an unbounded loop
/// can grind in a wedge.
pub const MAX_SLIDES: usize = 3;

/// One collidable body, extracted from a `Collider` or `Area` node.
#[derive(Clone, Debug)]
pub struct Body {
    /// The node this came from.
    pub node: NodeId,
    /// Its permanent id, which is also the contact sort key.
    pub uid: NodeUid,
    /// Shape in local space.
    pub shape: Shape,
    /// World position at the start of the phase.
    pub pos: Vec2Fx,
    /// Never swept, never pushed.
    pub is_static: bool,
    /// Reports overlaps and blocks nothing.
    pub is_area: bool,
    /// Bitmask of layers this body occupies.
    pub layer: u32,
    /// Bitmask of layers this body tests against.
    pub mask: u32,
    /// A bloom filter over the node's tags.
    ///
    /// One bit per tag, hashed into sixty-four. A miss is exact — the bit is
    /// definitely absent — and a hit needs confirming. That is enough to skip
    /// most of the work: a tagged query over a scene full of projectiles spends
    /// its time on candidates that cannot possibly match, and an integer test
    /// beats two map lookups and a string compare several hundred thousand
    /// times a tick.
    pub tags: u64,
}

/// The bit a tag sets in [`Body::tags`].
pub fn tag_bit(tag: &str) -> u64 {
    // FNV-1a, because it is short, stable across builds and platforms, and the
    // exact spread does not matter for a filter that is confirmed on a hit.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in tag.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    1u64 << (hash % 64)
}

impl Body {
    /// True when these two bodies are allowed to interact at all.
    ///
    /// Symmetric: either side's mask matching the other's layer is enough, so
    /// a projectile only has to declare what it hits and walls do not have to
    /// know about projectiles.
    pub fn interacts_with(&self, other: &Body) -> bool {
        (self.mask & other.layer) != 0 || (other.mask & self.layer) != 0
    }
}

/// A reported contact, in a stable order.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContactEvent {
    /// The moving body.
    pub a: NodeUid,
    /// What it touched.
    pub b: NodeUid,
    /// Surface normal, pointing back toward `a`.
    pub normal: Vec2Fx,
    /// True when `b` is an `Area` and nothing was blocked.
    pub trigger: bool,
}

/// Bodies plus a uniform spatial hash over them.
#[derive(Clone, Debug, Default)]
pub struct PhysicsWorld {
    bodies: Vec<Body>,
    /// Cell coordinate to body indices.
    ///
    /// A `BTreeMap` rather than a `HashMap`: bucket iteration reaches contact
    /// order, and hash iteration order is not stable across runs (I4).
    grid: BTreeMap<(i32, i32), Vec<usize>>,
}

impl PhysicsWorld {
    /// Extract every collidable body from a scene.
    ///
    /// Walks the tree depth-first, so body indices — and therefore every
    /// tie-break that falls back on them — are determined by the scene's shape
    /// rather than by allocation order.
    pub fn build(scene: &Scene) -> PhysicsWorld {
        let mut world = PhysicsWorld::default();
        for id in scene.walk() {
            let Some(node) = scene.get(id) else { continue };
            if !node.visible {
                continue;
            }
            let is_area = node.base == "Area";
            if node.base != "Collider" && !is_area {
                continue;
            }
            if is_area && !prop_bool(node, "monitoring", true) {
                continue;
            }
            let Some(shape) = shape_of(node) else {
                continue;
            };
            let world_transform = scene
                .world_of(id)
                .map(|t| t.pos)
                .unwrap_or(node.transform.pos);
            world.bodies.push(Body {
                node: id,
                uid: node.uid,
                shape,
                pos: world_transform,
                is_static: prop_bool(node, "is_static", false),
                is_area,
                layer: prop_int(node, "collision_layer", 1) as u32,
                mask: prop_int(node, "collision_mask", 1) as u32,
                tags: node.tags.iter().fold(0, |bits, t| bits | tag_bit(t)),
            });
        }
        world.reindex();
        world
    }

    /// Rebuild the spatial hash from the current body positions.
    pub fn reindex(&mut self) {
        self.grid.clear();
        for (index, body) in self.bodies.iter().enumerate() {
            for cell in cells_of(body) {
                self.grid.entry(cell).or_default().push(index);
            }
        }
    }

    /// Every body.
    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// Find a body by node id.
    pub fn index_of(&self, node: NodeId) -> Option<usize> {
        self.bodies.iter().position(|b| b.node == node)
    }

    /// Candidate bodies near a box, in ascending index order and without
    /// duplicates.
    pub fn candidates(&self, center: Vec2Fx, half: Vec2Fx) -> Vec<usize> {
        let min = cell_of(center - half);
        let max = cell_of(center + half);
        let mut out = Vec::new();
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                if let Some(bucket) = self.grid.get(&(x, y)) {
                    out.extend_from_slice(bucket);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Move a body toward `target`, sliding along whatever blocks it.
    ///
    /// Returns where it ended up and everything it touched on the way. Areas
    /// are reported and never block.
    pub fn move_body(&self, index: usize, target: Vec2Fx) -> (Vec2Fx, Vec<ContactEvent>) {
        let body = &self.bodies[index];
        let mut contacts = Vec::new();
        let mut pos = body.pos;
        let mut remaining = target - body.pos;
        let half = body.shape.bounds_half();

        for _ in 0..MAX_SLIDES {
            if remaining.is_zero() {
                break;
            }
            let Some((hit, other)) = self.earliest_hit(index, pos, half, remaining, &mut contacts)
            else {
                pos += remaining;
                remaining = Vec2Fx::ZERO;
                break;
            };

            // Advance to just before contact, then continue with whatever
            // motion survives being projected onto the surface. This is the
            // whole of "slide along the wall".
            let travelled = remaining * hit.toi;
            pos += travelled - hit.normal * -SKIN;
            contacts.push(ContactEvent {
                a: body.uid,
                b: self.bodies[other].uid,
                normal: hit.normal,
                trigger: false,
            });
            remaining = (remaining - travelled).slid_along(hit.normal);
        }
        if !remaining.is_zero() {
            pos += remaining;
        }

        self.collect_triggers(index, pos, half, &mut contacts);
        contacts.sort_by_key(|c| (c.b.body().to_string(), c.a.body().to_string()));
        contacts.dedup_by_key(|c| (c.a, c.b));
        (pos, contacts)
    }

    /// The first blocking body the sweep meets, if any.
    fn earliest_hit(
        &self,
        index: usize,
        pos: Vec2Fx,
        half: Vec2Fx,
        motion: Vec2Fx,
        _contacts: &mut Vec<ContactEvent>,
    ) -> Option<(Hit, usize)> {
        let body = &self.bodies[index];
        // Query the box that covers the whole step, so nothing is missed
        // between the start and end cells.
        let swept_center = pos + motion / 2;
        let swept_half = half + (motion / 2).abs();

        let mut best: Option<(Hit, usize)> = None;
        for other_index in self.candidates(swept_center, swept_half) {
            if other_index == index {
                continue;
            }
            let other = &self.bodies[other_index];
            if other.is_area || !body.interacts_with(other) {
                continue;
            }
            let Some(hit) = sweep_aabb(pos, half, motion, other.pos, other.shape.bounds_half())
            else {
                continue;
            };
            let better = match &best {
                None => true,
                // Ties break on the body's permanent id, never on index or
                // address, so contact order is reproducible (I4).
                Some((b, bi)) => {
                    hit.toi < b.toi
                        || (hit.toi == b.toi && other.uid.body() < self.bodies[*bi].uid.body())
                }
            };
            if better {
                best = Some((hit, other_index));
            }
        }
        best
    }

    /// Areas the body ends the step inside.
    fn collect_triggers(
        &self,
        index: usize,
        pos: Vec2Fx,
        half: Vec2Fx,
        contacts: &mut Vec<ContactEvent>,
    ) {
        let body = &self.bodies[index];
        for other_index in self.candidates(pos, half) {
            if other_index == index {
                continue;
            }
            let other = &self.bodies[other_index];
            if !other.is_area || !body.interacts_with(other) {
                continue;
            }
            if let Some(contact) = overlap(&body.shape, pos, &other.shape, other.pos) {
                contacts.push(ContactEvent {
                    a: body.uid,
                    b: other.uid,
                    normal: contact.normal,
                    trigger: true,
                });
            }
        }
    }

    /// Push a body out of anything it is currently inside.
    ///
    /// Sweeping handles motion; this handles the cases sweeping cannot — a
    /// body spawned inside a wall, or one a script teleported.
    pub fn depenetrate(&self, index: usize, pos: Vec2Fx) -> Vec2Fx {
        let body = &self.bodies[index];
        let mut out = pos;
        for other_index in self.candidates(pos, body.shape.bounds_half()) {
            if other_index == index {
                continue;
            }
            let other = &self.bodies[other_index];
            if other.is_area || !body.interacts_with(other) {
                continue;
            }
            if let Some(contact) = overlap(&body.shape, out, &other.shape, other.pos) {
                out += contact.normal * (contact.depth + SKIN);
            }
        }
        out
    }

    /// What a sensor at `pos` is overlapping, in id order.
    ///
    /// A sensor reports and does not resolve, so unlike [`PhysicsWorld::move_body`]
    /// this never moves anything — it answers "what am I touching here".
    pub fn overlaps(&self, index: usize, pos: Vec2Fx) -> Vec<ContactEvent> {
        let body = &self.bodies[index];
        let mut out = Vec::new();
        for other_index in self.candidates(pos, body.shape.bounds_half()) {
            if other_index == index {
                continue;
            }
            let other = &self.bodies[other_index];
            if !body.interacts_with(other) {
                continue;
            }
            if let Some(contact) = overlap(&body.shape, pos, &other.shape, other.pos) {
                out.push(ContactEvent {
                    a: body.uid,
                    b: other.uid,
                    normal: contact.normal,
                    // Always a trigger: a sensor blocks nothing by definition.
                    trigger: true,
                });
            }
        }
        // Sorted, so what a sensor reports never depends on cell iteration
        // order or on how the bodies happened to be laid out in memory (I4).
        out.sort_by_key(|c| c.b.body().to_string());
        out
    }

    /// The body nearest `at` within `radius` that `accept` allows.
    ///
    /// Scans the candidate cells and keeps a running minimum, rather than
    /// building the sorted list [`PhysicsWorld::within`] returns and then
    /// taking its head. That matters more than it sounds: a few hundred homing
    /// projectiles each asking for their nearest enemy is the hot path of a
    /// bullet-heavy game, and the allocation per call was most of its cost.
    ///
    /// Ties break on the id, so "the nearest" is the same body everywhere (I4).
    pub fn nearest(
        &self,
        at: Vec2Fx,
        radius: Fx,
        tag_filter: u64,
        accept: impl Fn(NodeUid) -> bool,
    ) -> Option<NodeUid> {
        let half = Vec2Fx::new(radius, radius);
        let limit = radius.wide() * radius.wide();
        let min = cell_of(at - half);
        let max = cell_of(at + half);

        let mut best: Option<(dimetric_core::FxWide, NodeUid)> = None;
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                let Some(bucket) = self.grid.get(&(x, y)) else {
                    continue;
                };
                for index in bucket {
                    let body = &self.bodies[*index];
                    // The cheap tests first: a bit that is absent is absent.
                    if tag_filter != 0 && body.tags & tag_filter == 0 {
                        continue;
                    }
                    let distance = (body.pos - at).length_squared();
                    if distance > limit || !accept(body.uid) {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some((d, uid)) => (distance, body.uid.body()) < (*d, uid.body()),
                    };
                    if better {
                        best = Some((distance, body.uid));
                    }
                }
            }
        }
        best.map(|(_, uid)| uid)
    }

    /// Every body whose centre is within `radius` of `at`, in id order.
    ///
    /// Centres rather than shapes, because "what is near me" is a question
    /// about where things are, and a caller that wants overlap has
    /// [`PhysicsWorld::overlaps`].
    pub fn within(&self, at: Vec2Fx, radius: Fx) -> Vec<NodeUid> {
        let half = Vec2Fx::new(radius, radius);
        let limit = radius.wide() * radius.wide();
        let mut out: Vec<NodeUid> = self
            .candidates(at, half)
            .into_iter()
            .filter(|i| (self.bodies[*i].pos - at).length_squared() <= limit)
            .map(|i| self.bodies[i].uid)
            .collect();
        // Sorted, so a script sees the same order everywhere (I4). By the body
        // bytes rather than an owned string: this runs per query per tick, and
        // allocating to compare was most of what it cost.
        out.sort_by(|a, b| a.body().cmp(b.body()));
        out.dedup();
        out
    }

    /// Every body whose shape contains `point`, in id order.
    pub fn query_point(&self, point: Vec2Fx) -> Vec<NodeUid> {
        let mut out: Vec<NodeUid> = self
            .candidates(point, Vec2Fx::ZERO)
            .into_iter()
            .filter(|i| self.bodies[*i].shape.contains(self.bodies[*i].pos, point))
            .map(|i| self.bodies[i].uid)
            .collect();
        out.sort_by_key(|u| u.body().to_string());
        out
    }

    /// Feed body layout into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.tag("bodies").len(self.bodies.len());
        for b in &self.bodies {
            h.node_uid(b.uid);
            h.vec2(b.pos);
            h.bool(b.is_static);
            h.bool(b.is_area);
            h.u64(b.layer as u64);
            h.u64(b.mask as u64);
        }
    }
}

fn cell_of(p: Vec2Fx) -> (i32, i32) {
    (
        p.x.floor_int().div_euclid(CELL_SIZE),
        p.y.floor_int().div_euclid(CELL_SIZE),
    )
}

fn cells_of(body: &Body) -> Vec<(i32, i32)> {
    let bounds = body.shape.bounds_at(body.pos);
    let min = cell_of(bounds.min());
    let max = cell_of(bounds.max());
    let mut out = Vec::new();
    for y in min.1..=max.1 {
        for x in min.0..=max.0 {
            out.push((x, y));
        }
    }
    out
}

fn shape_of(node: &dimetric_scene::Node) -> Option<Shape> {
    match node.get("shape").and_then(Value::as_str).unwrap_or("AABB") {
        "Circle" => Some(Shape::Circle {
            radius: node
                .get("radius")
                .and_then(Value::as_scalar)
                .unwrap_or(Fx::from_int(8)),
        }),
        "Polygon" => {
            let points = match node.get("points") {
                Some(Value::List(items)) => {
                    items.iter().filter_map(Value::as_vec2).collect::<Vec<_>>()
                }
                _ => Vec::new(),
            };
            if points.len() < 3 {
                None
            } else {
                Some(Shape::Polygon { points })
            }
        }
        _ => Some(Shape::box_of(
            node.get("size")
                .and_then(Value::as_vec2)
                .unwrap_or(Vec2Fx::from_ints(16, 16)),
        )),
    }
}

fn prop_bool(node: &dimetric_scene::Node, key: &str, default: bool) -> bool {
    node.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn prop_int(node: &dimetric_scene::Node, key: &str, default: i64) -> i64 {
    node.get(key).and_then(Value::as_int).unwrap_or(default)
}
