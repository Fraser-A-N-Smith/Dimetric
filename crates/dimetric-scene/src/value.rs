//! Property values and the typed references that appear in scene files.

use core::fmt;

use dimetric_core::{Angle, Fx, Rect, Vec2Fx};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// An RGBA colour. Alpha is always explicit in text, because a three-channel
/// literal that silently means "opaque" is one more thing to remember.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha.
    pub a: u8,
}

impl Color {
    /// Opaque white.
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// Construct from components.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// Parse `#rrggbbaa`.
    pub fn parse(s: &str) -> Result<Color, ColorError> {
        let body = s.strip_prefix('#').ok_or(ColorError::MissingHash)?;
        if body.len() != 8 {
            return Err(ColorError::WrongLength(body.len()));
        }
        let byte = |i: usize| u8::from_str_radix(&body[i..i + 2], 16).map_err(|_| ColorError::NotHex);
        Ok(Color {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: byte(6)?,
        })
    }

    /// Render as `#rrggbbaa`, lowercase.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }

    /// Linear components in `0.0 ..= 1.0` for the renderer.
    pub fn to_f32_array(self) -> [f32; 4] {
        // I3-exempt: render boundary conversion.
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

/// Why a colour literal was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColorError {
    /// No leading `#`.
    #[error("colours start with '#'")]
    MissingHash,
    /// Not eight hex digits.
    #[error("expected 8 hex digits (rrggbbaa), found {0}")]
    WrongLength(usize),
    /// A digit was not hex.
    #[error("colours are hexadecimal")]
    NotHex,
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_hex())
    }
}
impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A typed cross-reference.
///
/// The prefix is what makes dependency scanning a grep rather than a
/// schema-aware walk: every reference in every scene file is greppable by its
/// kind without parsing anything.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "lowercase")]
pub enum Reference {
    /// `asset:sprites/player`
    Asset(String),
    /// `scene:prefabs/skeleton`
    Scene(String),
    /// `node:n_m9v2ht5w`
    Node(String),
    /// `script:scripts/enemy.lua`
    Script(String),
}

impl Reference {
    /// The prefix, including the colon.
    pub fn prefix(&self) -> &'static str {
        match self {
            Reference::Asset(_) => "asset:",
            Reference::Scene(_) => "scene:",
            Reference::Node(_) => "node:",
            Reference::Script(_) => "script:",
        }
    }

    /// The part after the prefix.
    pub fn target(&self) -> &str {
        match self {
            Reference::Asset(s)
            | Reference::Scene(s)
            | Reference::Node(s)
            | Reference::Script(s) => s,
        }
    }

    /// Parse any reference form.
    pub fn parse(s: &str) -> Result<Reference, RefError> {
        match s.split_once(':') {
            Some(("asset", p)) => Ok(Reference::Asset(p.to_string())),
            Some(("scene", p)) => Ok(Reference::Scene(p.to_string())),
            Some(("node", p)) => Ok(Reference::Node(p.to_string())),
            Some(("script", p)) => Ok(Reference::Script(p.to_string())),
            Some((other, _)) => Err(RefError::UnknownPrefix(other.to_string())),
            None => Err(RefError::NoPrefix(s.to_string())),
        }
    }

    /// Render as text.
    pub fn to_text(&self) -> String {
        format!("{}{}", self.prefix(), self.target())
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_text())
    }
}

/// Why a reference literal was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    /// No `prefix:` at all.
    #[error("{0:?} has no reference prefix; expected one of asset:, scene:, node:, script:")]
    NoPrefix(String),
    /// A prefix that is not one of the four.
    #[error("{0:?} is not a known reference prefix")]
    UnknownPrefix(String),
}

/// A property value.
///
/// Several of these share a TOML representation — an enum, a reference and a
/// plain string are all quoted strings on disk. Parsing is therefore driven by
/// the kind's schema rather than by the literal alone, which is also what lets
/// a wrong type be reported as `DIM0201` instead of being quietly accepted.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// Fixed-point scalar, written `12.5`.
    Scalar(Fx),
    /// Whole number, written `42`.
    Int(i64),
    /// `true` or `false`.
    Bool(bool),
    /// Free text.
    Str(String),
    /// `[12.5, -4.0]`.
    Vec2(Vec2Fx),
    /// `[16, 16]`, for grid coordinates and cell sizes.
    Vec2i([i32; 2]),
    /// `[x, y, w, h]`.
    Rect(Rect),
    /// Degrees in text, binary angle units in memory.
    Angle(Angle),
    /// `"#rrggbbaa"`.
    Color(Color),
    /// One of a fixed set of names.
    Enum(String),
    /// A typed cross-reference.
    Ref(Reference),
    /// A homogeneous list.
    List(Vec<Value>),
    /// A string-keyed map. Ordered, so serialization is stable (I4).
    Map(IndexMap<String, Value>),
}

impl Value {
    /// The schema type name, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Scalar(_) => "scalar",
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::Str(_) => "string",
            Value::Vec2(_) => "vec2",
            Value::Vec2i(_) => "vec2i",
            Value::Rect(_) => "rect",
            Value::Angle(_) => "angle",
            Value::Color(_) => "color",
            Value::Enum(_) => "enum",
            Value::Ref(_) => "reference",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// The scalar inside, if this is one.
    pub fn as_scalar(&self) -> Option<Fx> {
        match self {
            Value::Scalar(v) => Some(*v),
            Value::Int(v) => i32::try_from(*v).ok().map(Fx::from_int),
            _ => None,
        }
    }

    /// The integer inside, if this is one.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// The boolean inside, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// The text inside, for strings and enums alike.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// The vector inside, if this is one.
    pub fn as_vec2(&self) -> Option<Vec2Fx> {
        match self {
            Value::Vec2(v) => Some(*v),
            Value::Vec2i([x, y]) => Some(Vec2Fx::from_ints(*x, *y)),
            _ => None,
        }
    }

    /// The grid coordinate inside, if this is one.
    pub fn as_vec2i(&self) -> Option<[i32; 2]> {
        match self {
            Value::Vec2i(v) => Some(*v),
            _ => None,
        }
    }

    /// The reference inside, if this is one.
    pub fn as_ref_value(&self) -> Option<&Reference> {
        match self {
            Value::Ref(r) => Some(r),
            _ => None,
        }
    }

    /// The colour inside, if this is one.
    pub fn as_color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// The angle inside, if this is one.
    pub fn as_angle(&self) -> Option<Angle> {
        match self {
            Value::Angle(a) => Some(*a),
            _ => None,
        }
    }

    /// Feed into a state hash. Tagged by type so that, say, the integer 1 and
    /// the scalar 1.0 do not collide.
    pub fn hash_state(&self, h: &mut dimetric_core::StateHasher) {
        h.tag(self.type_name());
        match self {
            Value::Scalar(v) => {
                h.fx(*v);
            }
            Value::Int(v) => {
                h.i64(*v);
            }
            Value::Bool(v) => {
                h.bool(*v);
            }
            Value::Str(s) | Value::Enum(s) => {
                h.str(s);
            }
            Value::Vec2(v) => {
                h.vec2(*v);
            }
            Value::Vec2i([x, y]) => {
                h.i32(*x).i32(*y);
            }
            Value::Rect(r) => {
                h.vec2(r.pos).vec2(r.size);
            }
            Value::Angle(a) => {
                h.angle(*a);
            }
            Value::Color(c) => {
                h.bytes(&[c.r, c.g, c.b, c.a]);
            }
            Value::Ref(r) => {
                h.str(&r.to_text());
            }
            Value::List(items) => {
                h.len(items.len());
                for v in items {
                    v.hash_state(h);
                }
            }
            Value::Map(map) => {
                h.len(map.len());
                // Sorted, not insertion-ordered: two scenes with the same
                // contents must hash the same however they were authored (I4).
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for k in keys {
                    h.str(k);
                    map[k].hash_state(h);
                }
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Scalar(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Str(s) | Value::Enum(s) => write!(f, "{s:?}"),
            Value::Vec2(v) => write!(f, "[{}, {}]", v.x, v.y),
            Value::Vec2i([x, y]) => write!(f, "[{x}, {y}]"),
            Value::Rect(r) => write!(f, "[{}, {}, {}, {}]", r.pos.x, r.pos.y, r.size.x, r.size.y),
            Value::Angle(a) => write!(f, "{a}"),
            Value::Color(c) => write!(f, "{c:?}"),
            Value::Ref(r) => write!(f, "{r:?}"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                write!(f, "{{ ")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k} = {v}")?;
                }
                write!(f, " }}")
            }
        }
    }
}
