//! Lua scripting.
//!
//! # The determinism trap, stated loudly
//!
//! **Lua numbers are `f64`.** Gameplay arithmetic done in raw Lua numbers and
//! then written into simulation state is the easiest way for a script — or an
//! agent — to break replay, and it will happen unless the rule is obvious.
//!
//! So: engine math is exposed as *userdata backed by fixed point*. `vec2` and
//! `fx` values do their arithmetic in `Fx`, never in `f64`. A plain Lua number
//! crossing into sim state is converted with deterministic rounding at the
//! boundary, and the non-portable parts of Lua's `math` library — `sin`, `cos`,
//! `exp`, `sqrt` and friends, whose results differ between platforms — are
//! removed from the sandbox and replaced by fixed-point equivalents on `fx`.
//!
//! # Where script state lives
//!
//! In Rust, not in Lua. `self.hp = 40` writes into [`SimState::vars`], and
//! `self.hp` reads it back. Lua holds code; the simulation holds state. A Lua
//! table cannot be snapshotted and restored bit-for-bit, so anything kept there
//! would silently drop out of rollback and out of the state hash.
//!
//! # Handles
//!
//! Nodes reach Lua as lightweight handles carrying a [`NodeUid`], validated on
//! every call. Passing Rust references instead would tangle the binding in
//! lifetimes immediately, and a handle to a destroyed node has to fail cleanly
//! rather than resolve to whatever was allocated in its place.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use dimetric_core::{Angle, Code, Diagnostic, Fx, NodeUid, Vec2Fx};
use dimetric_scene::Value;
use indexmap::IndexMap;
use mlua::{Lua, MultiValue, Table, UserData, UserDataMethods, Variadic};

use crate::state::{SignalEvent, SimState};
use crate::tick::{Hook, ScriptHost};

/// Shared handle to the state a hook is running against.
type Shared = Rc<RefCell<SimState>>;

/// What `require` can reach, shared with every environment that has one.
///
/// Held behind an `Rc<RefCell<_>>` because `require` is a Lua closure that can
/// call itself: a module may require another, which builds an environment with
/// its own `require` in it.
#[derive(Default)]
struct Modules {
    /// Every loaded script's source, by project-relative path. A module is
    /// evaluated from here rather than from disk — the simulation does no I/O,
    /// which is the same reason the sandbox has no `io`.
    sources: BTreeMap<String, String>,
    /// Modules already evaluated, frozen. A module is evaluated once, so two
    /// scripts requiring the same path get the same table.
    cache: BTreeMap<String, mlua::Value>,
    /// The chain being evaluated right now, so a cycle is an error naming the
    /// loop rather than a stack overflow.
    loading: Vec<String>,
}

/// Marks a table the engine has frozen, and protects the metatable from being
/// swapped for one that would let writes through.
const FROZEN: &str = "dimetric.module";

/// A fixed-point scalar, as Lua sees it.
#[derive(Clone, Copy, Debug)]
pub struct LuaFx(pub Fx);

/// A fixed-point vector, as Lua sees it.
#[derive(Clone, Copy, Debug)]
pub struct LuaVec2(pub Vec2Fx);

/// A validated reference to a node.
#[derive(Clone, Copy, Debug)]
pub struct NodeHandle(pub NodeUid);

impl UserData for LuaFx {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("abs", |_, this, ()| Ok(LuaFx(this.0.abs())));
        m.add_method("floor", |_, this, ()| Ok(this.0.floor_int()));
        m.add_method("round", |_, this, ()| Ok(this.0.round_int()));
        m.add_method("sqrt", |_, this, ()| Ok(LuaFx(this.0.sqrt())));
        // I3-exempt: handing a scalar to Lua is the scripting boundary. What
        // comes back is converted with deterministic rounding, never accumulated.
        m.add_method("tonumber", |_, this, ()| Ok(this.0.to_f64()));
        m.add_meta_method("__add", |_, this, other: LuaFx| Ok(LuaFx(this.0 + other.0)));
        m.add_meta_method("__sub", |_, this, other: LuaFx| Ok(LuaFx(this.0 - other.0)));
        m.add_meta_method("__mul", |_, this, other: LuaFx| Ok(LuaFx(this.0 * other.0)));
        m.add_meta_method("__div", |_, this, other: LuaFx| Ok(LuaFx(this.0 / other.0)));
        m.add_meta_method("__unm", |_, this, ()| Ok(LuaFx(-this.0)));
        m.add_meta_method("__eq", |_, this, other: LuaFx| Ok(this.0 == other.0));
        m.add_meta_method("__lt", |_, this, other: LuaFx| Ok(this.0 < other.0));
        m.add_meta_method("__le", |_, this, other: LuaFx| Ok(this.0 <= other.0));
        m.add_meta_method("__tostring", |_, this, ()| Ok(this.0.to_exact_string()));
    }
}

impl mlua::FromLua for LuaFx {
    fn from_lua(value: mlua::Value, _lua: &Lua) -> mlua::Result<LuaFx> {
        match value {
            mlua::Value::UserData(ud) => Ok(*ud.borrow::<LuaFx>()?),
            mlua::Value::Integer(i) => Ok(LuaFx(Fx::from_int(i as i32))),
            // I3-exempt: the scripting boundary. Lua numbers are f64 and this
            // is the single point where one becomes a scalar.
            mlua::Value::Number(n) => Ok(LuaFx(Fx::from_f64_lossy(n))),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "fx".into(),
                message: Some("expected a number or an fx value".into()),
            }),
        }
    }
}

impl UserData for LuaVec2 {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("length", |_, this, ()| Ok(LuaFx(this.0.length())));
        m.add_method("length_squared", |_, this, ()| {
            // Squared magnitudes are the accumulator type, and narrowing here
            // would defeat the point of having one. Hand Lua the f64 view; it
            // is for comparisons, and comparisons in Lua never reach sim state.
            Ok(this.0.length_squared().to_exact_string())
        });
        m.add_method("normalized", |_, this, ()| Ok(LuaVec2(this.0.normalized())));
        m.add_method("perp", |_, this, ()| Ok(LuaVec2(this.0.perp())));
        m.add_method("dot", |_, this, other: LuaVec2| {
            Ok(LuaFx(this.0.dot(other.0).narrow_saturating()))
        });
        m.add_method("angle", |_, this, ()| {
            Ok(LuaFx(Fx::from_raw(
                (this.0.to_angle().to_bam() as i32) << 16 >> 16,
            )))
        });
        m.add_method("angle_degrees", |_, this, ()| {
            Ok(this.0.to_angle().to_degrees_string())
        });
        m.add_method("rotated", |_, this, degrees: String| {
            let angle = Angle::from_degrees_str(&degrees)
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            Ok(LuaVec2(this.0.rotated(angle)))
        });
        m.add_method("with_length", |_, this, len: LuaFx| {
            Ok(LuaVec2(this.0.with_length(len.0)))
        });
        m.add_method("clamp_length", |_, this, max: LuaFx| {
            Ok(LuaVec2(this.0.clamp_length(max.0)))
        });
        m.add_method("x", |_, this, ()| Ok(LuaFx(this.0.x)));
        m.add_method("y", |_, this, ()| Ok(LuaFx(this.0.y)));
        m.add_meta_method("__add", |_, this, o: LuaVec2| Ok(LuaVec2(this.0 + o.0)));
        m.add_meta_method("__sub", |_, this, o: LuaVec2| Ok(LuaVec2(this.0 - o.0)));
        m.add_meta_method("__unm", |_, this, ()| Ok(LuaVec2(-this.0)));
        m.add_meta_method("__mul", |_, this, s: LuaFx| Ok(LuaVec2(this.0 * s.0)));
        m.add_meta_method("__div", |_, this, s: LuaFx| Ok(LuaVec2(this.0 / s.0)));
        m.add_meta_method("__eq", |_, this, o: LuaVec2| Ok(this.0 == o.0));
        m.add_meta_method("__tostring", |_, this, ()| {
            Ok(format!(
                "({}, {})",
                this.0.x.to_exact_string(),
                this.0.y.to_exact_string()
            ))
        });
        m.add_meta_method("__index", |lua, this, key: String| match key.as_str() {
            "x" => Ok(mlua::Value::UserData(lua.create_userdata(LuaFx(this.0.x))?)),
            "y" => Ok(mlua::Value::UserData(lua.create_userdata(LuaFx(this.0.y))?)),
            _ => Ok(mlua::Value::Nil),
        });
    }
}

impl mlua::FromLua for LuaVec2 {
    fn from_lua(value: mlua::Value, _lua: &Lua) -> mlua::Result<LuaVec2> {
        match value {
            mlua::Value::UserData(ud) => Ok(*ud.borrow::<LuaVec2>()?),
            other => Err(mlua::Error::FromLuaConversionError {
                from: other.type_name(),
                to: "vec2".into(),
                message: Some("expected a vec2".into()),
            }),
        }
    }
}

fn shared(lua: &Lua) -> mlua::Result<Shared> {
    lua.app_data_ref::<Shared>()
        .map(|r| r.clone())
        .ok_or_else(|| mlua::Error::runtime("no simulation is running"))
}

impl mlua::FromLua for NodeHandle {
    fn from_lua(value: mlua::Value, _: &Lua) -> mlua::Result<NodeHandle> {
        match value {
            mlua::Value::UserData(ud) => ud.borrow::<NodeHandle>().map(|h| *h),
            other => Err(mlua::Error::runtime(format!(
                "expected a node, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for NodeHandle {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("id", |_, this, ()| Ok(this.0.to_text()));

        m.add_method("name", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state.scene.get(id).map(|n| n.name.clone()))
        });

        m.add_method("path", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state.scene.path_of(id))
        });

        m.add_method("kind", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state.scene.get(id).map(|n| n.kind.clone()))
        });

        m.add_method("valid", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state.scene.by_uid(this.0).is_some())
        });

        m.add_method("has_tag", |lua, this, tag: String| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state.scene.get(id).is_some_and(|n| n.has_tag(&tag)))
        });

        // Kind properties, the ones declared by the node's schema.
        m.add_method("get", |lua, this, key: String| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            match state.scene.get(id).and_then(|n| n.get(&key)) {
                Some(v) => to_lua(lua, v),
                None => Ok(mlua::Value::Nil),
            }
        });

        m.add_method("set", |lua, this, (key, value): (String, mlua::Value)| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            let id = resolve(&state, this.0)?;
            let value = from_lua(value)?;
            if let Some(node) = state.scene.node_mut_no_transform(id) {
                node.set(key, value);
            }
            Ok(())
        });

        m.add_method("find", |lua, this, name: String| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state
                .scene
                .child_named(id, &name)
                .and_then(|c| state.scene.get(c))
                .map(|n| NodeHandle(n.uid)))
        });

        m.add_method("parent", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(state
                .scene
                .get(id)
                .and_then(|n| n.parent())
                .and_then(|p| state.scene.get(p))
                .map(|n| NodeHandle(n.uid)))
        });

        m.add_method("children", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            let handles: Vec<NodeHandle> = state
                .scene
                .children(id)
                .filter_map(|c| state.scene.get(c))
                .map(|n| NodeHandle(n.uid))
                .collect();
            lua.create_sequence_from(handles)
        });

        m.add_method(
            "emit",
            |lua, this, (name, payload): (String, Option<Table>)| {
                let state = shared(lua)?;
                let mut state = state.borrow_mut();
                let mut fields = IndexMap::new();
                if let Some(table) = payload {
                    // Sorted, so a payload built in a different order still
                    // produces the same delivery and the same hash (I4).
                    let mut pairs: Vec<(String, mlua::Value)> = table
                        .pairs::<String, mlua::Value>()
                        .collect::<mlua::Result<Vec<_>>>()?;
                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                    for (k, v) in pairs {
                        fields.insert(k, from_lua(v)?);
                    }
                }
                state.signals.push(SignalEvent {
                    from: this.0,
                    name,
                    payload: fields,
                });
                Ok(())
            },
        );

        m.add_method("destroy", |lua, this, ()| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            if !state.destroy_queue.contains(&this.0) {
                state.destroy_queue.push(this.0);
            }
            Ok(())
        });

        // Sound nodes. The properties are authored on the node, so a script
        // says *when* rather than *what* — which keeps the clip, the bus and
        // the gain somewhere a designer can see them and an override can reach.
        m.add_method("play", |lua, this, ()| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            let id = resolve(&state, this.0)?;
            let cue = state
                .scene
                .get(id)
                .and_then(crate::sound::SoundCue::of)
                .ok_or_else(|| mlua::Error::runtime("play() needs a Sound node with a stream"))?;
            state.autoplayed.push(cue.node);
            state.sounds.push(crate::sound::SoundEvent::Play(cue));
            Ok(())
        });

        m.add_method("stop", |lua, this, ()| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            let id = resolve(&state, this.0)?;
            let uid = state.scene.get(id).map(|n| n.uid).unwrap_or(this.0);
            state.autoplayed.retain(|n| *n != uid);
            state
                .sounds
                .push(crate::sound::SoundEvent::Stop { node: uid });
            Ok(())
        });

        m.add_method("set_velocity", |lua, this, v: LuaVec2| {
            let state = shared(lua)?;
            state.borrow_mut().velocity.insert(this.0, v.0);
            Ok(())
        });

        m.add_method("velocity", |lua, this, ()| {
            let state = shared(lua)?;
            let v = state.borrow().velocity_of(this.0);
            Ok(LuaVec2(v))
        });

        m.add_method("world_pos", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            Ok(LuaVec2(
                state
                    .scene
                    .world_of(id)
                    .map(|t| t.pos)
                    .unwrap_or(Vec2Fx::ZERO),
            ))
        });

        // `self.hp` reads a script variable; `self.pos` reads the transform.
        m.add_meta_method("__index", |lua, this, key: String| {
            let state = shared(lua)?;
            let state = state.borrow();
            let id = resolve(&state, this.0)?;
            match key.as_str() {
                "pos" => {
                    let pos = state
                        .scene
                        .get(id)
                        .map(|n| n.transform.pos)
                        .unwrap_or(Vec2Fx::ZERO);
                    Ok(mlua::Value::UserData(lua.create_userdata(LuaVec2(pos))?))
                }
                "rot" => {
                    let rot = state
                        .scene
                        .get(id)
                        .map(|n| n.transform.rot)
                        .unwrap_or(Angle::ZERO);
                    Ok(mlua::Value::String(
                        lua.create_string(rot.to_degrees_string())?,
                    ))
                }
                "visible" => Ok(mlua::Value::Boolean(
                    state.scene.get(id).is_some_and(|n| n.visible),
                )),
                _ => match state.var(this.0, &key) {
                    Some(v) => to_lua(lua, v),
                    None => Ok(mlua::Value::Nil),
                },
            }
        });

        m.add_meta_method(
            "__newindex",
            |lua, this, (key, value): (String, mlua::Value)| {
                let state = shared(lua)?;
                let mut state = state.borrow_mut();
                let id = resolve(&state, this.0)?;
                match key.as_str() {
                    "pos" => {
                        let v = LuaVec2::from_lua(value, lua)?;
                        state.scene.set_position(id, v.0);
                    }
                    "rot" => {
                        let text = match value {
                            mlua::Value::String(s) => s.to_str()?.to_string(),
                            mlua::Value::Integer(i) => format!("{i}.0"),
                            mlua::Value::Number(n) => format!("{n}"),
                            other => {
                                return Err(mlua::Error::runtime(format!(
                                    "rot expects degrees, found {}",
                                    other.type_name()
                                )))
                            }
                        };
                        let angle = Angle::from_degrees_str(&text)
                            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                        if let Some(node) = state.scene.get_mut(id) {
                            node.transform.rot = angle;
                        }
                    }
                    "visible" => {
                        if let (Some(node), mlua::Value::Boolean(b)) =
                            (state.scene.node_mut_no_transform(id), &value)
                        {
                            node.visible = *b;
                        }
                    }
                    _ => {
                        let v = from_lua(value)?;
                        state.set_var(this.0, key, v);
                    }
                }
                Ok(())
            },
        );

        m.add_meta_method("__tostring", |lua, this, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state
                .scene
                .by_uid(this.0)
                .and_then(|id| state.scene.path_of(id))
                .unwrap_or_else(|| format!("<destroyed {}>", this.0)))
        });
    }
}

use mlua::FromLua as _;

/// Resolve a handle, failing cleanly when the node is gone.
fn resolve(state: &SimState, uid: NodeUid) -> mlua::Result<dimetric_core::NodeId> {
    state
        .scene
        .by_uid(uid)
        .ok_or_else(|| mlua::Error::runtime(format!("node {uid} no longer exists")))
}

/// Convert a Lua table into a list or a map.
///
/// Lua has one table type and the engine has two values, so the shape has to be
/// inferred: a table whose keys are exactly `1..=n` is a list, and anything
/// else is a map. Sequences matter here beyond tidiness — an ordered list is
/// how a script keeps a spawn table or an upgrade order reproducible, and a map
/// iterates in key order, which is not the order anybody wrote (I4).
///
/// A table mixing the two is refused rather than silently losing half of it.
fn table_to_value(t: mlua::Table) -> mlua::Result<Value> {
    let mut integer_keys = Vec::new();
    let mut string_keys = Vec::new();
    for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
        let (key, value) = pair?;
        match key {
            mlua::Value::Integer(i) => integer_keys.push((i, value)),
            mlua::Value::String(s) => string_keys.push((s.to_str()?.to_string(), value)),
            // A table keyed by anything else has no representation in the scene
            // format, so it cannot be stored and saying so beats dropping it.
            other => {
                return Err(mlua::Error::runtime(format!(
                    "a table key must be a string or an integer, not {}",
                    other.type_name()
                )))
            }
        }
    }

    if !integer_keys.is_empty() && !string_keys.is_empty() {
        return Err(mlua::Error::runtime(
            "a table mixing array entries and named keys has no engine value; \
             use one or the other",
        ));
    }

    if !integer_keys.is_empty() {
        integer_keys.sort_by_key(|(i, _)| *i);
        let contiguous = integer_keys
            .iter()
            .enumerate()
            .all(|(index, (key, _))| *key == index as i64 + 1);
        if !contiguous {
            return Err(mlua::Error::runtime(
                "an array with gaps in it has no engine value; \
                 a list is 1..n with nothing missing",
            ));
        }
        let mut items = Vec::with_capacity(integer_keys.len());
        for (_, value) in integer_keys {
            items.push(from_lua(value)?);
        }
        return Ok(Value::List(items));
    }

    // Sorted, so the same table always produces the same value and the same
    // state hash whatever order Lua happened to iterate it in.
    string_keys.sort_by(|a, b| a.0.cmp(&b.0));
    let mut map = IndexMap::new();
    for (key, value) in string_keys {
        map.insert(key, from_lua(value)?);
    }
    Ok(Value::Map(map))
}

/// Convert an engine value into Lua.
fn to_lua(lua: &Lua, value: &Value) -> mlua::Result<mlua::Value> {
    Ok(match value {
        Value::Scalar(v) => mlua::Value::UserData(lua.create_userdata(LuaFx(*v))?),
        Value::Int(v) => mlua::Value::Integer(*v),
        Value::Bool(v) => mlua::Value::Boolean(*v),
        Value::Str(s) | Value::Enum(s) => mlua::Value::String(lua.create_string(s)?),
        Value::Vec2(v) => mlua::Value::UserData(lua.create_userdata(LuaVec2(*v))?),
        Value::Vec2i([x, y]) => {
            mlua::Value::UserData(lua.create_userdata(LuaVec2(Vec2Fx::from_ints(*x, *y)))?)
        }
        Value::Angle(a) => mlua::Value::String(lua.create_string(a.to_degrees_string())?),
        Value::Color(c) => mlua::Value::String(lua.create_string(c.to_hex())?),
        Value::Ref(r) => mlua::Value::String(lua.create_string(r.to_text())?),
        Value::Rect(r) => {
            let t = lua.create_table()?;
            t.set("pos", LuaVec2(r.pos))?;
            t.set("size", LuaVec2(r.size))?;
            mlua::Value::Table(t)
        }
        Value::List(items) => {
            let t = lua.create_table()?;
            for (i, v) in items.iter().enumerate() {
                t.set(i + 1, to_lua(lua, v)?)?;
            }
            mlua::Value::Table(t)
        }
        Value::Map(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                t.set(k.as_str(), to_lua(lua, v)?)?;
            }
            mlua::Value::Table(t)
        }
    })
}

/// Convert a Lua value into an engine value.
///
/// A Lua float becomes a scalar by deterministic rounding. The rounding itself
/// is reproducible everywhere; what is not safe is *accumulating* in Lua floats
/// and writing the result here, which is why the fixed-point userdata exists.
fn from_lua(value: mlua::Value) -> mlua::Result<Value> {
    Ok(match value {
        mlua::Value::Nil => Value::Bool(false),
        mlua::Value::Boolean(b) => Value::Bool(b),
        mlua::Value::Integer(i) => Value::Int(i),
        // I3-exempt: the scripting boundary, as above.
        mlua::Value::Number(n) => Value::Scalar(Fx::from_f64_lossy(n)),
        mlua::Value::String(s) => Value::Str(s.to_str()?.to_string()),
        mlua::Value::UserData(ud) => {
            if let Ok(v) = ud.borrow::<LuaFx>() {
                Value::Scalar(v.0)
            } else if let Ok(v) = ud.borrow::<LuaVec2>() {
                Value::Vec2(v.0)
            } else if let Ok(h) = ud.borrow::<NodeHandle>() {
                Value::Str(h.0.to_text())
            } else {
                return Err(mlua::Error::runtime("unsupported userdata"));
            }
        }
        mlua::Value::Table(t) => table_to_value(t)?,
        other => {
            return Err(mlua::Error::runtime(format!(
                "cannot store a {} in simulation state",
                other.type_name()
            )))
        }
    })
}

/// Runs node scripts.
pub struct LuaHost {
    lua: Lua,
    /// Script path to its environment table, which holds its hook functions.
    scripts: BTreeMap<String, Table>,
    /// Sources and evaluated modules, shared with every `require` closure.
    modules: Rc<RefCell<Modules>>,
    /// Ticks per second, for `tick.dt`.
    tick_rate: u32,
    /// Messages scripts wrote, in order, shared with the `log` closures.
    ///
    /// Deliberately not in `SimState`. A log line is output, the way a sound
    /// is: if writing one could reach the state hash, a script that logged
    /// only when a flag was on would play a different game with the flag off.
    /// Keeping the buffer over here rather than in a field the hash skips
    /// means there is nothing to get wrong later.
    log: Rc<RefCell<Vec<String>>>,
    /// What accumulates across runs.
    ///
    /// Beside the log lines and deliberately not in `SimState`: a profile
    /// differs between two players playing the same seed, so a field on the
    /// state is a field a later change starts hashing by accident. See
    /// [`crate::profile`].
    profile: Rc<RefCell<crate::profile::Profile>>,
    /// Baked fonts, for `ui.measure`.
    ///
    /// On the host rather than in `SimState` because the metric tables are
    /// large, unchanging, and come from the project rather than the run —
    /// exactly like the animation clips, and for the same reason a module's
    /// table is not in the state either. What a script *derives* from them
    /// lands in a control's rectangle, which is hashed.
    fonts: Rc<RefCell<crate::text::Fonts>>,
    /// What the simulation has told the host this tick.
    ///
    /// Beside the log lines and deliberately not on `SimState`: a later change
    /// cannot start hashing a field that does not exist. See [`crate::event`].
    events: Rc<RefCell<Vec<crate::event::GameEvent>>>,
}

impl LuaHost {
    /// Create a host with the sandbox installed.
    pub fn new(tick_rate: u32) -> Result<LuaHost, Diagnostic> {
        let lua = Lua::new();
        Ok(LuaHost {
            events: Rc::new(RefCell::new(Vec::new())),
            profile: Rc::new(RefCell::new(crate::profile::Profile::new())),
            fonts: Rc::new(RefCell::new(crate::text::Fonts::new())),
            lua,
            scripts: BTreeMap::new(),
            modules: Rc::new(RefCell::new(Modules::default())),
            tick_rate,
            log: Rc::new(RefCell::new(Vec::new())),
        })
    }

    /// Load a script under a project-relative path.
    ///
    /// The source is registered before it runs, so a script can `require`
    /// itself out of the same registry every other script reads.
    pub fn load(&mut self, path: &str, source: &str) -> Result<(), Diagnostic> {
        self.modules
            .borrow_mut()
            .sources
            .insert(path.to_string(), source.to_string());
        let env = build_environment(&self.lua, self.tick_rate, &self.handles(), path)?;
        self.lua
            .load(source)
            .set_name(path)
            .set_environment(env.clone())
            .exec()
            .map_err(|e| syntax_error(path, e))?;
        self.scripts.insert(path.to_string(), env);
        Ok(())
    }

    /// Load a project's whole script set.
    ///
    /// Every source is registered before any of them runs. Loading one at a
    /// time would make `require` depend on the order the caller happened to
    /// iterate in — `scripts/arena.lua` sorts before `scripts/spellbook.lua`,
    /// so requiring the second from the first would fail on nothing but the
    /// alphabet. Errors are collected rather than returned at the first one: a
    /// project with two broken scripts should report both.
    pub fn load_all<'a, I>(&mut self, scripts: I) -> Vec<Diagnostic>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let paths: Vec<String> = scripts
            .into_iter()
            .map(|(path, source)| {
                self.modules
                    .borrow_mut()
                    .sources
                    .insert(path.to_string(), source.to_string());
                path.to_string()
            })
            .collect();
        let mut diags = Vec::new();
        for path in paths {
            let source = self.modules.borrow().sources[&path].clone();
            if let Err(d) = self.load(&path, &source) {
                diags.push(d);
            }
        }
        diags
    }

    /// True when a script has been loaded.
    /// The profile store this host hands to scripts.
    ///
    /// Shared with the sandbox, so a host that loads a saved profile into it
    /// before the first tick — and writes it back when it reports itself
    /// dirty — is talking to the same table `profile.get` reads.
    ///
    /// Deliberately not on `ScriptHost`: the trait cannot hand out a `&mut`
    /// through an `Rc<RefCell<_>>`, and a trait method that returned a copy
    /// would be a profile whose writes went nowhere.
    /// Supply the baked fonts a script may measure text with.
    ///
    /// Handed in by the host, like the animation clips: a run that could not
    /// measure its own text would lay its interface out differently from one
    /// that could, and layout is hashed.
    pub fn set_fonts(&mut self, fonts: crate::text::Fonts) {
        *self.fonts.borrow_mut() = fonts;
    }

    /// The profile store this host hands to scripts.
    ///
    /// Shared with the sandbox, so a host that loads a saved profile into it
    /// before the first tick — and writes it back when it reports itself
    /// dirty — is talking to the same table `profile.get` reads.
    /// The handles this host shares with every sandbox it builds.
    fn handles(&self) -> HostHandles {
        HostHandles {
            modules: self.modules.clone(),
            log: self.log.clone(),
            profile: self.profile.clone(),
            fonts: self.fonts.clone(),
            events: self.events.clone(),
        }
    }

    /// The profile store this host hands to scripts.
    ///
    /// Shared with the sandbox, so a host that loads a saved profile into it
    /// before the first tick — and writes it back when it reports itself
    /// dirty — is talking to the same table `profile.get` reads.
    pub fn profile_handle(&self) -> Rc<RefCell<crate::profile::Profile>> {
        self.profile.clone()
    }

    /// Whether a script is loaded.
    pub fn has(&self, path: &str) -> bool {
        self.scripts.contains_key(path)
    }
}

/// The sandbox: exactly what a script may reach, and nothing else.
///
/// `os`, `io`, `dofile`, `load` and `package` are absent — a script that could
/// read the clock or the filesystem could break determinism without the engine
/// ever knowing. `math.random` is gone because randomness must come from a
/// seeded stream (I6), and the transcendental functions are gone because
/// platform `libm` does not agree with itself across operating systems.
///
/// `require` is here, but it is the engine's: it reads project scripts out of
/// memory rather than files off disk, and freezes what they return.
/// `rawset` is not, because it writes past a `__newindex` and would walk
/// straight through that freeze. Plain assignment does everything a script
/// needs; `rawget` and `rawlen` stay, since reading past a metatable breaks
/// nothing.
/// The handles a sandbox shares with its host.
///
/// Bundled because the list had grown to the point where the argument order
/// was the only thing holding it together. Everything in here has the same
/// shape and the same reason for existing: it belongs to the host rather than
/// to `SimState`, either because it must never be hashed (the log lines, the
/// profile) or because it is unchanging project data the run does not own (the
/// module cache, the fonts).
#[derive(Clone)]
pub(crate) struct HostHandles {
    modules: Rc<RefCell<Modules>>,
    log: Rc<RefCell<Vec<String>>>,
    profile: Rc<RefCell<crate::profile::Profile>>,
    fonts: Rc<RefCell<crate::text::Fonts>>,
    events: Rc<RefCell<Vec<crate::event::GameEvent>>>,
}

fn build_environment(
    lua: &Lua,
    tick_rate: u32,
    shared_state: &HostHandles,
    path: &str,
) -> Result<Table, Diagnostic> {
    let env = lua.create_table().map_err(|e| runtime_error(path, e))?;
    let globals = lua.globals();

    for name in [
        "assert",
        "error",
        "ipairs",
        "next",
        "pairs",
        "pcall",
        "select",
        "tonumber",
        "tostring",
        "type",
        "xpcall",
        "rawequal",
        "rawget",
        "rawlen",
        "setmetatable",
        "getmetatable",
    ] {
        if let Ok(v) = globals.get::<mlua::Value>(name) {
            let _ = env.set(name, v);
        }
    }
    for name in ["string", "table"] {
        if let Ok(v) = globals.get::<mlua::Value>(name) {
            let _ = env.set(name, v);
        }
    }

    // A reduced `math`: the exactly-defined integer operations stay, the
    // platform-dependent ones do not.
    if let Ok(math) = globals.get::<Table>("math") {
        let safe = lua.create_table().map_err(|e| runtime_error(path, e))?;
        for name in [
            "abs",
            "ceil",
            "floor",
            "fmod",
            "max",
            "min",
            "tointeger",
            "type",
        ] {
            if let Ok(v) = math.get::<mlua::Value>(name) {
                let _ = safe.set(name, v);
            }
        }
        // I3-exempt: `math.huge` is a Lua constant scripts compare against;
        // it never becomes simulation state.
        let _ = safe.set("huge", f64::INFINITY);
        let _ = env.set("math", safe);
    }

    env.set("_G", env.clone())
        .map_err(|e| runtime_error(path, e))?;
    install_api(lua, tick_rate, shared_state, &env, path)?;
    Ok(env)
}

/// Resolve a handle to a `TileLayer`, or say why it is not one.
///
/// Checked by the built-in a kind *behaves as* rather than by its name, so a
/// project that declares `Floor extends TileLayer` works — the same lookup
/// every other kind comparison in the engine goes through.
fn tile_layer(state: &SimState, NodeHandle(uid): NodeHandle) -> mlua::Result<NodeUid> {
    let Some(id) = state.scene.by_uid(uid) else {
        return Err(mlua::Error::external(Diagnostic::new(
            Code::STALE_HANDLE,
            format!("{uid} is not in the scene"),
        )));
    };
    let node = state.scene.get(id).expect("id from by_uid");
    if node.base != "TileLayer" {
        return Err(mlua::Error::external(Diagnostic::new(
            Code::SCRIPT_BAD_ARGUMENT,
            format!(
                "{} is a {} and the tiles API needs a TileLayer",
                node.name, node.kind
            ),
        )));
    }
    Ok(uid)
}

/// Narrow a Lua integer to a tile index.
///
/// Tiles are `u16` in the chunk format, and a script computing one from a
/// table lookup that came back `nil` would otherwise silently write whatever
/// the cast produced.
fn tile_index(v: i64) -> mlua::Result<u16> {
    u16::try_from(v).map_err(|_| {
        mlua::Error::external(Diagnostic::new(
            Code::SCRIPT_BAD_ARGUMENT,
            format!("{v} is not a tile index; they run from 0 to {}", u16::MAX),
        ))
    })
}

/// The names a script's environment actually holds.
///
/// Exists so the reference's table of globals can be *checked* rather than
/// maintained by hand. `docs/API.md` went a whole milestone claiming the
/// sandbox had ten globals while it had eleven, because that table was a
/// string literal; this is what stops the next one.
///
/// Sorted, so a caller comparing against [`crate::api_doc`] does not depend on
/// Lua's hash order (I4) — and this is a build-time question, not a simulation
/// one, but it is the same discipline and costs nothing.
pub fn sandbox_globals() -> Result<Vec<String>, Diagnostic> {
    let lua = Lua::new();
    let handles = HostHandles {
        modules: Rc::new(RefCell::new(Modules::default())),
        log: Rc::new(RefCell::new(Vec::new())),
        profile: Rc::new(RefCell::new(crate::profile::Profile::new())),
        fonts: Rc::new(RefCell::new(crate::text::Fonts::new())),
        events: Rc::new(RefCell::new(Vec::new())),
    };
    let env = build_environment(&lua, 60, &handles, "<introspection>")?;
    let mut names: Vec<String> = env
        .pairs::<String, mlua::Value>()
        .filter_map(|p: mlua::Result<(String, mlua::Value)>| p.ok().map(|(k, _)| k))
        .collect();
    names.sort();
    Ok(names)
}

fn install_api(
    lua: &Lua,
    tick_rate: u32,
    shared_state: &HostHandles,
    env: &Table,
    path: &str,
) -> Result<(), Diagnostic> {
    let log = &shared_state.log;
    let profile = &shared_state.profile;
    let fonts = &shared_state.fonts;
    let events = &shared_state.events;
    let err = |e: mlua::Error| runtime_error(path, e);

    // vec2(x, y)
    let vec2 = lua
        .create_function(|_, (x, y): (LuaFx, LuaFx)| Ok(LuaVec2(Vec2Fx::new(x.0, y.0))))
        .map_err(err)?;
    env.set("vec2", vec2).map_err(err)?;

    // fx: fixed-point construction and the trig the sandbox withholds.
    let fx = lua.create_table().map_err(err)?;
    fx.set(
        "new",
        lua.create_function(|_, v: LuaFx| Ok(v)).map_err(err)?,
    )
    .map_err(err)?;
    fx.set(
        "parse",
        lua.create_function(|_, s: String| {
            Fx::parse_exact(&s)
                .map(LuaFx)
                .map_err(|e| mlua::Error::runtime(e.to_string()))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    fx.set(
        "sin",
        lua.create_function(|_, degrees: String| {
            Angle::from_degrees_str(&degrees)
                .map(|a| LuaFx(a.sin()))
                .map_err(|e| mlua::Error::runtime(e.to_string()))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    fx.set(
        "cos",
        lua.create_function(|_, degrees: String| {
            Angle::from_degrees_str(&degrees)
                .map(|a| LuaFx(a.cos()))
                .map_err(|e| mlua::Error::runtime(e.to_string()))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    fx.set(
        "from_angle",
        lua.create_function(|_, degrees: String| {
            Angle::from_degrees_str(&degrees)
                .map(|a| LuaVec2(a.to_unit_vector()))
                .map_err(|e| mlua::Error::runtime(e.to_string()))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    env.set("fx", fx).map_err(err)?;

    // scene.find(path)
    let scene = lua.create_table().map_err(err)?;
    scene
        .set(
            "find",
            lua.create_function(|lua, p: String| {
                let state = shared(lua)?;
                let state = state.borrow();
                Ok(state
                    .scene
                    .resolve_path(&p)
                    .and_then(|id| state.scene.get(id))
                    .map(|n| NodeHandle(n.uid)))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    scene
        .set(
            "by_id",
            lua.create_function(|lua, id: String| {
                let state = shared(lua)?;
                let state = state.borrow();
                let uid = NodeUid::parse(&id).map_err(|e| mlua::Error::runtime(e.to_string()))?;
                Ok(state.scene.by_uid(uid).map(|_| NodeHandle(uid)))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    scene
        .set(
            "tagged",
            lua.create_function(|lua, tag: String| {
                let state = shared(lua)?;
                let state = state.borrow();
                // Depth-first order, so a script iterating this sees the
                // same sequence on every machine.
                let handles: Vec<NodeHandle> = state
                    .scene
                    .walk()
                    .into_iter()
                    .filter_map(|id| state.scene.get(id))
                    .filter(|n| n.has_tag(&tag))
                    .map(|n| NodeHandle(n.uid))
                    .collect();
                lua.create_sequence_from(handles)
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    // scene.spawn(prefab, at, parent) -> the id the node will have.
    //
    // The node does not exist yet: creating it mid-tick would put it into a
    // tree another script may be walking. It appears at the end of the
    // tick and gets `on_ready` on the next one. The id comes back now
    // because it is derived rather than drawn, so a script can hold it and
    // look the node up when it arrives.
    scene
        .set(
            "spawn",
            lua.create_function(
                |lua, (prefab, at, parent): (String, Option<LuaVec2>, Option<NodeHandle>)| {
                    let state = shared(lua)?;
                    let mut state = state.borrow_mut();
                    let counter = state.spawn_count;
                    state.spawn_count += 1;
                    // Seeded by the prefab's name rather than a template
                    // that may not be loaded, so the id is decided here and
                    // a missing prefab is a diagnostic rather than a panic.
                    let seed = crate::spawn::derive_uid(
                        NodeUid::parse("n_spawn000").expect("a valid uid"),
                        counter,
                    );
                    let id = crate::spawn::derive_uid(seed, prefab.len() as u64);
                    state.spawn_queue.push(crate::spawn::Spawn {
                        template: prefab,
                        at: at.map(|v| v.0).unwrap_or(Vec2Fx::ZERO),
                        parent: parent.map(|h| h.0),
                        id,
                    });
                    Ok(id.to_text())
                },
            )
            .map_err(err)?,
        )
        .map_err(err)?;

    // scene.near(at, radius, tag) -> handles within a radius.
    //
    // Over the broadphase the simulation already builds, rather than
    // walking every node: a homing projectile asking "what is near me" per
    // tick is the difference between a cost in the enemy count and a cost
    // in the product of both counts.
    //
    // Positions are as of the start of the tick, before anything moved.
    // That is a real semantic rather than an accident: every script sees
    // the same world, so what one of them finds does not depend on whether
    // another one has run yet.
    scene
        .set(
            "near",
            lua.create_function(|lua, (at, radius, tag): (LuaVec2, LuaFx, Option<String>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let found = state
                    .query
                    .as_ref()
                    .map(|w| w.within_tagged(at.0, radius.0, tag.as_deref()));
                let mut handles = Vec::new();
                for uid in found.unwrap_or_default() {
                    handles.push(NodeHandle(uid));
                }
                lua.create_sequence_from(handles)
            })
            .map_err(err)?,
        )
        .map_err(err)?;

    // scene.nearest(at, radius, tag) -> the closest one, or nothing.
    scene
        .set(
            "nearest",
            lua.create_function(|lua, (at, radius, tag): (LuaVec2, LuaFx, Option<String>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let Some(world) = state.query.as_ref() else {
                    return Ok(None);
                };
                // The scan walks the index for this tag, so a tagged query
                // never visits a body that could not match — which is what
                // a few hundred homing projectiles asking every tick made
                // expensive.
                // No confirmation against the scene: the index is exact, so
                // a hit is a hit.
                let found = world.nearest(at.0, radius.0, tag.as_deref(), |_| true);
                Ok(found.map(NodeHandle))
            })
            .map_err(err)?,
        )
        .map_err(err)?;

    // A scene the game wants next. Nothing is loaded here: the request is
    // state, the swap happens between ticks in the host, and the tick that
    // asked finishes over the tree it started with (I8). See `crate::load`.
    scene
        .set(
            "request_load",
            lua.create_function(|lua, (path, carry): (String, Option<mlua::Value>)| {
                let state = shared(lua)?;
                let mut state = state.borrow_mut();
                let carry = match carry {
                    Some(v) => from_lua(v)?,
                    None => Value::Map(Default::default()),
                };
                // Last request wins, and a second one in the same tick is
                // reported: two scripts asking for different floors is a bug
                // in the game, and silently taking one of them is how it
                // becomes a bug that only shows up sometimes.
                if let Some(existing) = &state.load_request {
                    if existing.path != path {
                        return Err(mlua::Error::external(Diagnostic::new(
                            Code::SCRIPT_BAD_ARGUMENT,
                            format!(
                                "two scenes requested in one tick: {:?} then {:?}",
                                existing.path, path
                            ),
                        )));
                    }
                }
                state.load_request = Some(crate::load::SceneLoad { path, carry });
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    scene
        .set(
            "carry",
            lua.create_function(|lua, ()| {
                let state = shared(lua)?;
                let state = state.borrow();
                to_lua(lua, &state.carry)
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("scene", scene).map_err(err)?;

    // tick.count and tick.dt
    let tick = lua.create_table().map_err(err)?;
    let rate = tick_rate;
    tick.set(
        "count",
        lua.create_function(|lua, ()| {
            let state = shared(lua)?;
            let t = state.borrow().tick.0;
            Ok(t)
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    tick.set(
        "dt",
        lua.create_function(move |_, ()| Ok(LuaFx(Fx::ONE / rate as i32)))
            .map_err(err)?,
    )
    .map_err(err)?;
    tick.set("rate", rate).map_err(err)?;
    env.set("tick", tick).map_err(err)?;

    // input: what the player is doing this tick.
    //
    // Read-only, and from `SimState` rather than from a device: inside a
    // tick there is no way to tell a gamepad from a replay log, which is
    // most of what makes replay possible (I8).
    let input = lua.create_table().map_err(err)?;
    input
        .set(
            "move",
            lua.create_function(|lua, player: Option<u32>| {
                let state = shared(lua)?;
                let state = state.borrow();
                Ok(LuaVec2(
                    state.input.player(player.unwrap_or(0) as usize).move_dir,
                ))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    input
        .set(
            "aim",
            lua.create_function(|lua, player: Option<u32>| {
                let state = shared(lua)?;
                let state = state.borrow();
                let aim = state.input.player(player.unwrap_or(0) as usize).aim;
                Ok(aim.to_degrees_string())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    input
        .set(
            "aim_vector",
            lua.create_function(|lua, player: Option<u32>| {
                let state = shared(lua)?;
                let state = state.borrow();
                let aim = state.input.player(player.unwrap_or(0) as usize).aim;
                Ok(LuaVec2(aim.to_unit_vector()))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    input
        .set(
            "held",
            lua.create_function(|lua, (button, player): (String, Option<u32>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let bit = button_bit(&button)?;
                Ok(state.input.player(player.unwrap_or(0) as usize).held(bit))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    input
        .set(
            "pressed",
            lua.create_function(|lua, (button, player): (String, Option<u32>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let bit = button_bit(&button)?;
                let index = player.unwrap_or(0) as usize;
                Ok(state
                    .input
                    .player(index)
                    .pressed(&state.previous_input.player(index), bit))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    input
        .set(
            "released",
            lua.create_function(|lua, (button, player): (String, Option<u32>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let bit = button_bit(&button)?;
                let index = player.unwrap_or(0) as usize;
                Ok(state
                    .previous_input
                    .player(index)
                    .pressed(&state.input.player(index), bit))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("input", input).map_err(err)?;

    // ui: what the player is doing to the interface.
    //
    // Everything here reads state the tick computed before scripts ran, so two
    // scripts asking on the same tick get the same answer regardless of which
    // runs first.
    let ui = lua.create_table().map_err(err)?;
    // Called with a node it answers "is this one hovered", and called with
    // nothing it hands back whichever is — the same shape as `pressed` and
    // `clicked`, so a script never has to remember which of the three is the
    // odd one out.
    ui.set(
        "hovered",
        lua.create_function(|lua, node: Option<NodeHandle>| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(match node {
                Some(NodeHandle(uid)) => mlua::Value::Boolean(state.ui.hovered == Some(uid)),
                None => match state.ui.hovered {
                    Some(uid) => mlua::Value::UserData(lua.create_userdata(NodeHandle(uid))?),
                    None => mlua::Value::Nil,
                },
            })
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "pressed",
        lua.create_function(|lua, node: Option<NodeHandle>| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(match node {
                Some(NodeHandle(uid)) => state.ui.pressed == Some(uid),
                None => state.ui.pressed.is_some(),
            })
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "clicked",
        lua.create_function(|lua, node: Option<NodeHandle>| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(match node {
                Some(NodeHandle(uid)) => state.ui.was_clicked(uid),
                None => !state.ui.clicked.is_empty(),
            })
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "captured",
        lua.create_function(|lua, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state.ui.captured)
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "pointer",
        lua.create_function(|lua, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(LuaVec2(crate::ui::pointer(&state.input)))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "focused",
        lua.create_function(|lua, ()| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state.ui.focused.map(NodeHandle))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "focus",
        lua.create_function(|lua, node: Option<NodeHandle>| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            state.ui.focused = node.map(|NodeHandle(uid)| uid);
            Ok(())
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "focus_next",
        lua.create_function(|lua, step: Option<i32>| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            let next = crate::ui::next_focus(
                &state.scene,
                state.canvas,
                state.ui.focused,
                step.unwrap_or(1),
            );
            state.ui.focused = next;
            Ok(next.map(NodeHandle))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    // Text measurement: a pure function of a baked font and a string, both of
    // which the engine already has. It contains no layout policy, which is why
    // it is here while the containers that would use it are not.
    let faces = fonts.clone();
    ui.set(
        "measure",
        lua.create_function(move |lua, (font, text): (String, String)| {
            let (w, h) = crate::text::measure(&faces.borrow(), &font, &text);
            let t = lua.create_table()?;
            t.set("w", w)?;
            t.set("h", h)?;
            Ok(t)
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    ui.set(
        "rect",
        lua.create_function(|lua, NodeHandle(uid): NodeHandle| {
            let state = shared(lua)?;
            let state = state.borrow();
            let rects = dimetric_scene::ui::layout(&state.scene, state.canvas);
            let Some(id) = state.scene.by_uid(uid) else {
                return Ok(None);
            };
            Ok(rects.get(&id).map(|r| {
                let t = lua.create_table()?;
                t.set("x", LuaFx(r.pos.x))?;
                t.set("y", LuaFx(r.pos.y))?;
                t.set("w", LuaFx(r.size.x))?;
                t.set("h", LuaFx(r.size.y))?;
                Ok::<_, mlua::Error>(t)
            }))
            .and_then(|t: Option<Result<mlua::Table, mlua::Error>>| t.transpose())
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    env.set("ui", ui).map_err(err)?;

    // tiles: the grid, read now and written at the end of the tick.
    //
    // Reads need no snapshot because writes are deferred — the grid does not
    // change inside a tick at all, so a read during one is already the grid as
    // it stood when the tick began. See `crate::tiles`.
    let tiles = lua.create_table().map_err(err)?;
    tiles
        .set(
            "get",
            lua.create_function(|lua, (layer, x, y): (NodeHandle, i32, i32)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let uid = tile_layer(&state, layer)?;
                Ok(dimetric_scene::chunk::tile_at(&state.scene.chunks, uid, x, y) as i64)
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    tiles
        .set(
            "set",
            lua.create_function(|lua, (layer, x, y, tile): (NodeHandle, i32, i32, i64)| {
                let state = shared(lua)?;
                let mut state = state.borrow_mut();
                let uid = tile_layer(&state, layer)?;
                let tile = tile_index(tile)?;
                state.tile_queue.push(crate::tiles::TileEdit::Set {
                    layer: uid,
                    x,
                    y,
                    tile,
                });
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    tiles
        .set(
            "fill",
            lua.create_function(
                |lua, (layer, x, y, w, h, tile): (NodeHandle, i32, i32, i32, i32, i64)| {
                    let state = shared(lua)?;
                    let mut state = state.borrow_mut();
                    let uid = tile_layer(&state, layer)?;
                    let tile = tile_index(tile)?;
                    // A negative extent is empty rather than an error: a
                    // generator computing `x1 - x0` for a degenerate room
                    // should get nothing, not a crash.
                    let (w, h) = (w.max(0), h.max(0));
                    let cells = w as i64 * h as i64;
                    if cells > crate::tiles::MAX_FILL_CELLS {
                        return Err(mlua::Error::external(Diagnostic::new(
                            Code::SCRIPT_BAD_ARGUMENT,
                            format!(
                                "tiles.fill covers {cells} cells; the limit is {}",
                                crate::tiles::MAX_FILL_CELLS
                            ),
                        )));
                    }
                    if cells > 0 {
                        state.tile_queue.push(crate::tiles::TileEdit::Fill {
                            layer: uid,
                            rect: [x, y, w, h],
                            tile,
                        });
                    }
                    Ok(())
                },
            )
            .map_err(err)?,
        )
        .map_err(err)?;
    tiles
        .set(
            "bounds",
            lua.create_function(|lua, layer: NodeHandle| {
                let state = shared(lua)?;
                let state = state.borrow();
                let uid = tile_layer(&state, layer)?;
                let Some([x, y, w, h]) =
                    dimetric_scene::chunk::layer_bounds(&state.scene.chunks, uid)
                else {
                    return Ok(None);
                };
                let t = lua.create_table()?;
                t.set("x", x)?;
                t.set("y", y)?;
                t.set("w", w)?;
                t.set("h", h)?;
                Ok(Some(t))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("tiles", tiles).map_err(err)?;

    // profile: what accumulates across runs, and never enters the hash.
    //
    // Not a field on `SimState` at all, for the reason the log lines are not:
    // "kept out entirely" is one fewer thing to get wrong than "a field the
    // hash skips". The hazard this cannot fix — a script branching on a
    // profile value and writing what it reads into state — is documented on
    // `crate::profile` and reported by `dim script check --determinism`.
    let profile_table = lua.create_table().map_err(err)?;
    let store = profile.clone();
    profile_table
        .set(
            "get",
            lua.create_function(move |lua, key: String| match store.borrow().get(&key) {
                Some(v) => to_lua(lua, v),
                None => Ok(mlua::Value::Nil),
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    let store = profile.clone();
    profile_table
        .set(
            "put",
            lua.create_function(move |_, (key, value): (String, mlua::Value)| {
                // Applied immediately rather than deferred to a phase
                // boundary. Deferring exists to stop one script's write
                // changing what another sees *within a tick that is hashed*;
                // nothing here is hashed, so the only thing deferral would buy
                // is the illusion that this is simulation state.
                store.borrow_mut().put(&key, from_lua(value)?);
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    let store = profile.clone();
    profile_table
        .set(
            "clear",
            lua.create_function(move |_, key: String| {
                store.borrow_mut().clear(&key);
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("profile", profile_table).map_err(err)?;

    // event: what the simulation tells the host.
    //
    // Never hashed, and structurally so — there is no field on `SimState` for
    // a later change to start hashing. A rollback re-emits; see
    // `crate::event` for why that is the right contract rather than an
    // accident of where the list lives.
    let event = lua.create_table().map_err(err)?;
    let sink = events.clone();
    event
        .set(
            "emit",
            lua.create_function(move |lua, (kind, payload): (String, Option<mlua::Value>)| {
                if kind.is_empty() {
                    return Err(mlua::Error::external(Diagnostic::new(
                        Code::SCRIPT_BAD_ARGUMENT,
                        "event.emit needs a kind; a host cannot route an unnamed event",
                    )));
                }
                let mut sink = sink.borrow_mut();
                if sink.len() >= crate::event::MAX_EVENTS_PER_TICK {
                    return Err(mlua::Error::external(Diagnostic::new(
                        Code::SCRIPT_BAD_ARGUMENT,
                        format!(
                            "more than {} events in one tick; this is usually a loop \
                                 emitting per entity rather than per happening",
                            crate::event::MAX_EVENTS_PER_TICK
                        ),
                    )));
                }
                let payload = match payload {
                    Some(v) => from_lua(v)?,
                    None => Value::Map(Default::default()),
                };
                let tick = {
                    let state = shared(lua)?;
                    let state = state.borrow();
                    state.tick
                };
                sink.push(crate::event::GameEvent {
                    tick,
                    kind,
                    payload,
                });
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("event", event).map_err(err)?;

    // camera: the view, and the inverse of it.
    //
    // In fixed point off the same `Projection` the renderer draws with, so a
    // picked cell is exact and identical everywhere. A copy of this arithmetic
    // in Lua would drift, and the symptom would be clicks landing one cell off
    // at certain camera positions.
    let camera = lua.create_table().map_err(err)?;
    camera
        .set(
            "to_world",
            lua.create_function(|lua, point: LuaVec2| {
                let state = shared(lua)?;
                let state = state.borrow();
                let view = crate::camera::view_of(&state.scene);
                Ok(LuaVec2(crate::camera::to_world(
                    view,
                    state.canvas,
                    state.resolution,
                    point.0,
                )))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    camera
        .set(
            "to_canvas",
            lua.create_function(|lua, world: LuaVec2| {
                let state = shared(lua)?;
                let state = state.borrow();
                let view = crate::camera::view_of(&state.scene);
                Ok(LuaVec2(crate::camera::to_canvas(
                    view,
                    state.canvas,
                    state.resolution,
                    world.0,
                )))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    camera
        .set(
            "center",
            lua.create_function(|lua, ()| {
                let state = shared(lua)?;
                let state = state.borrow();
                Ok(LuaVec2(crate::camera::view_of(&state.scene).center))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("camera", camera).map_err(err)?;

    // tween: cosmetic motion, measured in ticks like everything else.
    let tween = lua.create_table().map_err(err)?;
    tween
        .set(
            "to",
            lua.create_function(
                |lua,
                 (node, property, target, ticks, easing): (
                    NodeHandle,
                    String,
                    mlua::Value,
                    u32,
                    Option<String>,
                )| {
                    let easing = match easing.as_deref() {
                        None => crate::tween::Easing::Linear,
                        Some(name) => crate::tween::Easing::parse(name).ok_or_else(|| {
                            mlua::Error::runtime(format!(
                                "no easing called {name:?}; \
                                 try linear, ease_in, ease_out or ease_in_out"
                            ))
                        })?,
                    };
                    let target = from_lua(target)?;
                    let state = shared(lua)?;
                    let mut state = state.borrow_mut();
                    let id = resolve(&state, node.0)?;
                    let from = read_property(&state, id, &property).ok_or_else(|| {
                        mlua::Error::runtime(format!(
                            "{property:?} is not a property of this node, \
                             so there is nothing to tween from"
                        ))
                    })?;
                    if !crate::tween::can_tween(&from, &target) {
                        return Err(mlua::Error::runtime(format!(
                            "{property:?} is a {}, and the target is a {}",
                            from.type_name(),
                            target.type_name()
                        )));
                    }
                    // Starting a second tween on a property replaces the
                    // first. Two tweens fighting over one number is never
                    // what anybody meant.
                    let list = state.tweens.entry(node.0).or_default();
                    list.retain(|t| t.property != property);
                    list.push(crate::tween::Tween {
                        property,
                        from,
                        to: target,
                        elapsed: 0,
                        ticks,
                        easing,
                    });
                    Ok(())
                },
            )
            .map_err(err)?,
        )
        .map_err(err)?;
    tween
        .set(
            "cancel",
            lua.create_function(|lua, (node, property): (NodeHandle, Option<String>)| {
                let state = shared(lua)?;
                let mut state = state.borrow_mut();
                match property {
                    Some(property) => {
                        if let Some(list) = state.tweens.get_mut(&node.0) {
                            list.retain(|t| t.property != property);
                            if list.is_empty() {
                                state.tweens.remove(&node.0);
                            }
                        }
                    }
                    None => {
                        state.tweens.remove(&node.0);
                    }
                }
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    tween
        .set(
            "running",
            lua.create_function(|lua, (node, property): (NodeHandle, Option<String>)| {
                let state = shared(lua)?;
                let state = state.borrow();
                let Some(list) = state.tweens.get(&node.0) else {
                    return Ok(false);
                };
                Ok(match property {
                    Some(property) => list.iter().any(|t| t.property == property),
                    None => !list.is_empty(),
                })
            })
            .map_err(err)?,
        )
        .map_err(err)?;
    env.set("tween", tween).map_err(err)?;

    // anim: frame playback over the clips the importer produced.
    let anim = lua.create_table().map_err(err)?;
    anim.set(
        "play",
        lua.create_function(|lua, (node, clip): (NodeHandle, String)| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            let entry = state
                .anim
                .entry(node.0)
                .or_insert_with(|| crate::state::AnimState {
                    clip: clip.clone(),
                    frame: 0,
                    ticks_in_frame: 0,
                    playing: true,
                    finished: false,
                });
            // Playing the clip that is already playing does not restart it,
            // so `anim.play(self, "walk")` every tick is harmless.
            if entry.clip != clip {
                entry.clip = clip;
                entry.frame = 0;
                entry.ticks_in_frame = 0;
            }
            entry.playing = true;
            entry.finished = false;
            Ok(())
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    anim.set(
        "stop",
        lua.create_function(|lua, node: NodeHandle| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            if let Some(entry) = state.anim.get_mut(&node.0) {
                entry.playing = false;
            }
            Ok(())
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    anim.set(
        "frame",
        lua.create_function(|lua, node: NodeHandle| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state.anim.get(&node.0).map(|a| a.frame))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    anim.set(
        "playing",
        lua.create_function(|lua, node: NodeHandle| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state
                .anim
                .get(&node.0)
                .map(|a| a.playing && !a.finished)
                .unwrap_or(false))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    anim.set(
        "finished",
        lua.create_function(|lua, node: NodeHandle| {
            let state = shared(lua)?;
            let state = state.borrow();
            Ok(state.anim.get(&node.0).map(|a| a.finished).unwrap_or(false))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    env.set("anim", anim).map_err(err)?;

    // rng: named streams only. There is no unseeded path.
    let rng = lua.create_table().map_err(err)?;
    rng.set(
        "range",
        lua.create_function(|lua, (stream, lo, hi): (String, i32, i32)| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            Ok(state.rng.stream(&stream).range_i32(lo, hi))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    rng.set(
        "chance",
        lua.create_function(|lua, (stream, n, d): (String, u32, u32)| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            Ok(state.rng.stream(&stream).chance(n, d))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    rng.set(
        "unit",
        lua.create_function(|lua, stream: String| {
            let state = shared(lua)?;
            let mut state = state.borrow_mut();
            Ok(LuaFx(state.rng.stream(&stream).unit_fx()))
        })
        .map_err(err)?,
    )
    .map_err(err)?;
    env.set("rng", rng).map_err(err)?;

    // log: collected, never printed from inside a tick. Printing from in here
    // would interleave with whatever the host is writing and would happen on a
    // rolled-back tick as readily as a kept one; the runner decides what to do
    // with the lines once the tick is over.
    let log_table = lua.create_table().map_err(err)?;
    for level in ["info", "warn", "error"] {
        let sink = log.clone();
        let source = path.to_string();
        log_table
            .set(
                level,
                lua.create_function(move |_, args: Variadic<String>| {
                    sink.borrow_mut()
                        .push(format!("{level}: {source}: {}", args.join(" ")));
                    Ok(())
                })
                .map_err(err)?,
            )
            .map_err(err)?;
    }
    env.set("log", log_table).map_err(err)?;

    // require(path): another script's returned table, evaluated once.
    let handles = shared_state.clone();
    let require = lua
        .create_function(move |lua, name: String| require_module(lua, tick_rate, &handles, &name))
        .map_err(err)?;
    env.set("require", require).map_err(err)?;

    Ok(())
}

/// Evaluate a module, or hand back the one already evaluated.
///
/// A module is an ordinary project script that ends in a `return`, required by
/// the project-relative path a scene would write after `script:` — so
/// `require("scripts/spellbook.lua")`, with the extension, and one spelling
/// rather than several to guess between.
///
/// What comes back is frozen. A module's table is not part of `SimState`: it is
/// not hashed, not snapshotted, and not rewound. Anything written into it would
/// survive a rollback that rewound everything around it, which is a divergence
/// that shows up hours later in a replay rather than at the write. So the engine
/// refuses the write instead — a module holds constants and pure functions, and
/// state lives in node variables, where the hash can see it.
fn require_module(
    lua: &Lua,
    tick_rate: u32,
    shared_state: &HostHandles,
    name: &str,
) -> mlua::Result<mlua::Value> {
    let modules = &shared_state.modules;
    if let Some(cached) = modules.borrow().cache.get(name).cloned() {
        return Ok(cached);
    }
    let source = {
        let m = modules.borrow();
        if let Some(at) = m.loading.iter().position(|p| p == name) {
            let mut chain: Vec<&str> = m.loading[at..].iter().map(String::as_str).collect();
            chain.push(name);
            return Err(mlua::Error::runtime(format!(
                "module cycle: {}",
                chain.join(" -> ")
            )));
        }
        match m.sources.get(name) {
            Some(source) => source.clone(),
            None => {
                return Err(mlua::Error::runtime(format!(
                    "unknown module {name:?}; require takes a project-relative \
                     script path, such as \"scripts/spellbook.lua\""
                )));
            }
        }
    };

    modules.borrow_mut().loading.push(name.to_string());
    let modules = &shared_state.modules;
    let evaluated = build_environment(lua, tick_rate, shared_state, name)
        .map_err(|d| mlua::Error::runtime(d.message))
        .and_then(|env| {
            lua.load(&source)
                .set_name(name)
                .set_environment(env)
                .eval::<mlua::Value>()
        });
    modules.borrow_mut().loading.pop();

    let value = freeze(lua, evaluated?)?;
    modules
        .borrow_mut()
        .cache
        .insert(name.to_string(), value.clone());
    Ok(value)
}

/// Make a value read-only, all the way down.
///
/// A metatable on the table itself would not do it: `__newindex` fires only for
/// a key that is *absent*, so `spells.bolt = nil` — overwriting something the
/// module actually defines, which is the write worth stopping — would go
/// straight through. What comes back instead is an empty proxy whose metatable
/// forwards reads to a private copy. Every key is absent from the proxy, so
/// every write reaches the guard. `rawset` would still walk past it, which is
/// why the sandbox does not have `rawset`.
///
/// This catches writes through the table. It cannot catch a module function
/// that closes over a local and mutates that — Lua upvalues are not reachable
/// from here. A module that does so is holding simulation state outside the
/// hash, and no amount of freezing would tell you; that one is on whoever
/// writes it, which is why the rule is "constants and pure functions" rather
/// than "whatever the engine lets you get away with".
fn freeze(lua: &Lua, value: mlua::Value) -> mlua::Result<mlua::Value> {
    freeze_into(lua, value, &mut Vec::new())
}

/// `seen` maps a source table to the proxy already made for it, so a module
/// that refers to itself terminates and comes out with one proxy rather than an
/// infinite regress of them. It is a list because a module has a handful of
/// tables in it, not thousands.
fn freeze_into(
    lua: &Lua,
    value: mlua::Value,
    seen: &mut Vec<(*const std::ffi::c_void, Table)>,
) -> mlua::Result<mlua::Value> {
    let mlua::Value::Table(source) = &value else {
        return Ok(value);
    };
    if is_frozen(source)? {
        return Ok(value);
    }
    let key = source.to_pointer();
    if let Some((_, proxy)) = seen.iter().find(|(p, _)| *p == key) {
        return Ok(mlua::Value::Table(proxy.clone()));
    }

    let backing = lua.create_table()?;
    let proxy = lua.create_table()?;
    seen.push((key, proxy.clone()));

    let guard = lua.create_table()?;
    guard.set("__index", backing.clone())?;
    guard.set(
        "__newindex",
        lua.create_function(|_, (_, key): (Table, mlua::Value)| -> mlua::Result<()> {
            Err(mlua::Error::runtime(format!(
                "a module is read-only, and {} cannot be assigned: a module is \
                 not simulation state, so a write here would not be hashed and \
                 would survive a rollback that rewound everything around it",
                describe_key(&key)
            )))
        })?,
    )?;
    let len_of = backing.clone();
    guard.set(
        "__len",
        lua.create_function(move |_, _: Table| Ok(len_of.raw_len()))?,
    )?;
    let pairs_of = backing.clone();
    guard.set(
        "__pairs",
        lua.create_function(move |lua, _: Table| {
            Ok((
                lua.globals().get::<mlua::Value>("next")?,
                pairs_of.clone(),
                mlua::Value::Nil,
            ))
        })?,
    )?;
    guard.set("__metatable", FROZEN)?;
    proxy.set_metatable(Some(guard));

    let entries: Vec<(mlua::Value, mlua::Value)> =
        source.pairs().collect::<mlua::Result<Vec<_>>>()?;
    for (k, v) in entries {
        let frozen = freeze_into(lua, v, seen)?;
        backing.raw_set(k, frozen)?;
    }
    Ok(mlua::Value::Table(proxy))
}

/// True when this table is already a module proxy.
fn is_frozen(table: &Table) -> mlua::Result<bool> {
    match table.metatable() {
        Some(meta) => Ok(meta.get::<Option<String>>("__metatable")?.as_deref() == Some(FROZEN)),
        None => Ok(false),
    }
}

/// Name a rejected key the way the script wrote it.
fn describe_key(key: &mlua::Value) -> String {
    match key {
        mlua::Value::String(s) => match s.to_str() {
            Ok(s) => format!("`{s}`"),
            Err(_) => "that key".to_string(),
        },
        mlua::Value::Integer(i) => format!("index {i}"),
        other => format!("a {} key", other.type_name()),
    }
}

impl ScriptHost for LuaHost {
    fn dispatch(
        &mut self,
        state: &Shared,
        node: NodeUid,
        script: &str,
        hook: &Hook,
    ) -> Result<(), Diagnostic> {
        let Some(env) = self.scripts.get(script).cloned() else {
            return Ok(());
        };
        let name = hook.function_name();
        let Ok(function) = env.get::<mlua::Function>(name) else {
            // A script need not implement every hook.
            return Ok(());
        };

        // The state is handed to Lua for the duration of one call and taken
        // back immediately, so nothing can hold a reference across a tick.
        self.lua.set_app_data(state.clone());
        let handle = NodeHandle(node);
        let result = match hook {
            Hook::Collide {
                other,
                normal,
                trigger,
            } => {
                let mut args = MultiValue::new();
                args.push_back(mlua::Value::UserData(
                    self.lua
                        .create_userdata(handle)
                        .map_err(|e| runtime_error(script, e))?,
                ));
                args.push_back(mlua::Value::UserData(
                    self.lua
                        .create_userdata(NodeHandle(*other))
                        .map_err(|e| runtime_error(script, e))?,
                ));
                args.push_back(mlua::Value::UserData(
                    self.lua
                        .create_userdata(LuaVec2(*normal))
                        .map_err(|e| runtime_error(script, e))?,
                ));
                args.push_back(mlua::Value::Boolean(*trigger));
                function.call::<mlua::Value>(args)
            }
            Hook::Signal {
                name,
                from,
                payload,
                ..
            } => {
                let table = self
                    .lua
                    .create_table()
                    .map_err(|e| runtime_error(script, e))?;
                for (k, v) in payload {
                    let value = to_lua(&self.lua, v).map_err(|e| runtime_error(script, e))?;
                    table
                        .set(k.as_str(), value)
                        .map_err(|e| runtime_error(script, e))?;
                }
                function.call::<mlua::Value>((handle, NodeHandle(*from), name.as_str(), table))
            }
            Hook::AnimEvent { event } => function.call::<mlua::Value>((handle, event.as_str())),
            _ => function.call::<mlua::Value>(handle),
        };
        self.lua.remove_app_data::<Shared>();

        result.map(|_| ()).map_err(|e| runtime_error(script, e))
    }

    fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn take_events(&mut self) -> Vec<crate::event::GameEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn reload(&mut self, path: &str, source: &str) -> Result<(), Diagnostic> {
        // A fresh environment, so a function the new source deleted is gone
        // rather than lingering from the old one. Node variables are untouched:
        // they live in `SimState`, not in here.
        //
        // Every *other* script is re-run too, and the module cache is dropped.
        // A script that required this path holds the table it returned, and
        // reloading only the file that changed would leave it reading last
        // version's constants — a hot reload that appears to do nothing, which
        // is worse than one that does not work. Which scripts those are is not
        // tracked, because re-running a project's scripts is a keystroke's
        // worth of work on a save and a dependency graph is a thing to get
        // wrong.
        {
            let mut modules = self.modules.borrow_mut();
            modules.sources.insert(path.to_string(), source.to_string());
            modules.cache.clear();
        }
        let sources: Vec<(String, String)> = {
            let modules = self.modules.borrow();
            let mut paths: Vec<&str> = self.scripts.keys().map(String::as_str).collect();
            if !self.scripts.contains_key(path) {
                paths.push(path);
            }
            paths
                .into_iter()
                .filter_map(|p| modules.sources.get(p).map(|s| (p.to_string(), s.clone())))
                .collect()
        };
        let mut first = None;
        for (p, source) in sources {
            if let Err(d) = self.load(&p, &source) {
                first.get_or_insert(d);
            }
        }
        match first {
            Some(d) => Err(d),
            None => Ok(()),
        }
    }
}

fn syntax_error(path: &str, e: mlua::Error) -> Diagnostic {
    Diagnostic::new(Code::SCRIPT_SYNTAX, e.to_string())
        .with_span(dimetric_core::Span::file(path))
        .with_field("detail", e.to_string())
}

fn runtime_error(path: &str, e: mlua::Error) -> Diagnostic {
    let code = match &e {
        mlua::Error::SyntaxError { .. } => Code::SCRIPT_SYNTAX,
        _ => Code::SCRIPT_RUNTIME,
    };
    Diagnostic::new(code, e.to_string())
        .with_span(dimetric_core::Span::file(path))
        .with_field("detail", e.to_string())
}

/// Read a property for a tween to start from, reserved keys included.
fn read_property(state: &SimState, id: dimetric_core::NodeId, property: &str) -> Option<Value> {
    let node = state.scene.get(id)?;
    Some(match property {
        "pos" => Value::Vec2(node.transform.pos),
        "scale" => Value::Vec2(node.transform.scale),
        "rot" => Value::Angle(node.transform.rot),
        other => node.get(other)?.clone(),
    })
}

/// The bit a button's name refers to.
///
/// Named rather than numbered, so a script says `input.held("fire")` and the
/// engine keeps the bit layout to itself.
fn button_bit(name: &str) -> mlua::Result<u32> {
    use crate::input::buttons;
    Ok(match name {
        "fire" => buttons::FIRE,
        "alt" => buttons::ALT,
        "dash" => buttons::DASH,
        "use" => buttons::USE,
        "pause" => buttons::PAUSE,
        other => {
            return Err(mlua::Error::runtime(format!(
                "no button called {other:?}; try fire, alt, dash, use or pause"
            )))
        }
    })
}
