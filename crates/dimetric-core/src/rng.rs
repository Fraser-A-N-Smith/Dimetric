//! Seeded randomness.
//!
//! Every random value in a simulation comes from one of these, seeded from the
//! run seed (I6). There is no ambient randomness anywhere in the engine, and
//! Lua's `math.random` is removed from the sandbox.
//!
//! Streams are named. Splitting randomness by purpose — spawn tables, damage
//! rolls, upgrade offers — means adding a new random call in one system does
//! not shift the numbers every other system sees, which is what makes a saved
//! replay survive a gameplay patch.

use serde::{Deserialize, Serialize};

use crate::fx::Fx;

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

/// A PCG32 generator.
///
/// PCG rather than a hash-based or xorshift generator because it is small,
/// has a well-understood period, and — the part that matters here — is defined
/// entirely in terms of wrapping integer arithmetic, so it produces identical
/// output on every platform without any floating point anywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    /// Create a generator from a seed and a stream selector.
    ///
    /// Two generators with the same seed and different streams produce
    /// unrelated sequences.
    pub fn new(seed: u64, stream: u64) -> Rng {
        let mut rng = Rng {
            state: 0,
            inc: (stream << 1) | 1,
        };
        rng.step();
        rng.state = rng.state.wrapping_add(seed);
        rng.step();
        rng
    }

    /// Create a generator for a named stream of a run seed.
    pub fn named(seed: u64, stream_name: &str) -> Rng {
        Rng::new(seed, stream_selector(stream_name))
    }

    #[inline]
    fn step(&mut self) {
        self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(self.inc);
    }

    /// The next 32 bits.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// The next 64 bits, as two draws.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | self.next_u32() as u64
    }

    /// A uniform value in `lo .. hi`, excluding `hi`.
    ///
    /// Rejection-sampled rather than a plain modulo, because modulo bias would
    /// make a 1-in-3 drop chance measurably not 1-in-3. The rejection loop is
    /// deterministic: the same state always rejects the same draws.
    ///
    /// # Panics
    /// If `lo >= hi`.
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        assert!(lo < hi, "Rng::range_i32 needs a non-empty range");
        let span = (hi as i64 - lo as i64) as u64 as u32;
        lo + self.below_u32(span) as i32
    }

    /// A uniform value in `0 .. bound`.
    ///
    /// # Panics
    /// If `bound` is zero.
    pub fn below_u32(&mut self, bound: u32) -> u32 {
        assert!(bound > 0, "Rng::below_u32 needs a non-zero bound");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % bound;
            }
        }
    }

    /// A uniform fixed-point value in `0.0 ..< 1.0`.
    #[inline]
    pub fn unit_fx(&mut self) -> Fx {
        Fx::from_raw((self.next_u32() >> 16) as i32)
    }

    /// A uniform fixed-point value in `lo ..< hi`.
    pub fn range_fx(&mut self, lo: Fx, hi: Fx) -> Fx {
        let span = (hi.wide() - lo.wide()).narrow_saturating();
        lo + span * self.unit_fx()
    }

    /// True with probability `numerator / denominator`.
    ///
    /// # Panics
    /// If `denominator` is zero.
    pub fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        self.below_u32(denominator) < numerator
    }

    /// A uniform direction.
    pub fn angle(&mut self) -> crate::angle::Angle {
        crate::angle::Angle::from_bam(self.next_u32() as u16)
    }

    /// Pick an index into a collection of `len` items.
    ///
    /// # Panics
    /// If `len` is zero.
    pub fn index(&mut self, len: usize) -> usize {
        assert!(len > 0, "Rng::index needs a non-empty collection");
        self.below_u32(len as u32) as usize
    }

    /// Shuffle in place, using the same Fisher-Yates order everywhere.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below_u32(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// Capture the generator's full state for a snapshot.
    #[inline]
    pub fn snapshot(self) -> RngState {
        RngState {
            state: self.state,
            inc: self.inc,
        }
    }

    /// Restore a generator from a snapshot.
    #[inline]
    pub fn restore(state: RngState) -> Rng {
        Rng {
            state: state.state,
            inc: state.inc,
        }
    }
}

/// A generator's complete state. Part of every sim snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RngState {
    /// The LCG state.
    pub state: u64,
    /// The stream increment, always odd.
    pub inc: u64,
}

/// Map a stream name to a stream selector.
///
/// Hashed rather than assigned by registration order, so a stream keeps its
/// sequence no matter when in the program it is first asked for.
pub fn stream_selector(name: &str) -> u64 {
    let digest = blake3::hash(name.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// The named generators belonging to one run.
///
/// Iteration is over a `BTreeMap`, not a `HashMap`, because anything that walks
/// this collection would otherwise pick up hash-seed ordering and break
/// invariant I4.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RngStreams {
    seed: u64,
    streams: std::collections::BTreeMap<String, Rng>,
}

impl RngStreams {
    /// Create an empty set of streams for a run seed.
    pub fn new(seed: u64) -> RngStreams {
        RngStreams {
            seed,
            streams: std::collections::BTreeMap::new(),
        }
    }

    /// The run seed these streams derive from.
    #[inline]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Borrow a named stream, creating it on first use.
    pub fn stream(&mut self, name: &str) -> &mut Rng {
        let seed = self.seed;
        self.streams
            .entry(name.to_string())
            .or_insert_with(|| Rng::named(seed, name))
    }

    /// Every live stream, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Rng)> {
        self.streams.iter().map(|(k, v)| (k.as_str(), v))
    }
}
