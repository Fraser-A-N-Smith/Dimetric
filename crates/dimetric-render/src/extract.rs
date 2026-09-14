//! Turning a scene into a frame's worth of draw calls.
//!
//! This is the whole of the renderer's contact with simulation state, and it is
//! strictly a read (I7). Nothing here writes a transform, and the interpolated
//! positions it produces are drawn and thrown away — they never go back into
//! anything physics will read.
//!
//! Kept separate from the GPU backend because it is the part worth testing.
//! Extraction can be checked on a machine with no graphics hardware at all,
//! which means the ordering and grouping are pinned before a driver is
//! involved.

use std::collections::BTreeMap;

use dimetric_core::{Angle, Fx, NodeUid, Vec2Fx};
use dimetric_scene::chunk::{CHUNK_SIZE, EMPTY_TILE};
use dimetric_scene::{Color, Scene, Value};

use crate::atlas::{Atlas, PLACEHOLDER_NAME};
use crate::batch::{Batch, Blend, DrawItem};
use crate::projection::Camera;
use crate::sort::SortKey;

/// A light to accumulate.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LightItem {
    /// World position.
    pub pos: Vec2Fx,
    /// Falloff radius, in world units.
    pub radius: Fx,
    /// Brightness multiplier.
    pub energy: Fx,
    /// Colour.
    pub color: Color,
    /// Direction the cone points. Ignored by radial lights.
    pub direction: Angle,
    /// Half-width of the cone. `None` for a radial light.
    pub cone: Option<Angle>,
}

/// Everything one frame needs to draw.
pub struct Frame {
    /// Sprites, already sorted.
    pub sprites: Vec<DrawItem>,
    /// Runs of sprites that can be issued as one instanced draw.
    pub batches: Vec<Batch>,
    /// Lights, in a defined order.
    pub lights: Vec<LightItem>,
    /// The view this frame was extracted for.
    pub camera: Camera,
}

impl Frame {
    /// How many draw calls this frame costs.
    pub fn draw_calls(&self) -> usize {
        self.batches.len()
    }
}

/// Interpolation between two simulation states.
///
/// Essential for a 60 Hz simulation on a 144 Hz display: without it every
/// display frame shows one of 60 positions and the motion visibly steps.
pub struct Interpolation<'a> {
    /// The state one tick ago.
    pub previous: &'a Scene,
    /// How far between the two states to draw, `0.0 ..= 1.0`.
    pub alpha: f32,
}

/// Walk a scene into a frame.
pub fn extract(
    scene: &Scene,
    atlas: &Atlas,
    camera: &Camera,
    interpolation: Option<Interpolation<'_>>,
) -> Frame {
    let mut sprites = Vec::new();
    let mut lights = Vec::new();

    // I3-exempt: alpha arrives from the host's accumulator as a float, and this
    // is the render boundary. It is converted once, here, and the interpolation
    // itself happens in fixed point.
    let alpha = interpolation
        .as_ref()
        .map(|i| Fx::from_f64_lossy(i.alpha.clamp(0.0, 1.0) as f64))
        .unwrap_or(Fx::ZERO);
    let previous = interpolation.as_ref().map(|i| i.previous);

    // Depth-first, so the order sprites are produced in depends on the tree and
    // not on allocation. The sort key decides draw order, but a stable
    // extraction order keeps the tie-break meaningful.
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        if !visible(scene, id) {
            continue;
        }
        let world = match scene.world_of(id) {
            Some(t) => t,
            None => continue,
        };
        let pos = interpolated(node.uid, world.pos, previous, alpha);

        match node.kind.as_str() {
            "Sprite2D" | "AnimatedSprite2D" => {
                if let Some(item) = sprite(node, pos, world.rot, camera, atlas) {
                    sprites.push(item);
                }
            }
            "TileLayer" => tiles(scene, node, pos, camera, atlas, &mut sprites),
            "Light2D" => {
                if let Some(light) = light(node, pos) {
                    lights.push(light);
                }
            }
            _ => {}
        }
    }

    let batches = crate::batch::build(&mut sprites);
    // Sorted by position and then by colour so the light list is reproducible;
    // additive blending is commutative, but a stable order keeps a golden image
    // stable too.
    lights.sort_by_key(|l| {
        (
            l.pos.x.to_raw(),
            l.pos.y.to_raw(),
            l.radius.to_raw(),
            (l.color.r, l.color.g, l.color.b, l.color.a),
        )
    });

    Frame {
        sprites,
        batches,
        lights,
        camera: *camera,
    }
}

/// A node is drawn only if it and every ancestor is visible.
fn visible(scene: &Scene, id: dimetric_core::NodeId) -> bool {
    let mut cursor = Some(id);
    while let Some(current) = cursor {
        let Some(node) = scene.get(current) else {
            return false;
        };
        if !node.visible {
            return false;
        }
        cursor = node.parent();
    }
    true
}

/// Blend this tick's position with last tick's.
fn interpolated(uid: NodeUid, current: Vec2Fx, previous: Option<&Scene>, alpha: Fx) -> Vec2Fx {
    let Some(previous) = previous else {
        return current;
    };
    // A node that did not exist last tick has nothing to blend from, and
    // sliding it in from a position it never occupied would look worse than
    // popping it in where it actually is.
    let Some(before) = previous
        .by_uid(uid)
        .and_then(|id| previous.world_of(id))
        .map(|t| t.pos)
    else {
        return current;
    };
    before.lerp(current, alpha)
}

/// Build a sprite from a `Sprite2D` or `AnimatedSprite2D`.
fn sprite(
    node: &dimetric_scene::Node,
    pos: Vec2Fx,
    rotation: Angle,
    camera: &Camera,
    atlas: &Atlas,
) -> Option<DrawItem> {
    let key = node
        .get("texture")
        .or_else(|| node.get("frames"))
        .and_then(Value::as_ref_value)
        .map(|r| r.target().to_string())
        .unwrap_or_default();

    // A missing texture draws the placeholder rather than nothing. A sprite
    // that silently fails to appear is far harder to diagnose than a magenta
    // checkerboard.
    let mut region = atlas
        .region(&key)
        .or_else(|| atlas.region(PLACEHOLDER_NAME))?;

    // An animation sheet is one image holding its frames side by side, so the
    // frame showing now is a slice of it. The index comes off the node, which
    // the simulation wrote during its own tick: the renderer never asks the
    // simulation anything (I7).
    let frames = atlas.frames(&key);
    if frames > 1 {
        let width = region.size.0 / frames;
        let index = node
            .get("frame")
            .and_then(Value::as_int)
            .unwrap_or(0)
            .clamp(0, frames as i64 - 1) as u32;
        region = region.sub(index * width, 0, width, region.size.1);
    }

    if let Some(Value::Rect(r)) = node.get("region") {
        region = region.sub(
            r.pos.x.to_int_trunc().max(0) as u32,
            r.pos.y.to_int_trunc().max(0) as u32,
            r.size.x.to_int_trunc().max(0) as u32,
            r.size.y.to_int_trunc().max(0) as u32,
        );
    }

    let mut uv = region.uv;
    if node.get("flip_h").and_then(Value::as_bool).unwrap_or(false) {
        uv.swap(0, 2);
    }
    if node.get("flip_v").and_then(Value::as_bool).unwrap_or(false) {
        uv.swap(1, 3);
    }

    let offset = node
        .get("offset")
        .and_then(Value::as_vec2)
        .unwrap_or(Vec2Fx::ZERO);
    let center = pos + offset;
    let size = Vec2Fx::from_ints(region.size.0 as i32, region.size.1 as i32);
    let blend = node
        .get("blend")
        .and_then(Value::as_str)
        .and_then(Blend::parse)
        .unwrap_or_default();
    let modulate = node
        .get("modulate")
        .and_then(Value::as_color)
        .unwrap_or(Color::WHITE);

    Some(DrawItem {
        key: SortKey::new(node.layer, camera.projection.depth_of(center), 0, node.uid),
        atlas: 0,
        blend,
        shader: 0,
        pos: center,
        size,
        rotation,
        uv,
        modulate: [modulate.r, modulate.g, modulate.b, modulate.a],
        node: node.uid,
    })
}

/// Expand a tile layer's chunks into sprites.
///
/// One sprite per non-empty cell. The design document calls for chunks baked
/// into static vertex buffers and invalidated per chunk on edit; that caching
/// lives in the GPU backend, which is the layer that owns buffers. What is here
/// is the bake itself, kept pure so it can be tested.
fn tiles(
    scene: &Scene,
    node: &dimetric_scene::Node,
    origin: Vec2Fx,
    camera: &Camera,
    atlas: &Atlas,
    out: &mut Vec<DrawItem>,
) {
    let cell = node
        .get("cell")
        .and_then(Value::as_vec2i)
        .unwrap_or([16, 16]);
    if cell[0] <= 0 || cell[1] <= 0 {
        return;
    }
    let tileset = node
        .get("tileset")
        .and_then(Value::as_ref_value)
        .map(|r| r.target().to_string())
        .unwrap_or_default();
    let Some(sheet) = atlas
        .region(&tileset)
        .or_else(|| atlas.region(PLACEHOLDER_NAME))
    else {
        return;
    };
    let columns = (sheet.size.0 / cell[0] as u32).max(1);
    let modulate = node
        .get("modulate")
        .and_then(Value::as_color)
        .unwrap_or(Color::WHITE);
    let size = Vec2Fx::from_ints(cell[0], cell[1]);
    let half = size / 2;

    for chunk in scene.chunks.iter().filter(|c| c.layer == node.uid) {
        let Some(cells) = chunk.cells() else { continue };
        for (index, tile) in cells.iter().enumerate() {
            if *tile == EMPTY_TILE {
                continue;
            }
            let local_x = index as i32 % CHUNK_SIZE;
            let local_y = index as i32 / CHUNK_SIZE;
            let tile_x = chunk.at[0] * CHUNK_SIZE + local_x;
            let tile_y = chunk.at[1] * CHUNK_SIZE + local_y;

            // Tile 0 means empty, so index 1 is the first tile in the sheet.
            let sheet_index = *tile as u32 - 1;
            let uv = sheet
                .sub(
                    (sheet_index % columns) * cell[0] as u32,
                    (sheet_index / columns) * cell[1] as u32,
                    cell[0] as u32,
                    cell[1] as u32,
                )
                .uv;

            let center = origin + Vec2Fx::from_ints(tile_x * cell[0], tile_y * cell[1]) + half;
            out.push(DrawItem {
                key: SortKey::new(node.layer, camera.projection.depth_of(center), 0, node.uid),
                atlas: 0,
                blend: Blend::Alpha,
                shader: 0,
                pos: center,
                size,
                rotation: Angle::ZERO,
                uv,
                modulate: [modulate.r, modulate.g, modulate.b, modulate.a],
                node: node.uid,
            });
        }
    }
}

/// Build a light from a `Light2D`.
fn light(node: &dimetric_scene::Node, pos: Vec2Fx) -> Option<LightItem> {
    let radius = node.get("radius").and_then(Value::as_scalar)?;
    if radius <= Fx::ZERO {
        return None;
    }
    let cone = match node.get("shape").and_then(Value::as_str) {
        Some("Cone") => node.get("cone_angle").and_then(Value::as_angle),
        _ => None,
    };
    Some(LightItem {
        pos,
        radius,
        energy: node
            .get("energy")
            .and_then(Value::as_scalar)
            .unwrap_or(Fx::ONE),
        color: node
            .get("color")
            .and_then(Value::as_color)
            .unwrap_or(Color::WHITE),
        direction: node
            .get("rot")
            .and_then(Value::as_angle)
            .unwrap_or(Angle::ZERO),
        cone,
    })
}

/// Every asset a scene refers to, in name order.
///
/// Used to decide what the atlas has to contain before a frame can be drawn.
pub fn required_assets(scene: &Scene) -> Vec<String> {
    let mut found: BTreeMap<String, ()> = BTreeMap::new();
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        for key in ["texture", "frames", "tileset"] {
            if let Some(reference) = node.get(key).and_then(Value::as_ref_value) {
                found.insert(reference.target().to_string(), ());
            }
        }
    }
    found.into_keys().collect()
}
