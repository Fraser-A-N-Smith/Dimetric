//! Binary angles.
//!
//! An [`Angle`] is a 16-bit fraction of a turn: 0 is east, 16384 is a quarter
//! turn, and 65536 wraps back to zero for free because the type is `u16`. No
//! range reduction, no accumulated drift, no `-pi..pi` versus `0..2pi`
//! argument.
//!
//! Trigonometry reads committed lookup tables and interpolates in fixed point.
//! Calling the platform's `sin` instead would be faster to write and would
//! silently break replay across operating systems.

use core::fmt;
use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fx::{exact_decimal_pow2, split_decimal, Fx, FxParseError};
use crate::trig_table::{ATAN_UNIT, SIN_Q};

/// Binary angle units in a full turn.
pub const TURN: u32 = 65536;
/// Binary angle units in a quarter turn.
pub const QUARTER: u32 = TURN / 4;

/// Degrees per binary angle unit is `45 / 8192`, which is exact in decimal
/// because the denominator is a power of two.
const DEG_NUM: u128 = 45;
const DEG_DEN_BITS: u32 = 13; // 8192

/// A direction, stored as a 16-bit fraction of a turn.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Angle(u16);

impl Angle {
    /// East.
    pub const ZERO: Angle = Angle(0);
    /// North (a quarter turn counter-clockwise).
    pub const QUARTER_TURN: Angle = Angle(16384);
    /// West.
    pub const HALF_TURN: Angle = Angle(32768);

    /// Construct from raw binary angle units. Wraps.
    #[inline]
    pub const fn from_bam(bam: u16) -> Angle {
        Angle(bam)
    }

    /// The raw binary angle units.
    #[inline]
    pub const fn to_bam(self) -> u16 {
        self.0
    }

    /// Construct from degrees, rounding to the nearest representable angle.
    ///
    /// One binary unit is 360/65536 of a turn, so most whole degrees are not
    /// representable — 30° lands between two units. The rounding is exact
    /// integer arithmetic and therefore identical everywhere, and
    /// `dim scene fmt` rewrites the literal to the angle actually stored, so
    /// the loss shows up in a diff rather than hiding in a file.
    pub fn from_degrees_str(s: &str) -> Result<Angle, FxParseError> {
        let (neg, digits, places) = split_decimal(s)?;
        // bam = degrees * 8192 / 45, rounded half away from zero.
        let num = digits << DEG_DEN_BITS;
        let den = DEG_NUM * 10u128.pow(places);
        let bam = (2 * num + den) / (2 * den);
        let signed = if neg { -(bam as i128) } else { bam as i128 };
        Ok(Angle(signed.rem_euclid(TURN as i128) as u16))
    }

    /// Render as exact decimal degrees in `0.0 ..< 360.0`.
    pub fn to_degrees_string(self) -> String {
        exact_decimal_pow2(false, self.0 as u64 * DEG_NUM as u64, DEG_DEN_BITS)
    }

    /// Degrees as a float, for the render and authoring boundary only.
    ///
    /// I3-exempt: this is the render boundary.
    #[inline]
    pub fn to_degrees_f32(self) -> f32 {
        self.0 as f32 * (360.0 / 65536.0)
    }

    /// Sine, in `-1.0 ..= 1.0`.
    pub fn sin(self) -> Fx {
        let a = self.0 as u32;
        let quadrant = a >> 14;
        let rem = a & (QUARTER - 1);
        let magnitude = match quadrant {
            0 | 2 => sin_quarter(rem),
            _ => sin_quarter(QUARTER - rem),
        };
        if quadrant >= 2 {
            Fx::from_raw(-magnitude)
        } else {
            Fx::from_raw(magnitude)
        }
    }

    /// Cosine, in `-1.0 ..= 1.0`.
    #[inline]
    pub fn cos(self) -> Fx {
        (self + Angle(QUARTER as u16)).sin()
    }

    /// Sine and cosine together, which is the usual call site.
    #[inline]
    pub fn sin_cos(self) -> (Fx, Fx) {
        (self.sin(), self.cos())
    }

    /// The unit vector pointing along this angle.
    #[inline]
    pub fn to_unit_vector(self) -> crate::vec::Vec2Fx {
        crate::vec::Vec2Fx::new(self.cos(), self.sin())
    }

    /// The shortest signed difference to `other`, in binary units.
    ///
    /// Result is in `-32768 ..= 32767`: negative means `other` is clockwise.
    #[inline]
    pub fn delta_to(self, other: Angle) -> i32 {
        (other.0.wrapping_sub(self.0)) as i16 as i32
    }

    /// Step toward `other` by at most `max_step` binary units.
    pub fn rotate_toward(self, other: Angle, max_step: u16) -> Angle {
        let delta = self.delta_to(other);
        let step = delta.clamp(-(max_step as i32), max_step as i32);
        Angle(self.0.wrapping_add(step as i16 as u16))
    }

    /// The direction of the vector `(x, y)`, the fixed-point `atan2`.
    ///
    /// `atan2(0, 0)` is [`Angle::ZERO`] by convention, matching IEEE's choice
    /// for `atan2(+0, +0)` so that a zero-length aim vector points east rather
    /// than panicking mid-tick.
    pub fn from_vector(x: Fx, y: Fx) -> Angle {
        let (xr, yr) = (x.to_raw() as i64, y.to_raw() as i64);
        if xr == 0 && yr == 0 {
            return Angle::ZERO;
        }
        let (ax, ay) = (xr.unsigned_abs(), yr.unsigned_abs());
        // Reduce to the first octant, where the ratio is in [0, 1].
        let octant_angle = if ay <= ax {
            atan_unit_ratio(ay, ax)
        } else {
            QUARTER as i64 - atan_unit_ratio(ax, ay)
        };
        let bam = match (xr >= 0, yr >= 0) {
            (true, true) => octant_angle,
            (false, true) => TURN as i64 / 2 - octant_angle,
            (false, false) => TURN as i64 / 2 + octant_angle,
            (true, false) => TURN as i64 - octant_angle,
        };
        Angle(bam.rem_euclid(TURN as i64) as u16)
    }
}

/// Sine over a quarter turn, `rem` in `0 ..= 16384`, scaled by 65536.
fn sin_quarter(rem: u32) -> i32 {
    let index = (rem >> 4) as usize;
    if index >= SIN_Q.len() - 1 {
        return SIN_Q[SIN_Q.len() - 1];
    }
    let frac = (rem & 0xF) as i64;
    let lo = SIN_Q[index] as i64;
    let hi = SIN_Q[index + 1] as i64;
    (lo + (((hi - lo) * frac) >> 4)) as i32
}

/// `atan(small / large)` in binary units, for `0 <= small <= large`.
fn atan_unit_ratio(small: u64, large: u64) -> i64 {
    debug_assert!(small <= large && large > 0);
    // Ratio in 16.16, so 65536 means exactly 1.
    let ratio = ((small as u128) << 16) / large as u128;
    let index = (ratio >> 8) as usize;
    if index >= ATAN_UNIT.len() - 1 {
        return ATAN_UNIT[ATAN_UNIT.len() - 1] as i64;
    }
    let frac = (ratio & 0xFF) as i64;
    let lo = ATAN_UNIT[index] as i64;
    let hi = ATAN_UNIT[index + 1] as i64;
    lo + (((hi - lo) * frac) >> 8)
}

impl Add for Angle {
    type Output = Angle;
    #[inline]
    fn add(self, r: Angle) -> Angle {
        Angle(self.0.wrapping_add(r.0))
    }
}
impl Sub for Angle {
    type Output = Angle;
    #[inline]
    fn sub(self, r: Angle) -> Angle {
        Angle(self.0.wrapping_sub(r.0))
    }
}
impl Neg for Angle {
    type Output = Angle;
    #[inline]
    fn neg(self) -> Angle {
        Angle(self.0.wrapping_neg())
    }
}
impl AddAssign for Angle {
    #[inline]
    fn add_assign(&mut self, r: Angle) {
        *self = *self + r;
    }
}
impl SubAssign for Angle {
    #[inline]
    fn sub_assign(&mut self, r: Angle) {
        *self = *self - r;
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_degrees_string())
    }
}
impl fmt::Debug for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Angle({}° / {} bam)", self.to_degrees_string(), self.0)
    }
}
impl Serialize for Angle {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_degrees_string())
    }
}
impl<'de> Deserialize<'de> for Angle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Angle, D::Error> {
        let s = String::deserialize(d)?;
        Angle::from_degrees_str(&s).map_err(serde::de::Error::custom)
    }
}
