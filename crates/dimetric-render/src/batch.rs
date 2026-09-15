//! Sprite batching.
//!
//! One instanced draw per run of sprites sharing an atlas, a blend mode and a
//! shader. The cost of owning a 2D renderer is low, and this is most of it: a
//! batcher, a sort, a tilemap chunker and a light pass.

use dimetric_core::{NodeUid, Vec2Fx};

use crate::sort::SortKey;

/// How a sprite composites.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Blend {
    /// Standard alpha blending.
    #[default]
    Alpha,
    /// Added to what is underneath. Spell effects and lights.
    Additive,
    /// Multiplied with what is underneath.
    Multiply,
}

impl Blend {
    /// Parse the scene-file spelling.
    pub fn parse(name: &str) -> Option<Blend> {
        Some(match name {
            "Alpha" => Blend::Alpha,
            "Additive" => Blend::Additive,
            "Multiply" => Blend::Multiply,
            _ => return None,
        })
    }
}

/// One sprite to draw.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawItem {
    /// Where it sorts.
    pub key: SortKey,
    /// Which atlas it samples.
    pub atlas: u16,
    /// How it composites.
    pub blend: Blend,
    /// Which shader draws it.
    pub shader: u16,
    /// World position of the sprite's centre.
    pub pos: Vec2Fx,
    /// Size in world units.
    pub size: Vec2Fx,
    /// Rotation about the centre.
    pub rotation: dimetric_core::Angle,
    /// Sub-rectangle of the atlas, `[u_min, v_min, u_max, v_max]`.
    pub uv: [f32; 4],
    /// Tint, as RGBA bytes.
    pub modulate: [u8; 4],
    /// Node it came from, for picking and for debugging.
    pub node: NodeUid,
}

/// What the batcher splits on, packed for the sort key.
///
/// The batcher merges adjacent items that agree on atlas, blend and shader, so
/// the sort has to put items that agree on all three next to each other. This
/// packing is the one place that correspondence is written down; if a fourth
/// thing ever splits a batch, it goes here and the sort follows for free.
pub fn batch_group(atlas: u16, shader: u16, blend: Blend) -> u16 {
    (atlas << 8) | ((shader & 0xf) << 4) | (blend as u16 & 0xf)
}

/// A contiguous run of draw items that can be issued as one instanced draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Batch {
    /// Atlas the run samples.
    pub atlas: u16,
    /// Blend mode.
    pub blend: Blend,
    /// Shader.
    pub shader: u16,
    /// Index of the first item.
    pub start: usize,
    /// How many items.
    pub count: usize,
}

/// Sort draw items and group them into batches.
///
/// The sort comes first and the grouping second, never the other way around:
/// grouping by texture before sorting would let a batch draw on top of
/// something it should be behind. Batching only ever merges *adjacent* items,
/// so it can never change the order the sort decided on.
pub fn build(items: &mut [DrawItem]) -> Vec<Batch> {
    items.sort_unstable_by_key(|i| i.key);

    let mut batches: Vec<Batch> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match batches.last_mut() {
            Some(batch)
                if batch.atlas == item.atlas
                    && batch.blend == item.blend
                    && batch.shader == item.shader =>
            {
                batch.count += 1;
            }
            _ => batches.push(Batch {
                atlas: item.atlas,
                blend: item.blend,
                shader: item.shader,
                start: index,
                count: 1,
            }),
        }
    }
    batches
}
