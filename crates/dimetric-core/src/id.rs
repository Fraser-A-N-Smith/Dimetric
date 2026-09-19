//! Stable identity.
//!
//! Three kinds of name exist in the engine and they are not interchangeable:
//!
//! - **[`NodeId`]** — a generational slotmap key. Fast, in-memory only, and
//!   never written to a file.
//! - **[`NodeUid`]** — the `n_k3xq7a2p` string that appears in a `.dim` file
//!   and in every cross-reference. Permanent: renaming, reparenting and moving
//!   a node never change it.
//! - **A path** — `/Arena01/Player/Sprite`, derived from names. Convenient for
//!   scripts and the CLI, and never stored as a reference, because names
//!   change.
//!
//! Uids are random rather than sequential, which looks worse and merges far
//! better. Two branches each appending a node with a counter both produce
//! `n_47`, and git resolves that into a file that is silently wrong. Two
//! branches each producing a random 40-bit id do not collide, and if they ever
//! did, the duplicate-id check (`DIM0102`) catches it loudly on load.
//!
//! A uid stores its eight characters rather than the bits behind them, so the
//! text that went into a scene file is the text that comes back out. Packing
//! into an integer would quietly normalise a hand-written `n_sk_stats`, and a
//! format that rewrites ids on load cannot claim byte-identical round trips
//! (I2).

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

slotmap::new_key_type! {
    /// In-memory handle to a node. Generational, so a handle to a deleted node
    /// never silently resolves to whatever was allocated in its place.
    pub struct NodeId;
}

/// Characters in the body of a uid.
pub const UID_LEN: usize = 8;
/// Bits of entropy in a generated uid.
pub const UID_BITS: u32 = 40;

/// Crockford-style base32, minus the characters that are easy to misread.
/// Generation draws from this; parsing is more permissive.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The alphabet generated ids are drawn from, for documentation.
///
/// Exposed so `docs/API.md` can state the format without a second copy of it
/// going stale. Anyone hand-writing or generating a `.meta` has to know this,
/// and until it was written down the only way to find out was to read this
/// file — which is how a generator came to emit `sprites_ashfen_bogling` and
/// have 84 sidecars overwritten.
pub fn generated_alphabet() -> &'static str {
    // Valid ASCII by construction.
    std::str::from_utf8(ALPHABET).expect("the alphabet is ASCII")
}

/// Every uid prefix the engine defines, with what it identifies.
pub const UID_PREFIXES: &[(&str, &str)] = &[
    ("n_", "a node, in a `.dim` scene"),
    ("a_", "an asset, in a `.meta` sidecar"),
];

/// Why a uid string was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UidError {
    /// The expected prefix was missing.
    #[error("expected a {expected:?} prefix on {found:?}")]
    BadPrefix {
        /// The prefix the type requires.
        expected: &'static str,
        /// What was actually supplied.
        found: String,
    },
    /// The body was not eight characters.
    #[error("expected {UID_LEN} characters after the prefix, found {0}")]
    BadLength(usize),
    /// The body contained something outside `[0-9a-z_]`.
    #[error("{0:?} is not a valid uid character; use [0-9a-z_]")]
    BadCharacter(char),
}

macro_rules! uid_type {
    ($name:ident, $prefix:literal, $what:literal) => {
        #[doc = concat!("A permanent ", $what, " identifier: `", $prefix, "` plus eight characters.")]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; UID_LEN]);

        impl $name {
            /// The textual prefix this id carries.
            pub const PREFIX: &'static str = $prefix;

            /// Draw a fresh id from a seeded stream.
            ///
            /// Ids come from the project's RNG rather than the operating
            /// system so that applying a recorded command log reproduces the
            /// same ids — otherwise undo, replay and agent transcripts would
            /// all drift.
            pub fn generate(rng: &mut crate::rng::Rng) -> Self {
                let bits = rng.next_u64();
                let mut body = [0u8; UID_LEN];
                for (i, slot) in body.iter_mut().enumerate() {
                    let shift = (UID_LEN - 1 - i) as u32 * 5;
                    *slot = ALPHABET[((bits >> shift) & 31) as usize];
                }
                $name(body)
            }

            /// The eight body characters, without the prefix.
            #[inline]
            pub fn body(&self) -> &str {
                core::str::from_utf8(&self.0).expect("uid bodies are validated ASCII")
            }

            /// Render as the canonical text form.
            pub fn to_text(self) -> String {
                let mut s = String::with_capacity($prefix.len() + UID_LEN);
                s.push_str($prefix);
                s.push_str(self.body());
                s
            }

            /// Parse the canonical text form.
            ///
            /// Parsing accepts any of `[0-9a-z_]`, which is wider than the
            /// generation alphabet, because people do hand-write ids in scene
            /// files and `n_sk_stats` reads better than random noise. Whatever
            /// is accepted is stored verbatim and written back unchanged.
            pub fn parse(s: &str) -> Result<Self, UidError> {
                let body = s.strip_prefix($prefix).ok_or_else(|| UidError::BadPrefix {
                    expected: $prefix,
                    found: s.to_string(),
                })?;
                if body.len() != UID_LEN || !body.is_ascii() {
                    return Err(UidError::BadLength(body.chars().count()));
                }
                for c in body.chars() {
                    if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                        return Err(UidError::BadCharacter(c));
                    }
                }
                let mut arr = [0u8; UID_LEN];
                arr.copy_from_slice(body.as_bytes());
                Ok($name(arr))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.pad(&self.to_text())
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.to_text())
            }
        }
        impl FromStr for $name {
            type Err = UidError;
            fn from_str(s: &str) -> Result<Self, UidError> {
                $name::parse(s)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_text())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

uid_type!(NodeUid, "n_", "node");
uid_type!(AssetId, "a_", "asset");
