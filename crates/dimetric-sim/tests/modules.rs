//! What a script can share with another script, and what it must not.
//!
//! The sandbox had no `require`, so two scripts could not share a table and the
//! sorcerer slice published its spell definitions as variables on a node. That
//! worked, and it put constants somewhere the state hash had to carry them.
//!
//! A module is the other answer, and it comes with a constraint the node did
//! not need: a module's table is not simulation state. It is not hashed, not
//! snapshotted, and not rewound — so a module that could be written to would be
//! a place for state to hide from a rollback. These tests pin that it cannot.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{InputLog, LuaHost, Sim, SimConfig};

const ROOM: &str = r#"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Room"
script = "script:scripts/room.lua"
"#;

fn scene() -> Scene {
    let out = dimetric_scene::parse(ROOM, "test.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

/// Load a script set, step it once, and report either the first thing that
/// went wrong or the variable the room script published.
fn run(scripts: &[(&str, &str)]) -> Result<String, String> {
    let mut host = LuaHost::new(60).map_err(|d| d.message.clone())?;
    let diags = host.load_all(scripts.iter().copied());
    if let Some(d) = diags.first() {
        return Err(d.message.clone());
    }
    let mut sim = Sim::new(scene(), 7, Box::new(host), SimConfig::default());
    let log = InputLog::new(7, "test", 1);
    sim.step(log.frame(0));
    if let Some(d) = sim.diagnostics().iter().next() {
        return Err(d.message.clone());
    }
    let state = sim.state();
    Ok(state
        .vars
        .get(&dimetric_core::NodeUid::parse("n_root0000").unwrap())
        .and_then(|v| v.get("out"))
        .map(|v| format!("{v:?}"))
        .unwrap_or_else(|| "<unset>".into()))
}

const SPELLS: &str = r#"
return {
  bolt = { damage = 12, speed = 150 },
  order = { "bolt", "nova" },
  scale = function(n, pct) return n * pct // 100 end,
}
"#;

#[test]
fn a_script_can_require_another() {
    let out = run(&[
        (
            "scripts/room.lua",
            r#"
local spells = require("scripts/spells.lua")
function on_ready(self)
  self.out = spells.bolt.damage .. "/" .. spells.order[1] .. "/" .. spells.scale(10, 150)
end
"#,
        ),
        ("scripts/spells.lua", SPELLS),
    ])
    .expect("the script set loads and runs");
    assert_eq!(out, r#"Str("12/bolt/15")"#);
}

#[test]
fn load_order_does_not_decide_whether_require_works() {
    // `scripts/arena.lua` sorts before `scripts/spells.lua`, which is exactly
    // the case that would break if sources were registered as each ran.
    let scripts = [
        ("scripts/arena.lua", "local s = require(\"scripts/spells.lua\")\nfunction on_ready(self) self.out = s.bolt.speed end\n"),
        ("scripts/spells.lua", SPELLS),
    ];
    let mut host = LuaHost::new(60).expect("lua");
    assert!(host.load_all(scripts.iter().copied()).is_empty());
}

#[test]
fn a_module_is_evaluated_once() {
    // The module counts its own evaluations in a local. Two requires, one run.
    let out = run(&[
        (
            "scripts/room.lua",
            r#"
local a = require("scripts/counter.lua")
local b = require("scripts/counter.lua")
function on_ready(self)
  self.out = a.runs .. "/" .. b.runs .. "/" .. tostring(a == b)
end
"#,
        ),
        (
            "scripts/counter.lua",
            "RUNS = (RUNS or 0) + 1\nreturn { runs = RUNS }\n",
        ),
    ])
    .expect("runs");
    assert_eq!(out, r#"Str("1/1/true")"#);
}

#[test]
fn a_module_cannot_be_written_to() {
    let err = run(&[
        (
            "scripts/room.lua",
            r#"
local spells = require("scripts/spells.lua")
function on_ready(self) spells.bolt = nil end
"#,
        ),
        ("scripts/spells.lua", SPELLS),
    ])
    .expect_err("assigning into a module is refused");
    assert!(err.contains("read-only"), "{err}");
    assert!(err.contains("`bolt`"), "{err}");
}

#[test]
fn the_freeze_reaches_nested_tables() {
    let err = run(&[
        (
            "scripts/room.lua",
            r#"
local spells = require("scripts/spells.lua")
function on_ready(self) spells.bolt.damage = 9999 end
"#,
        ),
        ("scripts/spells.lua", SPELLS),
    ])
    .expect_err("a nested table is frozen too");
    assert!(err.contains("read-only"), "{err}");
    assert!(err.contains("`damage`"), "{err}");
}

#[test]
fn a_module_list_cannot_be_appended_to() {
    let err = run(&[
        (
            "scripts/room.lua",
            r#"
local spells = require("scripts/spells.lua")
function on_ready(self) table.insert(spells.order, "chain") end
"#,
        ),
        ("scripts/spells.lua", SPELLS),
    ])
    .expect_err("table.insert respects the guard");
    assert!(err.contains("read-only"), "{err}");
}

#[test]
fn the_guard_cannot_be_removed() {
    // `__metatable` does two jobs: `setmetatable` refuses to replace a
    // protected one, and `getmetatable` hands back the marker string rather
    // than the guard, so there is nothing to reach in and edit.
    for attempt in [
        "setmetatable(spells, {})",
        "local m = getmetatable(spells) m.__newindex = nil",
        "getmetatable(spells).__index.bolt = nil",
    ] {
        let source = format!(
            "local spells = require(\"scripts/spells.lua\")\nfunction on_ready(self) {attempt} end\n"
        );
        let err = run(&[
            ("scripts/room.lua", &source),
            ("scripts/spells.lua", SPELLS),
        ])
        .expect_err("the metatable is protected");
        assert!(
            err.contains("protected metatable") || err.contains("attempt to index a"),
            "{attempt}: {err}"
        );
    }
}

#[test]
fn a_frozen_table_still_reads_like_a_table() {
    // Length and iteration go through the proxy, so a module list is a list.
    let out = run(&[
        (
            "scripts/room.lua",
            r#"
local spells = require("scripts/spells.lua")
function on_ready(self)
  local keys = {}
  for k in pairs(spells.bolt) do keys[#keys + 1] = k end
  table.sort(keys)
  self.out = #spells.order .. "/" .. spells.order[2] .. "/" .. table.concat(keys, ",")
end
"#,
        ),
        ("scripts/spells.lua", SPELLS),
    ])
    .expect("runs");
    assert_eq!(out, r#"Str("2/nova/damage,speed")"#);
}

#[test]
fn a_module_can_require_a_module() {
    let out = run(&[
        ("scripts/room.lua", "local t = require(\"scripts/top.lua\")\nfunction on_ready(self) self.out = t.deep end\n"),
        ("scripts/top.lua", "local b = require(\"scripts/base.lua\")\nreturn { deep = b.value * 2 }\n"),
        ("scripts/base.lua", "return { value = 21 }\n"),
    ])
    .expect("runs");
    assert_eq!(out, "Int(42)");
}

#[test]
fn a_cycle_is_an_error_naming_the_loop() {
    let err = run(&[
        (
            "scripts/room.lua",
            "require(\"scripts/a.lua\")\nfunction on_ready(self) end\n",
        ),
        ("scripts/a.lua", "require(\"scripts/b.lua\")\nreturn {}\n"),
        ("scripts/b.lua", "require(\"scripts/a.lua\")\nreturn {}\n"),
    ])
    .expect_err("a cycle does not recurse forever");
    assert!(err.contains("module cycle"), "{err}");
    assert!(
        err.contains("scripts/a.lua -> scripts/b.lua -> scripts/a.lua"),
        "{err}"
    );
}

#[test]
fn an_unknown_module_says_what_a_path_looks_like() {
    let err = run(&[(
        "scripts/room.lua",
        "require(\"spells\")\nfunction on_ready(self) end\n",
    )])
    .expect_err("a bare name is not a module path");
    assert!(err.contains("unknown module"), "{err}");
    assert!(err.contains("scripts/spellbook.lua"), "{err}");
}

#[test]
fn rawset_cannot_walk_past_the_guard() {
    let err = run(&[(
        "scripts/room.lua",
        "function on_ready(self) rawset({}, 1, 1) end\n",
    )])
    .expect_err("rawset is not in the sandbox");
    assert!(err.contains("rawset"), "{err}");
}

/// Reloading a module has to reach the scripts that required it.
///
/// This is the cost the gap note was worried about: a script holds the table a
/// module returned, so reloading only the file that changed leaves it reading
/// the old constants — a hot reload that silently does nothing, which is worse
/// than one that fails.
#[test]
fn reloading_a_module_reaches_the_scripts_that_required_it() {
    let room = r#"
local tuning = require("scripts/tuning.lua")
function on_tick(self) self.out = tuning.damage end
"#;
    let mut host = LuaHost::new(60).expect("lua");
    let diags = host.load_all([
        ("scripts/room.lua", room),
        ("scripts/tuning.lua", "return { damage = 12 }\n"),
    ]);
    assert!(diags.is_empty(), "{diags:?}");

    let mut sim = Sim::new(scene(), 7, Box::new(host), SimConfig::default());
    let log = InputLog::new(7, "test", 1);
    sim.step(log.frame(0));
    assert_eq!(read_out(&sim), "Int(12)");

    // Only the module changes. The script that reads it is untouched on disk.
    sim.scripts_mut()
        .reload("scripts/tuning.lua", "return { damage = 30 }\n")
        .expect("the reload succeeds");
    sim.step(log.frame(1));
    assert_eq!(read_out(&sim), "Int(30)");
}

fn read_out(sim: &Sim) -> String {
    sim.state()
        .vars
        .get(&dimetric_core::NodeUid::parse("n_root0000").unwrap())
        .and_then(|v| v.get("out"))
        .map(|v| format!("{v:?}"))
        .unwrap_or_else(|| "<unset>".into())
}

/// `log.info` used to build a string and drop it on the floor.
///
/// The API was documented, the buffer to hold the lines existed, and nothing
/// connected them — so a script's diagnostics went nowhere and the slice, which
/// never called `log` at all, never noticed.
#[test]
fn a_script_can_log() {
    let mut host = LuaHost::new(60).expect("lua");
    let diags = host.load_all([(
        "scripts/room.lua",
        "function on_tick(self) log.info(\"tick\", tick.count()) log.warn(\"careful\") end\n",
    )]);
    assert!(diags.is_empty(), "{diags:?}");

    let mut sim = Sim::new(scene(), 7, Box::new(host), SimConfig::default());
    let log = InputLog::new(7, "test", 1);
    sim.step(log.frame(0));
    assert_eq!(
        sim.take_log(),
        vec![
            "info: scripts/room.lua: tick 0".to_string(),
            "warn: scripts/room.lua: careful".to_string(),
        ]
    );

    // Taken means taken: the next tick's lines are the next tick's.
    sim.step(log.frame(1));
    assert_eq!(sim.take_log().len(), 2);
    assert!(sim.take_log().is_empty());
}

/// Logging must not be able to change the game.
#[test]
fn logging_does_not_reach_the_state_hash() {
    let quiet = "function on_tick(self) self.n = (self.n or 0) + rng.range(\"x\", 1, 6) end\n";
    let loud = "function on_tick(self)\n  log.info(\"n is\", self.n or 0)\n  self.n = (self.n or 0) + rng.range(\"x\", 1, 6)\nend\n";

    let hash_of = |source: &str| {
        let mut host = LuaHost::new(60).expect("lua");
        assert!(host.load_all([("scripts/room.lua", source)]).is_empty());
        let mut sim = Sim::new(scene(), 7, Box::new(host), SimConfig::default());
        let log = InputLog::new(7, "test", 1);
        for tick in 0..20 {
            sim.step(log.frame(tick));
            sim.take_log();
        }
        sim.hash()
    };
    assert_eq!(hash_of(quiet), hash_of(loud));
}
