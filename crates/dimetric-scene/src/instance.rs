//! Prefab instancing.
//!
//! An `Instance` node names another scene file and carries sparse overrides.
//! Resolving one grafts the source tree in beneath the instance node, applies
//! the overrides, and returns a flat runtime scene with no instances left in
//! it.
//!
//! This is built at M1 rather than later on purpose. Retrofitting scene
//! instancing into a data model that never had it is miserable, and it is the
//! highest-value feature in the whole model — enemy variants, prop sets and
//! room templates all fall out of it.

use std::collections::BTreeMap;

use dimetric_core::{Angle, Code, Diagnostic, Diagnostics, NodeId, NodeUid, Vec2Fx};

use crate::node::{Node, Override};
use crate::schema::{is_reserved, KindRegistry};
use crate::tree::Scene;
use crate::value::{Reference, Value};

/// Nesting depth beyond which resolution gives up.
///
/// Nesting is unlimited by design; this only exists so that a cycle the
/// `DIM0107` check somehow misses fails loudly instead of exhausting the stack.
pub const MAX_DEPTH: usize = 32;

/// Where resolved prefabs come from.
pub trait SceneSource {
    /// Load the scene a `scene:` reference names.
    fn load(&self, reference: &Reference) -> Result<Scene, Diagnostic>;
}

/// A source that knows nothing, for tests and for scenes with no instances.
pub struct NoSources;

impl SceneSource for NoSources {
    fn load(&self, reference: &Reference) -> Result<Scene, Diagnostic> {
        Err(
            Diagnostic::new(Code::ASSET_MISSING, format!("cannot load {reference}"))
                .with_field("reference", reference.to_text()),
        )
    }
}

/// Derive the runtime id of a node brought in by an instance.
///
/// Two instances of the same prefab would otherwise carry the same ids, and so
/// would a prefab that happens to share an id with the scene instancing it.
/// Derived ids are deterministic — the same instance and source id always
/// produce the same result — and never written to a file, so they cost nothing
/// in a diff.
pub fn derive_uid(instance: NodeUid, source: NodeUid) -> NodeUid {
    let mut hasher = blake3_of(instance, source);
    // Map 40 bits onto the generation alphabet.
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut body = String::with_capacity(10);
    body.push_str("n_");
    for _ in 0..8 {
        body.push(ALPHABET[(hasher & 31) as usize] as char);
        hasher >>= 5;
    }
    NodeUid::parse(&body).expect("derived ids use the generation alphabet")
}

fn blake3_of(a: NodeUid, b: NodeUid) -> u64 {
    let mut h = blake3::Hasher::new();
    h.update(a.body().as_bytes());
    h.update(b.body().as_bytes());
    u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().expect("8 bytes"))
}

/// Flatten every instance in `scene`.
///
/// Returns a scene containing no `Instance` nodes, plus anything that went
/// wrong. An unresolvable instance is reported and left in place as an empty
/// node rather than aborting the whole load — a designer with one broken
/// prefab reference should still be able to open the room.
pub fn resolve(
    scene: &Scene,
    sources: &dyn SceneSource,
    registry: &KindRegistry,
) -> (Scene, Diagnostics) {
    let mut stack = Vec::new();
    resolve_nested(scene, sources, registry, &mut stack, 0)
}

/// Resolve while carrying the enclosing instance stack.
///
/// The stack has to survive the recursion, or a prefab that instances itself
/// simply recurses until the process dies instead of reporting `DIM0107`.
fn resolve_nested(
    scene: &Scene,
    sources: &dyn SceneSource,
    registry: &KindRegistry,
    stack: &mut Vec<String>,
    depth: usize,
) -> (Scene, Diagnostics) {
    let mut diags = Diagnostics::new();
    let mut out = Scene::new();
    if let Some(root) = scene.root() {
        graft(
            scene, root, None, &mut out, sources, registry, &mut diags, stack, depth,
        );
    }
    // Connections and chunks belong to the scene as much as its nodes do.
    // Nodes that were not instanced keep their ids, and an instance root keeps
    // the instance's own id, so the outer file's blocks carry over unchanged.
    out.connections.extend(scene.connections.iter().cloned());
    out.chunks.extend(scene.chunks.iter().cloned());
    (out, diags)
}

#[allow(clippy::too_many_arguments)]
fn graft(
    src: &Scene,
    src_id: NodeId,
    dst_parent: Option<NodeId>,
    out: &mut Scene,
    sources: &dyn SceneSource,
    registry: &KindRegistry,
    diags: &mut Diagnostics,
    stack: &mut Vec<String>,
    depth: usize,
) -> Option<NodeId> {
    let node = src.get(src_id)?;

    // An `Instance` node does not survive resolution. The source scene's root
    // takes its place, wearing its name, transform and id, so that a reference
    // to the instance still resolves and `/Arena01/Skeleton_01` addresses what
    // a designer expects it to.
    let anchor = if node.base == "Instance" {
        expand_instance(
            src, src_id, dst_parent, out, sources, registry, diags, stack, depth,
        )?
    } else {
        let mut copy = node.clone();
        copy.inner_parent = None;
        match out.insert(copy, dst_parent) {
            Ok(id) => id,
            Err(d) => {
                diags.push(d);
                return None;
            }
        }
    };

    let instance_uid = node.uid;
    for child in src.children(src_id).collect::<Vec<_>>() {
        let Some(child_node) = src.get(child) else {
            continue;
        };
        // A child declared with an `<instance>/<inner>` parent attaches inside
        // the instantiated tree; a plain child attaches at its root.
        let target = match child_node.inner_parent {
            Some(inner) => match out.by_uid(derive_uid(instance_uid, inner)) {
                Some(id) => Some(id),
                None => {
                    diags.push(
                        Diagnostic::new(
                            Code::DANGLING_PARENT,
                            format!(
                                "{} attaches under {inner}, which the instanced scene does not contain",
                                child_node.uid
                            ),
                        )
                        .with_field("id", child_node.uid.to_text())
                        .with_field("inner", inner.to_text()),
                    );
                    continue;
                }
            },
            None => Some(anchor),
        };
        graft(
            src, child, target, out, sources, registry, diags, stack, depth,
        );
    }
    Some(anchor)
}

#[allow(clippy::too_many_arguments)]
fn expand_instance(
    src: &Scene,
    src_id: NodeId,
    dst_parent: Option<NodeId>,
    out: &mut Scene,
    sources: &dyn SceneSource,
    registry: &KindRegistry,
    diags: &mut Diagnostics,
    stack: &mut Vec<String>,
    depth: usize,
) -> Option<NodeId> {
    let node = src.get(src_id).expect("instance exists");
    let instance_uid = node.uid;
    let Some(reference) = node.scene.clone() else {
        diags.push(
            Diagnostic::new(
                Code::MISSING_KEY,
                "an Instance node needs a `scene` reference",
            )
            .with_node(src.path_of(src_id).unwrap_or_default())
            .with_field("id", instance_uid.to_text()),
        );
        return None;
    };

    let key = reference.to_text();
    if stack.contains(&key) {
        diags.push(
            Diagnostic::new(
                Code::INSTANCE_CYCLE,
                format!("{key} instances itself, via {}", stack.join(" -> ")),
            )
            .with_node(src.path_of(src_id).unwrap_or_default())
            .with_field("reference", key),
        );
        return None;
    }
    if depth >= MAX_DEPTH {
        diags.push(
            Diagnostic::new(
                Code::INSTANCE_CYCLE,
                format!("instance nesting passed {MAX_DEPTH} levels at {key}"),
            )
            .with_field("reference", key),
        );
        return None;
    }

    let source = match sources.load(&reference) {
        Ok(s) => s,
        Err(d) => {
            diags.push(d.with_node(src.path_of(src_id).unwrap_or_default()));
            return None;
        }
    };

    // Overrides for this instance, indexed by the source id they target.
    let mut by_target: BTreeMap<NodeUid, &Override> = BTreeMap::new();
    for block in &src.overrides {
        if block.instance == instance_uid {
            by_target.insert(block.target, block);
        }
    }

    // An override whose target has vanished is preserved and warned about,
    // never dropped. Silently discarding a designer's work because a prefab was
    // mid-edit is not a trade worth making.
    for target in by_target.keys() {
        if source.by_uid(*target).is_none() {
            diags.push(
                Diagnostic::new(
                    Code::ORPHANED_OVERRIDE,
                    format!(
                        "override targets {target}, which {} no longer contains; \
                         it is kept in the file and ignored at runtime",
                        reference.to_text()
                    ),
                )
                .with_node(src.path_of(src_id).unwrap_or_default())
                .with_field("instance", instance_uid.to_text())
                .with_field("target", target.to_text()),
            );
        }
    }

    stack.push(reference.to_text());
    // Resolve the source's own instances first, so nesting works and the
    // outermost overrides apply last.
    let (flat_source, inner_diags) = resolve_nested(&source, sources, registry, stack, depth + 1);
    diags.extend(inner_diags);
    stack.pop();

    let source_root = flat_source.root()?;
    let anchor = copy_instanced(
        &flat_source,
        source_root,
        dst_parent,
        out,
        instance_uid,
        &by_target,
        diags,
        registry,
        true,
        src.get(src_id),
    );

    // The prefab's own connections and chunks come with it, with their ids
    // rewritten to the ones the instanced copy actually has. Without this a
    // prefab's signals would resolve to nodes that are not in the scene, and
    // they would fail silently — the worst way for a signal to fail.
    let remap = |uid: NodeUid| -> NodeUid {
        if flat_source
            .root()
            .and_then(|r| flat_source.get(r))
            .map(|n| n.uid)
            == Some(uid)
        {
            instance_uid
        } else {
            derive_uid(instance_uid, uid)
        }
    };
    for connection in &flat_source.connections {
        out.connections.push(crate::node::Connection {
            from: remap(connection.from),
            signal: connection.signal.clone(),
            to: remap(connection.to),
            method: connection.method.clone(),
        });
    }
    for chunk in &flat_source.chunks {
        let mut copy = chunk.clone();
        copy.layer = remap(chunk.layer);
        out.chunks.push(copy);
    }

    anchor
}

#[allow(clippy::too_many_arguments)]
fn copy_instanced(
    source: &Scene,
    source_id: NodeId,
    dst_parent: Option<NodeId>,
    out: &mut Scene,
    instance_uid: NodeUid,
    overrides: &BTreeMap<NodeUid, &Override>,
    diags: &mut Diagnostics,
    registry: &KindRegistry,
    is_root: bool,
    instance_node: Option<&Node>,
) -> Option<NodeId> {
    let node = source.get(source_id)?;
    if overrides.get(&node.uid).is_some_and(|b| b.removed) {
        return None;
    }

    let mut copy = node.clone();
    copy.inner_parent = None;
    // The substituted root keeps the instance's own id, so outer references to
    // the instance still resolve. Everything below it gets a derived id, so two
    // instances of one prefab cannot shadow each other.
    copy.uid = if is_root {
        instance_uid
    } else {
        derive_uid(instance_uid, node.uid)
    };

    // The instance node's own properties override the source root, which is
    // what makes `pos` on an instance mean what everyone expects it to mean.
    if is_root {
        if let Some(inst) = instance_node {
            copy.name = inst.name.clone();
            copy.transform = inst.transform;
            copy.visible = inst.visible;
            if inst.z != 0 {
                copy.z = inst.z;
            }
            if inst.layer != 0 {
                copy.layer = inst.layer;
            }
            if !inst.tags.is_empty() {
                copy.tags = inst.tags.clone();
            }
            if inst.script.is_some() {
                copy.script = inst.script.clone();
            }
            for (key, value) in &inst.props {
                apply_override(&mut copy, key, value, registry, diags);
            }
        }
    }

    if let Some(block) = overrides.get(&node.uid) {
        for (key, value) in &block.props {
            apply_override(&mut copy, key, value, registry, diags);
        }
    }

    let name = copy.name.clone();
    let new_id = match out.insert(copy, dst_parent) {
        Ok(id) => id,
        Err(d) => {
            diags.push(d.with_field("instanced_name", name));
            return None;
        }
    };

    for child in source.children(source_id).collect::<Vec<_>>() {
        copy_instanced(
            source,
            child,
            Some(new_id),
            out,
            instance_uid,
            overrides,
            diags,
            registry,
            false,
            None,
        );
    }
    Some(new_id)
}

/// Apply one override key to a node, coercing where the schema allows it.
fn apply_override(
    node: &mut Node,
    key: &str,
    value: &Value,
    registry: &KindRegistry,
    diags: &mut Diagnostics,
) {
    if is_reserved(key) {
        match key {
            "name" => {
                if let Some(s) = value.as_str() {
                    node.name = s.to_string();
                }
            }
            "pos" => {
                if let Some(v) = value.as_vec2() {
                    node.transform.pos = v;
                }
            }
            "scale" => {
                if let Some(v) = value.as_vec2() {
                    node.transform.scale = v;
                }
            }
            "rot" => {
                if let Some(a) = value.as_angle() {
                    node.transform.rot = a;
                } else if let Some(s) = value.as_scalar() {
                    node.transform.rot =
                        Angle::from_degrees_str(&s.to_exact_string()).unwrap_or(node.transform.rot);
                }
            }
            "visible" => {
                if let Some(b) = value.as_bool() {
                    node.visible = b;
                }
            }
            "z" => {
                if let Some(i) = value.as_int() {
                    node.z = i as i32;
                }
            }
            "layer" => {
                if let Some(i) = value.as_int() {
                    node.layer = i as i32;
                }
            }
            "script" => {
                if let Some(Reference::Script(_)) = value.as_ref_value() {
                    node.script = value.as_ref_value().cloned();
                }
            }
            _ => {}
        }
        return;
    }

    // Overrides are parsed without a schema, because the target's kind is only
    // known once the prefab is loaded. This is where the type finally gets
    // checked.
    let Some(schema) = registry.get(&node.kind) else {
        node.props.insert(key.to_string(), value.clone());
        return;
    };
    let Some(prop) = schema.property(key) else {
        diags.push(
            Diagnostic::new(
                Code::UNKNOWN_PROPERTY,
                format!("{} has no property {key:?} to override", node.kind),
            )
            .with_field("kind", node.kind.clone())
            .with_field("property", key.to_string()),
        );
        return;
    };

    let coerced = match (&prop.ty, value) {
        // A whole number written for a scalar property is unambiguous and
        // lossless, so it is widened rather than rejected. The reverse is not.
        (crate::schema::PropertyType::Scalar, Value::Int(i)) => i32::try_from(*i)
            .ok()
            .map(|i| Value::Scalar(dimetric_core::Fx::from_int(i))),
        (crate::schema::PropertyType::Enum(names), Value::Str(s)) if names.contains(s) => {
            Some(Value::Enum(s.clone()))
        }
        (crate::schema::PropertyType::Vec2, Value::Vec2i([x, y])) => {
            Some(Value::Vec2(Vec2Fx::from_ints(*x, *y)))
        }
        (crate::schema::PropertyType::Angle, Value::Scalar(s)) => {
            Angle::from_degrees_str(&s.to_exact_string())
                .ok()
                .map(Value::Angle)
        }
        _ => Some(value.clone()),
    };

    match coerced {
        Some(v) => {
            node.props.insert(key.to_string(), v);
        }
        None => diags.push(
            Diagnostic::new(
                Code::TYPE_MISMATCH,
                format!(
                    "override {key} is a {}, but {} expects a {}",
                    value.type_name(),
                    node.kind,
                    prop.ty.name()
                ),
            )
            .with_field("property", key.to_string()),
        ),
    }
}
