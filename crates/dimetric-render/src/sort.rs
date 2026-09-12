//! Draw ordering.
//!
//! One `u64` per sprite, packed so that a plain numeric sort produces the
//! correct draw order. Sorting integers is both faster than a comparator and,
//! more importantly, *total* — there is no pair of sprites whose relative order
//! depends on which one the sort happened to look at first.

use dimetric_core::{Fx, NodeUid};

/// Bits given to each field of a sort key.
pub const LAYER_BITS: u32 = 8;
/// Bits for the depth field.
pub const DEPTH_BITS: u32 = 24;
/// Bits for the texture field.
pub const TEXTURE_BITS: u32 = 16;
/// Bits for the tie-break field.
pub const TIE_BITS: u32 = 16;

/// A packed draw-order key.
///
/// Field order is layer, then depth, then texture, then a tie-break. Layer
/// first because it is an authored decision and must win; texture before the
/// tie-break because grouping by texture is what lets the batcher emit few
/// draw calls; a tie-break last so the order is never ambiguous.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SortKey(pub u64);

impl SortKey {
    /// Build a key.
    ///
    /// `depth` is the world-space depth from
    /// [`Projection::depth_of`](crate::Projection::depth_of); it is biased into
    /// an unsigned range so that negative coordinates sort before positive
    /// ones instead of wrapping past them.
    pub fn new(layer: i32, depth: Fx, texture: u16, tie: NodeUid) -> SortKey {
        let layer = (layer.clamp(-128, 127) + 128) as u64 & mask(LAYER_BITS);
        let depth = bias_depth(depth);
        let texture = texture as u64 & mask(TEXTURE_BITS);
        let tie = tie_break(tie);
        SortKey(
            (layer << (DEPTH_BITS + TEXTURE_BITS + TIE_BITS))
                | (depth << (TEXTURE_BITS + TIE_BITS))
                | (texture << TIE_BITS)
                | tie,
        )
    }

    /// The layer this key sorts into.
    pub fn layer(self) -> i32 {
        ((self.0 >> (DEPTH_BITS + TEXTURE_BITS + TIE_BITS)) & mask(LAYER_BITS)) as i32 - 128
    }

    /// The texture this key sorts into.
    pub fn texture(self) -> u16 {
        ((self.0 >> TIE_BITS) & mask(TEXTURE_BITS)) as u16
    }
}

fn mask(bits: u32) -> u64 {
    (1u64 << bits) - 1
}

/// Map a signed depth onto the unsigned range the key packs.
///
/// Depth is quantised to whole world units. A sprite has to move a full unit
/// to change its sort position, which stops two nearly-coincident sprites from
/// flickering past each other as one drifts by a fraction of a pixel.
fn bias_depth(depth: Fx) -> u64 {
    const HALF: i64 = 1 << (DEPTH_BITS - 1);
    let units = depth.floor_int() as i64;
    (units + HALF).clamp(0, (1i64 << DEPTH_BITS) - 1) as u64
}

/// A stable tie-break from a node's permanent id.
///
/// The id rather than an index or an address: those depend on allocation, and
/// two sprites swapping draw order between runs is exactly the kind of
/// difference a golden-image test would catch and nobody could explain.
fn tie_break(uid: NodeUid) -> u64 {
    let bytes = uid.body().as_bytes();
    let mut acc = 0u64;
    for b in bytes {
        acc = acc.wrapping_mul(31).wrapping_add(*b as u64);
    }
    acc & mask(TIE_BITS)
}
