//! Fixed-point scalars.
//!
//! [`Fx`] is the simulation's only scalar type. It wraps [`fixed::types::I32F16`]
//! — ±32768 world units at a resolution of 1/65536 — and is deliberately the
//! *narrow* type. [`FxWide`] is the accumulator: same resolution, range
//! ±1.4×10¹⁴, wide enough to hold the square of anything an `Fx` can represent.
//!
//! # Why two types
//!
//! `Fx`'s weakness is not world size, it's intermediate overflow. Two points
//! 1000 units apart have a squared distance of 1,000,000, which leaves `Fx`
//! range immediately. Rather than widening the world, operations that can
//! exceed `Fx` range — `dot`, `cross`, `length_squared`, areas, running sums —
//! return `FxWide`. Narrowing back is explicit and checked, so the compiler
//! forces the author to decide what happens on overflow.
//!
//! # Why saturating
//!
//! The `fixed` crate inherits Rust's integer semantics: panic on overflow in
//! debug, wrap in release. Two profiles computing different arithmetic would
//! make replays depend on the build, so every operator here delegates to
//! *saturating* operations and produces identical results in every profile.
//!
//! In debug builds a saturation additionally trips an assertion, so the bug
//! surfaces during testing instead of sitting in a release binary. Code that
//! expects to saturate should call the explicit `saturating_*` methods, which
//! never assert; code that wants to handle the case itself should call
//! `checked_*`.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, Sub, SubAssign};
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Backing representation of [`Fx`]: 16 integer bits, 16 fractional bits,
/// so ±32768 at a resolution of 1/65536.
///
/// Note the naming. The `fixed` crate counts *integer* bits, not total width,
/// so a 32-bit type with 16 fractional bits is `I16F16` — the design document
/// calls it `I32F16`, which would be a 48-bit type. The ranges quoted there
/// (±32768, and ±1.4×10¹⁴ for the accumulator) are the authority, and these
/// are the types that match them.
///
/// Widening the world later is a change to this alias and to [`FxWideRepr`],
/// not to call sites.
pub type FxRepr = fixed::types::I16F16;
/// Backing representation of [`FxWide`]: 48 integer bits, 16 fractional bits,
/// so ±1.4×10¹⁴ at the same resolution as [`Fx`].
pub type FxWideRepr = fixed::types::I48F16;

/// Number of fractional bits in both [`Fx`] and [`FxWide`].
pub const FRAC_BITS: u32 = 16;
/// Raw value of `1.0`.
pub const ONE_RAW: i32 = 1 << FRAC_BITS;

/// Narrowing a [`FxWide`] that does not fit in [`Fx`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("value {value} does not fit in Fx (range {min} ..= {max})")]
pub struct Overflow {
    /// The value that could not be narrowed, rendered exactly.
    pub value: String,
    /// `Fx::MIN`, rendered exactly.
    pub min: &'static str,
    /// `Fx::MAX`, rendered exactly.
    pub max: &'static str,
}

// ---------------------------------------------------------------------------
// Fx
// ---------------------------------------------------------------------------

/// A fixed-point scalar: 16 integer bits, 16 fractional bits, saturating.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Fx(FxRepr);

impl Fx {
    /// Zero.
    pub const ZERO: Fx = Fx(FxRepr::from_bits(0));
    /// One.
    pub const ONE: Fx = Fx(FxRepr::from_bits(ONE_RAW));
    /// Negative one.
    pub const NEG_ONE: Fx = Fx(FxRepr::from_bits(-ONE_RAW));
    /// One half.
    pub const HALF: Fx = Fx(FxRepr::from_bits(ONE_RAW / 2));
    /// The smallest representable positive value, 1/65536.
    pub const DELTA: Fx = Fx(FxRepr::from_bits(1));
    /// The most negative representable value.
    pub const MIN: Fx = Fx(FxRepr::from_bits(i32::MIN));
    /// The largest representable value.
    pub const MAX: Fx = Fx(FxRepr::from_bits(i32::MAX));

    /// Construct from the raw backing integer (`value * 65536`).
    #[inline]
    pub const fn from_raw(raw: i32) -> Self {
        Fx(FxRepr::from_bits(raw))
    }

    /// The raw backing integer.
    #[inline]
    pub const fn to_raw(self) -> i32 {
        self.0.to_bits()
    }

    /// Construct from a whole number, saturating if it is out of range.
    #[inline]
    pub fn from_int(v: i32) -> Self {
        match v.checked_mul(ONE_RAW) {
            Some(raw) => Fx::from_raw(raw),
            None if v.is_negative() => Fx::MIN,
            None => Fx::MAX,
        }
    }

    /// Truncate toward zero to a whole number.
    #[inline]
    pub const fn to_int_trunc(self) -> i32 {
        self.to_raw() / ONE_RAW
    }

    /// Largest whole number less than or equal to `self`.
    #[inline]
    pub const fn floor_int(self) -> i32 {
        self.to_raw() >> FRAC_BITS
    }

    /// Smallest whole number greater than or equal to `self`.
    #[inline]
    pub fn ceil_int(self) -> i32 {
        (((self.to_raw() as i64) + (ONE_RAW as i64 - 1)) >> FRAC_BITS) as i32
    }

    /// Round half away from zero to a whole number.
    ///
    /// Half-away-from-zero rather than banker's rounding, because it is the
    /// rule a designer expects and it is symmetric about zero — banker's
    /// rounding is not, and asymmetric rounding shows up as drift in
    /// long-running simulations.
    #[inline]
    pub fn round_int(self) -> i32 {
        let raw = self.to_raw() as i64;
        let half = (ONE_RAW / 2) as i64;
        let rounded = if raw >= 0 {
            (raw + half) >> FRAC_BITS
        } else {
            -((-raw + half) >> FRAC_BITS)
        };
        rounded as i32
    }

    /// `self` with its integer part kept and fraction discarded, toward zero.
    #[inline]
    pub fn trunc(self) -> Fx {
        Fx::from_raw(self.to_int_trunc().saturating_mul(ONE_RAW))
    }

    /// Fractional part, carrying the sign of `self`.
    #[inline]
    pub fn fract(self) -> Fx {
        Fx::from_raw(self.to_raw() % ONE_RAW)
    }

    /// Absolute value, saturating at [`Fx::MAX`] for [`Fx::MIN`].
    #[inline]
    pub fn abs(self) -> Fx {
        Fx(self.0.saturating_abs())
    }

    /// `-1`, `0` or `1`.
    #[inline]
    pub fn signum(self) -> Fx {
        match self.to_raw().cmp(&0) {
            core::cmp::Ordering::Less => Fx::NEG_ONE,
            core::cmp::Ordering::Equal => Fx::ZERO,
            core::cmp::Ordering::Greater => Fx::ONE,
        }
    }

    /// True when the value is exactly zero.
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.to_raw() == 0
    }

    /// The smaller of two values.
    #[inline]
    pub fn min(self, other: Fx) -> Fx {
        if self.0 <= other.0 {
            self
        } else {
            other
        }
    }

    /// The larger of two values.
    #[inline]
    pub fn max(self, other: Fx) -> Fx {
        if self.0 >= other.0 {
            self
        } else {
            other
        }
    }

    /// Clamp into `lo ..= hi`.
    ///
    /// # Panics
    /// If `lo > hi`.
    #[inline]
    pub fn clamp(self, lo: Fx, hi: Fx) -> Fx {
        assert!(lo <= hi, "Fx::clamp called with lo > hi");
        self.max(lo).min(hi)
    }

    /// Linear interpolation. `t` is not clamped.
    #[inline]
    pub fn lerp(self, to: Fx, t: Fx) -> Fx {
        self + (to - self) * t
    }

    /// Widen to the accumulator type. Always exact.
    #[inline]
    pub const fn wide(self) -> FxWide {
        FxWide::from_raw(self.to_raw() as i64)
    }

    /// Square root, rounded toward zero. Negative inputs return zero.
    #[inline]
    pub fn sqrt(self) -> Fx {
        self.wide().sqrt()
    }

    /// Reciprocal. Division by zero saturates and asserts in debug builds.
    #[inline]
    pub fn recip(self) -> Fx {
        Fx::ONE / self
    }

    // -- explicit overflow handling ---------------------------------------

    /// Addition, `None` on overflow.
    #[inline]
    pub fn checked_add(self, rhs: Fx) -> Option<Fx> {
        self.0.checked_add(rhs.0).map(Fx)
    }
    /// Subtraction, `None` on overflow.
    #[inline]
    pub fn checked_sub(self, rhs: Fx) -> Option<Fx> {
        self.0.checked_sub(rhs.0).map(Fx)
    }
    /// Multiplication, `None` on overflow.
    #[inline]
    pub fn checked_mul(self, rhs: Fx) -> Option<Fx> {
        self.0.checked_mul(rhs.0).map(Fx)
    }
    /// Division, `None` on overflow or division by zero.
    #[inline]
    pub fn checked_div(self, rhs: Fx) -> Option<Fx> {
        self.0.checked_div(rhs.0).map(Fx)
    }

    /// Addition, saturating without asserting.
    #[inline]
    pub fn saturating_add(self, rhs: Fx) -> Fx {
        Fx(self.0.saturating_add(rhs.0))
    }
    /// Subtraction, saturating without asserting.
    #[inline]
    pub fn saturating_sub(self, rhs: Fx) -> Fx {
        Fx(self.0.saturating_sub(rhs.0))
    }
    /// Multiplication, saturating without asserting.
    #[inline]
    pub fn saturating_mul(self, rhs: Fx) -> Fx {
        Fx(self.0.saturating_mul(rhs.0))
    }
    /// Division, saturating without asserting. Division by zero saturates
    /// toward the sign of the numerator, and by convention `0 / 0` is zero.
    #[inline]
    pub fn saturating_div(self, rhs: Fx) -> Fx {
        match self.0.checked_div(rhs.0) {
            Some(v) => Fx(v),
            None => saturate_div(self.to_raw() as i64, rhs.to_raw() as i64, Fx::MIN, Fx::MAX),
        }
    }

    // -- float boundary ----------------------------------------------------

    /// Convert to `f32` for the render or authoring boundary.
    ///
    /// Never call this from simulation code — invariant I3.
    ///
    /// I3-exempt: this is the render boundary.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.to_raw() as f32 / ONE_RAW as f32
    }

    /// Convert to `f64` for the render or authoring boundary.
    ///
    /// Never call this from simulation code — invariant I3.
    ///
    /// I3-exempt: this is the render boundary.
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.to_raw() as f64 / ONE_RAW as f64
    }

    /// Convert from `f64`, rounding to the nearest representable value.
    ///
    /// This is lossy by construction and exists only for asset import and
    /// tooling that reads float-typed third-party formats. Scene files parse
    /// through [`Fx::parse_exact`], which refuses to round.
    ///
    /// I3-exempt: this is the import and scripting boundary.
    pub fn from_f64_lossy(v: f64) -> Fx {
        if v.is_nan() {
            return Fx::ZERO;
        }
        let scaled = (v * ONE_RAW as f64).round();
        if scaled <= i32::MIN as f64 {
            Fx::MIN
        } else if scaled >= i32::MAX as f64 {
            Fx::MAX
        } else {
            Fx::from_raw(scaled as i32)
        }
    }

    // -- exact decimal text ------------------------------------------------

    /// Render exactly, always with at least one fractional digit.
    ///
    /// Every `Fx` has a terminating decimal expansion because its denominator
    /// is a power of two, so this never rounds and
    /// [`Fx::parse_exact`] recovers the identical value.
    ///
    /// The trailing `.0` on whole numbers is load-bearing: it is what keeps a
    /// scalar distinguishable from an integer in TOML.
    pub fn to_exact_string(self) -> String {
        let raw = self.to_raw();
        let mag = raw.unsigned_abs() as u64;
        format_exact(raw < 0, mag >> FRAC_BITS, (mag & 0xFFFF) as u32)
    }

    /// Parse a decimal that is exactly representable.
    ///
    /// Returns [`FxParseError::NotRepresentable`] rather than rounding.
    /// Silent rounding is precisely how replay determinism dies quietly.
    pub fn parse_exact(s: &str) -> Result<Fx, FxParseError> {
        let (neg, int, frac_raw) = parse_decimal(s)?;
        let total = int
            .checked_mul(ONE_RAW as u128)
            .and_then(|v| v.checked_add(frac_raw as u128))
            .ok_or(FxParseError::OutOfRange)?;
        let signed = if neg { -(total as i128) } else { total as i128 };
        if signed < i32::MIN as i128 || signed > i32::MAX as i128 {
            return Err(FxParseError::OutOfRange);
        }
        Ok(Fx::from_raw(signed as i32))
    }
}

/// Why a decimal string could not become an [`Fx`] or [`FxWide`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FxParseError {
    /// The string is not a plain decimal number.
    #[error("not a decimal number: {0:?}")]
    Malformed(String),
    /// Exponent notation, infinities and NaN are refused outright.
    #[error("{0} is not accepted in scene files; write the value out in full")]
    UnsupportedForm(&'static str),
    /// The value is a well-formed decimal, but not exactly representable.
    #[error("{0} is not exactly representable in fixed-point (the resolution is 1/65536)")]
    NotRepresentable(String),
    /// The value is exactly representable but outside the type's range.
    #[error("value is outside the representable range")]
    OutOfRange,
}

/// Split a decimal literal into `(negative, digits, decimal_places)` such that
/// the value is `digits / 10^decimal_places`.
///
/// Shared by every exact-decimal parser in the engine so that they agree on
/// what counts as well-formed.
pub fn split_decimal(s: &str) -> Result<(bool, u128, u32), FxParseError> {
    let raw = s.trim();
    if raw.is_empty() {
        return Err(FxParseError::Malformed(s.to_string()));
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains('e') {
        return Err(FxParseError::UnsupportedForm("exponent notation"));
    }
    if lower.contains("inf") || lower.contains("nan") {
        return Err(FxParseError::UnsupportedForm("inf/nan"));
    }
    let (neg, rest) = match raw.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    let rest: String = rest.chars().filter(|c| *c != '_').collect();
    let (int_str, frac_str) = match rest.split_once('.') {
        Some((i, f)) => (i, f.to_string()),
        None => (rest.as_str(), String::new()),
    };
    if int_str.is_empty() && frac_str.is_empty() {
        return Err(FxParseError::Malformed(s.to_string()));
    }
    if !int_str.bytes().all(|b| b.is_ascii_digit()) || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(FxParseError::Malformed(s.to_string()));
    }
    let frac_str = frac_str.trim_end_matches('0');
    if int_str.len() + frac_str.len() > 30 {
        return Err(FxParseError::OutOfRange);
    }
    let mut digits: u128 = if int_str.is_empty() {
        0
    } else {
        int_str.parse().map_err(|_| FxParseError::OutOfRange)?
    };
    for b in frac_str.bytes() {
        digits = digits
            .checked_mul(10)
            .and_then(|d| d.checked_add((b - b'0') as u128))
            .ok_or(FxParseError::OutOfRange)?;
    }
    Ok((neg, digits, frac_str.len() as u32))
}

/// Shared decimal parser. Returns `(negative, integer_part, raw_fraction)`
/// where `raw_fraction` is the numerator over 65536.
fn parse_decimal(s: &str) -> Result<(bool, u128, u32), FxParseError> {
    let raw = s.trim();
    if raw.is_empty() {
        return Err(FxParseError::Malformed(s.to_string()));
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains('e') {
        return Err(FxParseError::UnsupportedForm("exponent notation"));
    }
    if lower.contains("inf") || lower.contains("nan") {
        return Err(FxParseError::UnsupportedForm("inf/nan"));
    }

    let (neg, rest) = match raw.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    // TOML permits digit separators; they carry no value.
    let rest: String = rest.chars().filter(|c| *c != '_').collect();

    let (int_str, frac_str) = match rest.split_once('.') {
        Some((i, f)) => (i, f),
        None => (rest.as_str(), ""),
    };
    if int_str.is_empty() && frac_str.is_empty() {
        return Err(FxParseError::Malformed(s.to_string()));
    }
    if !int_str.bytes().all(|b| b.is_ascii_digit()) || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(FxParseError::Malformed(s.to_string()));
    }

    let int: u128 = if int_str.is_empty() {
        0
    } else {
        int_str.parse().map_err(|_| FxParseError::OutOfRange)?
    };

    let trimmed = frac_str.trim_end_matches('0');
    if trimmed.is_empty() {
        return Ok((neg, int, 0));
    }
    // A denominator of 2^16 needs at most 16 decimal places; more digits than
    // that cannot land on a representable value.
    if trimmed.len() > 16 {
        return Err(FxParseError::NotRepresentable(raw.to_string()));
    }
    let numerator: u128 = trimmed.parse().map_err(|_| FxParseError::OutOfRange)?;
    let denominator = 10u128.pow(trimmed.len() as u32);
    let scaled = numerator << FRAC_BITS;
    if scaled % denominator != 0 {
        return Err(FxParseError::NotRepresentable(raw.to_string()));
    }
    Ok((neg, int, (scaled / denominator) as u32))
}

fn format_exact(neg: bool, int_part: u64, frac_part: u32) -> String {
    exact_decimal_pow2(neg, (int_part << FRAC_BITS) | frac_part as u64, FRAC_BITS)
}

/// Render `mag / 2^frac_bits` as an exact decimal, with an explicit sign and
/// always at least one fractional digit.
///
/// Any power-of-two denominator has a terminating decimal expansion, because
/// `1/2^k` is `5^k/10^k`. That is the whole reason fixed-point survives a text
/// round trip where floats do not.
pub fn exact_decimal_pow2(neg: bool, mag: u64, frac_bits: u32) -> String {
    let int_part = mag >> frac_bits;
    let frac_part = mag & ((1u64 << frac_bits) - 1);

    let mut out = String::with_capacity(24);
    if neg && (int_part != 0 || frac_part != 0) {
        out.push('-');
    }
    out.push_str(&int_part.to_string());
    out.push('.');
    if frac_part == 0 {
        out.push('0');
        return out;
    }
    let scaled = frac_part as u128 * 5u128.pow(frac_bits);
    let digits = format!("{:0width$}", scaled, width = frac_bits as usize);
    out.push_str(digits.trim_end_matches('0'));
    out
}

fn saturate_div<T: Copy>(num: i64, den: i64, min: T, max: T) -> T {
    // Reached only when `checked_div` failed: either division by zero, or
    // MIN / -1. Both saturate toward the sign of the mathematical result.
    let negative = (num < 0) != (den < 0);
    if num == 0 || !negative {
        max
    } else {
        min
    }
}

#[inline]
fn guard32(v: Option<FxRepr>, fallback: FxRepr, op: &'static str) -> FxRepr {
    if v.is_none() {
        debug_assert!(
            false,
            "Fx::{op} saturated. This is a bug: see invariant I3. \
             Use FxWide for accumulators, or call the explicit saturating_/checked_ variant."
        );
    }
    v.unwrap_or(fallback)
}

impl Add for Fx {
    type Output = Fx;
    #[inline]
    fn add(self, rhs: Fx) -> Fx {
        Fx(guard32(
            self.0.checked_add(rhs.0),
            self.0.saturating_add(rhs.0),
            "add",
        ))
    }
}
impl Sub for Fx {
    type Output = Fx;
    #[inline]
    fn sub(self, rhs: Fx) -> Fx {
        Fx(guard32(
            self.0.checked_sub(rhs.0),
            self.0.saturating_sub(rhs.0),
            "sub",
        ))
    }
}
impl Mul for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, rhs: Fx) -> Fx {
        Fx(guard32(
            self.0.checked_mul(rhs.0),
            self.0.saturating_mul(rhs.0),
            "mul",
        ))
    }
}
impl Div for Fx {
    type Output = Fx;
    #[inline]
    fn div(self, rhs: Fx) -> Fx {
        match self.0.checked_div(rhs.0) {
            Some(v) => Fx(v),
            None => {
                debug_assert!(
                    !rhs.is_zero(),
                    "Fx::div by zero. This is a bug: see invariant I3."
                );
                debug_assert!(false, "Fx::div saturated. This is a bug: see invariant I3.");
                saturate_div(self.to_raw() as i64, rhs.to_raw() as i64, Fx::MIN, Fx::MAX)
            }
        }
    }
}
impl Rem for Fx {
    type Output = Fx;
    #[inline]
    fn rem(self, rhs: Fx) -> Fx {
        debug_assert!(!rhs.is_zero(), "Fx::rem by zero. See invariant I3.");
        if rhs.is_zero() {
            return Fx::ZERO;
        }
        Fx::from_raw(self.to_raw() % rhs.to_raw())
    }
}
impl Neg for Fx {
    type Output = Fx;
    #[inline]
    fn neg(self) -> Fx {
        Fx(guard32(
            self.0.checked_neg(),
            self.0.saturating_neg(),
            "neg",
        ))
    }
}

impl Mul<i32> for Fx {
    type Output = Fx;
    #[inline]
    fn mul(self, rhs: i32) -> Fx {
        Fx(guard32(
            self.0.checked_mul_int(rhs),
            self.0.saturating_mul_int(rhs),
            "mul_int",
        ))
    }
}
impl Div<i32> for Fx {
    type Output = Fx;
    #[inline]
    fn div(self, rhs: i32) -> Fx {
        debug_assert!(rhs != 0, "Fx::div by zero. See invariant I3.");
        if rhs == 0 {
            return if self.to_raw() < 0 { Fx::MIN } else { Fx::MAX };
        }
        Fx::from_raw(self.to_raw() / rhs)
    }
}

macro_rules! assign_ops {
    ($($tr:ident, $m:ident, $op:tt;)*) => {$(
        impl $tr for Fx {
            #[inline]
            fn $m(&mut self, rhs: Fx) { *self = *self $op rhs; }
        }
    )*};
}
assign_ops! {
    AddAssign, add_assign, +;
    SubAssign, sub_assign, -;
    MulAssign, mul_assign, *;
    DivAssign, div_assign, /;
}

impl Sum for Fx {
    fn sum<I: Iterator<Item = Fx>>(iter: I) -> Fx {
        iter.fold(Fx::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_exact_string())
    }
}
impl fmt::Debug for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fx({})", self.to_exact_string())
    }
}
impl FromStr for Fx {
    type Err = FxParseError;
    fn from_str(s: &str) -> Result<Fx, FxParseError> {
        Fx::parse_exact(s)
    }
}

impl From<i16> for Fx {
    fn from(v: i16) -> Fx {
        Fx::from_raw(v as i32 * ONE_RAW)
    }
}

impl Serialize for Fx {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Text, not bits: a snapshot or state dump must stay diffable (I2).
        s.serialize_str(&self.to_exact_string())
    }
}
impl<'de> Deserialize<'de> for Fx {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Fx, D::Error> {
        let s = String::deserialize(d)?;
        Fx::parse_exact(&s).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// FxWide
// ---------------------------------------------------------------------------

/// The accumulator: 48 integer bits, 16 fractional bits.
///
/// Holds the square of any [`Fx`] with room to spare. Prefer comparing squared
/// magnitudes here (`d.length_squared() < radius_sq`) over calling `length()`:
/// it avoids the square root entirely and is the common case in collision and
/// aggro checks.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct FxWide(FxWideRepr);

impl FxWide {
    /// Zero.
    pub const ZERO: FxWide = FxWide(FxWideRepr::from_bits(0));
    /// One.
    pub const ONE: FxWide = FxWide(FxWideRepr::from_bits(ONE_RAW as i64));
    /// The most negative representable value.
    pub const MIN: FxWide = FxWide(FxWideRepr::from_bits(i64::MIN));
    /// The largest representable value.
    pub const MAX: FxWide = FxWide(FxWideRepr::from_bits(i64::MAX));

    /// Construct from the raw backing integer (`value * 65536`).
    #[inline]
    pub const fn from_raw(raw: i64) -> Self {
        FxWide(FxWideRepr::from_bits(raw))
    }
    /// The raw backing integer.
    #[inline]
    pub const fn to_raw(self) -> i64 {
        self.0.to_bits()
    }
    /// Construct from a whole number.
    #[inline]
    pub fn from_int(v: i64) -> Self {
        FxWide(FxWideRepr::saturating_from_num(v))
    }

    /// Absolute value.
    #[inline]
    pub fn abs(self) -> FxWide {
        FxWide(self.0.saturating_abs())
    }
    /// True when the value is exactly zero.
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.to_raw() == 0
    }
    /// The smaller of two values.
    #[inline]
    pub fn min(self, o: FxWide) -> FxWide {
        if self.0 <= o.0 {
            self
        } else {
            o
        }
    }
    /// The larger of two values.
    #[inline]
    pub fn max(self, o: FxWide) -> FxWide {
        if self.0 >= o.0 {
            self
        } else {
            o
        }
    }

    /// Narrow back to [`Fx`], failing if the value does not fit.
    ///
    /// There is deliberately no `From<FxWide> for Fx`: narrowing is a decision
    /// about what happens on overflow, and the compiler should make the author
    /// take it.
    pub fn narrow(self) -> Result<Fx, Overflow> {
        let raw = self.to_raw();
        if raw < i32::MIN as i64 || raw > i32::MAX as i64 {
            Err(Overflow {
                value: self.to_exact_string(),
                min: "-32768.0",
                max: "32767.9999847412109375",
            })
        } else {
            Ok(Fx::from_raw(raw as i32))
        }
    }

    /// Narrow back to [`Fx`], clamping instead of failing.
    #[inline]
    pub fn narrow_saturating(self) -> Fx {
        Fx::from_raw(self.to_raw().clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }

    /// Square root, rounded toward zero. Negative inputs return zero.
    ///
    /// The result of `sqrt` on a squared magnitude always lands back in `Fx`
    /// range, which is why this returns the narrow type directly.
    pub fn sqrt(self) -> Fx {
        let raw = self.to_raw();
        if raw <= 0 {
            debug_assert!(
                raw >= 0,
                "FxWide::sqrt of a negative value. See invariant I3."
            );
            return Fx::ZERO;
        }
        // sqrt(raw / 2^16) = sqrt(raw * 2^16) / 2^16
        let scaled = (raw as u128) << FRAC_BITS;
        let root = isqrt_u128(scaled);
        Fx::from_raw(root.min(i32::MAX as u128) as i32)
    }

    /// Multiplication, `None` on overflow.
    #[inline]
    pub fn checked_mul(self, rhs: FxWide) -> Option<FxWide> {
        self.0.checked_mul(rhs.0).map(FxWide)
    }
    /// Addition, `None` on overflow.
    #[inline]
    pub fn checked_add(self, rhs: FxWide) -> Option<FxWide> {
        self.0.checked_add(rhs.0).map(FxWide)
    }

    /// Render exactly. See [`Fx::to_exact_string`].
    pub fn to_exact_string(self) -> String {
        let raw = self.to_raw();
        let mag = raw.unsigned_abs();
        format_exact(raw < 0, mag >> FRAC_BITS, (mag & 0xFFFF) as u32)
    }

    /// Parse a decimal that is exactly representable.
    pub fn parse_exact(s: &str) -> Result<FxWide, FxParseError> {
        let (neg, int, frac_raw) = parse_decimal(s)?;
        let total = int
            .checked_mul(ONE_RAW as u128)
            .and_then(|v| v.checked_add(frac_raw as u128))
            .ok_or(FxParseError::OutOfRange)?;
        let signed = if neg { -(total as i128) } else { total as i128 };
        if signed < i64::MIN as i128 || signed > i64::MAX as i128 {
            return Err(FxParseError::OutOfRange);
        }
        Ok(FxWide::from_raw(signed as i64))
    }
}

#[inline]
fn guard64(v: Option<FxWideRepr>, fallback: FxWideRepr, op: &'static str) -> FxWideRepr {
    if v.is_none() {
        debug_assert!(
            false,
            "FxWide::{op} saturated. This is a bug: see invariant I3."
        );
    }
    v.unwrap_or(fallback)
}

impl Add for FxWide {
    type Output = FxWide;
    #[inline]
    fn add(self, r: FxWide) -> FxWide {
        FxWide(guard64(
            self.0.checked_add(r.0),
            self.0.saturating_add(r.0),
            "add",
        ))
    }
}
impl Sub for FxWide {
    type Output = FxWide;
    #[inline]
    fn sub(self, r: FxWide) -> FxWide {
        FxWide(guard64(
            self.0.checked_sub(r.0),
            self.0.saturating_sub(r.0),
            "sub",
        ))
    }
}
impl Mul for FxWide {
    type Output = FxWide;
    #[inline]
    fn mul(self, r: FxWide) -> FxWide {
        FxWide(guard64(
            self.0.checked_mul(r.0),
            self.0.saturating_mul(r.0),
            "mul",
        ))
    }
}
impl Div for FxWide {
    type Output = FxWide;
    #[inline]
    fn div(self, r: FxWide) -> FxWide {
        match self.0.checked_div(r.0) {
            Some(v) => FxWide(v),
            None => {
                debug_assert!(false, "FxWide::div saturated or divided by zero. See I3.");
                saturate_div(self.to_raw(), r.to_raw(), FxWide::MIN, FxWide::MAX)
            }
        }
    }
}
impl Neg for FxWide {
    type Output = FxWide;
    #[inline]
    fn neg(self) -> FxWide {
        FxWide(guard64(
            self.0.checked_neg(),
            self.0.saturating_neg(),
            "neg",
        ))
    }
}
impl AddAssign for FxWide {
    #[inline]
    fn add_assign(&mut self, r: FxWide) {
        *self = *self + r;
    }
}
impl SubAssign for FxWide {
    #[inline]
    fn sub_assign(&mut self, r: FxWide) {
        *self = *self - r;
    }
}
impl Sum for FxWide {
    fn sum<I: Iterator<Item = FxWide>>(iter: I) -> FxWide {
        iter.fold(FxWide::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for FxWide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_exact_string())
    }
}
impl fmt::Debug for FxWide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FxWide({})", self.to_exact_string())
    }
}
impl FromStr for FxWide {
    type Err = FxParseError;
    fn from_str(s: &str) -> Result<FxWide, FxParseError> {
        FxWide::parse_exact(s)
    }
}
impl From<Fx> for FxWide {
    fn from(v: Fx) -> FxWide {
        v.wide()
    }
}
impl Serialize for FxWide {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_exact_string())
    }
}
impl<'de> Deserialize<'de> for FxWide {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<FxWide, D::Error> {
        let s = String::deserialize(d)?;
        FxWide::parse_exact(&s).map_err(serde::de::Error::custom)
    }
}

/// Integer square root by Newton iteration.
///
/// Owned rather than taken from `libm` on purpose: `sqrt` from a platform math
/// library is not guaranteed bit-identical across targets, and one differing
/// low bit is a diverged replay.
pub fn isqrt_u128(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Seed with a power of two at least as large as the true root, so the
    // iteration is monotonically decreasing and terminates.
    let bits = 128 - n.leading_zeros();
    let mut x = 1u128 << bits.div_ceil(2);
    loop {
        let next = (x + n / x) >> 1;
        if next >= x {
            break;
        }
        x = next;
    }
    x
}

/// Integer square root of a `u64`.
#[inline]
pub fn isqrt_u64(n: u64) -> u64 {
    isqrt_u128(n as u128) as u64
}
