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
        mlua::Value::Table(t) => {
            let mut map = IndexMap::new();
            let mut pairs: Vec<(String, mlua::Value)> = t
                .pairs::<String, mlua::Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in pairs {
                map.insert(k, from_lua(v)?);
            }
            Value::Map(map)
        }
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
    /// Ticks per second, for `tick.dt`.
    tick_rate: u32,
    /// Messages scripts wrote, in order.
    pub log: Vec<String>,
}

impl LuaHost {
    /// Create a host with the sandbox installed.
    pub fn new(tick_rate: u32) -> Result<LuaHost, Diagnostic> {
        let lua = Lua::new();
        Ok(LuaHost {
            lua,
            scripts: BTreeMap::new(),
            tick_rate,
            log: Vec::new(),
        })
    }

    /// Load a script under a project-relative path.
    pub fn load(&mut self, path: &str, source: &str) -> Result<(), Diagnostic> {
        let env = self.build_environment(path)?;
        self.lua
            .load(source)
            .set_name(path)
            .set_environment(env.clone())
            .exec()
            .map_err(|e| syntax_error(path, e))?;
        self.scripts.insert(path.to_string(), env);
        Ok(())
    }

    /// True when a script has been loaded.
    pub fn has(&self, path: &str) -> bool {
        self.scripts.contains_key(path)
    }

    /// The sandbox: exactly what a script may reach, and nothing else.
    ///
    /// `os`, `io`, `require`, `dofile`, `load` and `package` are absent — a
    /// script that could read the clock or the filesystem could break
    /// determinism without the engine ever knowing. `math.random` is gone
    /// because randomness must come from a seeded stream (I6), and the
    /// transcendental functions are gone because platform `libm` does not agree
    /// with itself across operating systems.
    fn build_environment(&self, path: &str) -> Result<Table, Diagnostic> {
        let lua = &self.lua;
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
            "unpack",
            "xpcall",
            "rawequal",
            "rawget",
            "rawset",
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
        self.install_api(&env, path)?;
        Ok(env)
    }

    fn install_api(&self, env: &Table, path: &str) -> Result<(), Diagnostic> {
        let lua = &self.lua;
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
                    let uid =
                        NodeUid::parse(&id).map_err(|e| mlua::Error::runtime(e.to_string()))?;
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
        env.set("scene", scene).map_err(err)?;

        // tick.count and tick.dt
        let tick = lua.create_table().map_err(err)?;
        let rate = self.tick_rate;
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

        // log: collected, never printed from inside a tick.
        let log = lua.create_table().map_err(err)?;
        for level in ["info", "warn", "error"] {
            log.set(
                level,
                lua.create_function(move |_, args: Variadic<String>| {
                    Ok(format!("{}: {}", level, args.join(" ")))
                })
                .map_err(err)?,
            )
            .map_err(err)?;
        }
        env.set("log", log).map_err(err)?;

        Ok(())
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
