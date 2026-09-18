//! Draw ordering.
//!
//! One `u64` per sprite, packed so that a plain numeric sort produces the
//! correct draw order. Sorting integers is both faster than a comparator and,
//! more importantly, *total* — there is no pair of sprites whose relative order
//! depends on which one the sort happened to look at first.

use dimetric_core::{Fx, NodeUid};

/// Bits given to each field of a sort key.
pub const LAYER_BITS: u32 = 8;
/// Bits for the authored within-layer order.
pub const Z_BITS: u32 = 8;
/// Bits for the depth field.
///
/// Sixteen, which is exact rather than generous. Depth comes from
/// [`Projection::depth_of`](crate::Projection::depth_of), which returns an
/// `Fx`; `Fx` saturates at +/-32,768, so a depth outside that range cannot be
/// produced even by an isometric `x + y` where both terms are at the limit.
/// This field used to be 24 bits and eight of them could never be reached —
/// which is where `z` came from without anything else giving anything up.
pub const DEPTH_BITS: u32 = 16;
/// Bits for the batch-group field.
pub const GROUP_BITS: u32 = 16;
/// Bits for the tie-break field.
pub const TIE_BITS: u32 = 16;

/// A packed draw-order key.
///
/// Field order is layer, then `z`, then depth, then batch group, then a
/// tie-break. The two authored fields come first because an authored decision
/// must win: `layer` separates whole classes of node, `z` orders within one,
/// and a game that says a projectile draws over a corpse means it regardless of
/// which is further down the screen. The batch group sits before the tie-break
/// because two sprites at the same depth are in no meaningful order anyway, and
/// putting the ones that can be drawn together next to each other is what lets
/// the batcher emit few draw calls; a tie-break last so the order is never
/// ambiguous.
///
/// `z` was reserved, stored on every node, settable through the command bus,
/// visible in the inspector, readable by a probe — and read by nothing. The
/// example project sets it on five nodes, putting bolts over the player over
/// the enemies, and none of it did anything. It does now.
///
/// The group is [`batch_group`](crate::batch::batch_group) — whatever the
/// batcher splits on. It has to be exactly that: a key that sorted by something
/// the batcher ignores would produce a tidy order and the same number of draw
/// calls, which is the shape of an optimisation that does nothing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SortKey(pub u64);

impl SortKey {
    /// Build a key.
    ///
    /// `depth` is the world-space depth from
    /// [`Projection::depth_of`](crate::Projection::depth_of); it is biased into
    /// an unsigned range so that negative coordinates sort before positive
    /// ones instead of wrapping past them.
    pub fn new(layer: i32, z: i32, depth: Fx, group: u16, tie: NodeUid) -> SortKey {
        let layer = (layer.clamp(-128, 127) + 128) as u64 & mask(LAYER_BITS);
        let z = (z.clamp(-128, 127) + 128) as u64 & mask(Z_BITS);
        let depth = bias_depth(depth);
        let group = group as u64 & mask(GROUP_BITS);
        let tie = tie_break(tie);
        SortKey(
            (layer << (Z_BITS + DEPTH_BITS + GROUP_BITS + TIE_BITS))
                | (z << (DEPTH_BITS + GROUP_BITS + TIE_BITS))
                | (depth << (GROUP_BITS + TIE_BITS))
                | (group << TIE_BITS)
                | tie,
        )
    }

    /// The layer this key sorts into.
    pub fn layer(self) -> i32 {
        ((self.0 >> (Z_BITS + DEPTH_BITS + GROUP_BITS + TIE_BITS)) & mask(LAYER_BITS)) as i32 - 128
    }

    /// The within-layer order this key sorts into.
    pub fn z(self) -> i32 {
        ((self.0 >> (DEPTH_BITS + GROUP_BITS + TIE_BITS)) & mask(Z_BITS)) as i32 - 128
    }

    /// The batch group this key sorts into.
    pub fn group(self) -> u16 {
        ((self.0 >> TIE_BITS) & mask(GROUP_BITS)) as u16
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
