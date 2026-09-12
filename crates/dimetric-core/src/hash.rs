//! Deterministic state hashing.
//!
//! Every tick produces a hash of complete simulation state. Replay compares
//! those hashes tick by tick, which is what turns "the replay broke" into
//! "state diverged at tick 4117, in `/Arena01/Skeleton_03.pos`".
//!
//! Two rules make a hash trustworthy:
//!
//! 1. **Feed a length or a tag before variable-length data.** Otherwise
//!    `["ab", "c"]` and `["a", "bc"]` hash identically, and a divergence hides.
//! 2. **Feed in a defined order.** Any caller walking a map must sort first
//!    (I4). [`StateHasher`] cannot enforce this, so it is on the caller.

use crate::angle::Angle;
use crate::fx::{Fx, FxWide};
use crate::id::{AssetId, NodeUid};
use crate::rng::RngState;
use crate::vec::Vec2Fx;

/// A 32-byte state hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateHash([u8; 32]);

impl StateHash {
    /// The raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex, the form that appears in input logs and CI output.
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// The first eight hex characters, for log lines where the full hash is
    /// noise.
    pub fn short(self) -> String {
        self.to_hex()[..8].to_string()
    }

    /// Parse from hex.
    pub fn from_hex(s: &str) -> Option<StateHash> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(StateHash(out))
    }
}

impl core::fmt::Display for StateHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.pad(&self.to_hex())
    }
}
impl core::fmt::Debug for StateHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "StateHash({})", self.short())
    }
}
impl serde::Serialize for StateHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}
impl<'de> serde::Deserialize<'de> for StateHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<StateHash, D::Error> {
        let s = String::deserialize(d)?;
        StateHash::from_hex(&s)
            .ok_or_else(|| serde::de::Error::custom("expected 64 hex characters"))
    }
}

/// Accumulates simulation state into a [`StateHash`].
///
/// Only simulation state may be fed in. Presentation state — audio playback
/// position, cosmetic tweens, particles, render interpolation — must never
/// reach a hasher, or replay starts depending on frame timing and audio device
/// latency (I7).
#[derive(Clone, Default)]
pub struct StateHasher {
    inner: blake3::Hasher,
}

impl StateHasher {
    /// A fresh hasher.
    pub fn new() -> StateHasher {
        StateHasher {
            inner: blake3::Hasher::new(),
        }
    }

    /// Finish and produce the hash.
    pub fn finish(&self) -> StateHash {
        StateHash(*self.inner.finalize().as_bytes())
    }

    /// Feed a field name, so that reordering or renaming a field changes the
    /// hash rather than silently aliasing with another field.
    pub fn tag(&mut self, name: &str) -> &mut Self {
        self.inner.update(&(name.len() as u32).to_le_bytes());
        self.inner.update(name.as_bytes());
        self
    }

    /// Feed a length prefix before a variable-length sequence.
    pub fn len(&mut self, n: usize) -> &mut Self {
        self.inner.update(&(n as u64).to_le_bytes());
        self
    }

    /// Feed raw bytes.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.inner.update(&(b.len() as u64).to_le_bytes());
        self.inner.update(b);
        self
    }

    /// Feed a string.
    pub fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }

    /// Feed a boolean.
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.inner.update(&[v as u8]);
        self
    }

    /// Feed a signed integer.
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.inner.update(&v.to_le_bytes());
        self
    }

    /// Feed an unsigned integer.
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.inner.update(&v.to_le_bytes());
        self
    }

    /// Feed a 32-bit signed integer.
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.inner.update(&v.to_le_bytes());
        self
    }

    /// Feed a scalar, by its exact bits.
    pub fn fx(&mut self, v: Fx) -> &mut Self {
        self.i32(v.to_raw())
    }

    /// Feed a wide scalar.
    pub fn fx_wide(&mut self, v: FxWide) -> &mut Self {
        self.i64(v.to_raw())
    }

    /// Feed a vector.
    pub fn vec2(&mut self, v: Vec2Fx) -> &mut Self {
        self.fx(v.x).fx(v.y)
    }

    /// Feed an angle.
    pub fn angle(&mut self, v: Angle) -> &mut Self {
        self.inner.update(&v.to_bam().to_le_bytes());
        self
    }

    /// Feed a node uid.
    pub fn node_uid(&mut self, v: NodeUid) -> &mut Self {
        self.str(v.body())
    }

    /// Feed an asset id.
    pub fn asset_id(&mut self, v: AssetId) -> &mut Self {
        self.str(v.body())
    }

    /// Feed a generator's state.
    pub fn rng(&mut self, v: RngState) -> &mut Self {
        self.u64(v.state).u64(v.inc)
    }

    /// Feed another hash, for composing sub-hashes.
    pub fn hash(&mut self, v: StateHash) -> &mut Self {
        self.inner.update(&v.0);
        self
    }
}

/// Anything that can contribute to a state hash.
pub trait HashState {
    /// Feed `self` into `hasher`, in a fixed order.
    fn hash_state(&self, hasher: &mut StateHasher);

    /// Convenience: hash `self` on its own.
    fn state_hash(&self) -> StateHash
    where
        Self: Sized,
    {
        let mut h = StateHasher::new();
        self.hash_state(&mut h);
        h.finish()
    }
}
