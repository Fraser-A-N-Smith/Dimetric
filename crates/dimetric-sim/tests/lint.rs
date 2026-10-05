//! The Lua determinism lint.
//!
//! `cargo xtask lint-sim` holds the engine's Rust to I3 and I4 and gates it in
//! CI. Game logic is Lua, and it was held to the same invariants by nothing at
//! all. The argument `CONTRIBUTING.md` makes for the Rust lints applies word
//! for word: these do not fail loudly, they fail three weeks later as a replay
//! that diverges on someone else's machine.

use dimetric_sim::lint::check;

fn hazards(src: &str) -> Vec<String> {
    check("t.lua", src)
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

fn count(src: &str) -> usize {
    check("t.lua", src).len()
}

#[test]
fn pairs_is_flagged() {
    // Lua's hash iteration order is unspecified, and `ENGINE-GAPS.md` already
    // records this as why OFFER_ORDER exists in the slice.
    let found = hazards("for k, v in pairs(self.spells) do self.mana = 1 end");
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("pairs"), "{found:?}");
}

#[test]
fn ipairs_is_not() {
    // The whole point: an array walked in order is reproducible, and a lint
    // that flagged the fix as well as the bug would be useless.
    assert_eq!(
        count("for i, v in ipairs(self.order) do self.mana = 1 end"),
        0
    );
}

#[test]
fn a_name_that_merely_ends_in_pairs_is_not_flagged() {
    assert_eq!(count("local x = my_pairs(t)"), 0);
    assert_eq!(count("local x = self.pairs_left"), 0);
}

#[test]
fn an_acknowledged_pairs_is_allowed() {
    // The escape hatch. A conservative lint needs one, or it gets turned off.
    assert_eq!(
        count("for k, v in pairs(t) do total = total + 1 end -- @ordered"),
        0
    );
}

#[test]
fn a_fractional_literal_is_flagged() {
    let found = hazards("self.speed = self.speed * 1.5");
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("1.5"), "{found:?}");
}

#[test]
fn a_whole_number_is_not() {
    // `self.hp = 20` is exact. Flagging it would make the lint noise, and
    // noise is how a lint stops being read.
    assert_eq!(count("self.hp = 20"), 0);
    assert_eq!(count("self.hp = self.hp - 3"), 0);
}

#[test]
fn a_number_inside_a_string_is_not_flagged() {
    // The false positive that mattered: `fx.parse(\"0.1\")` is the *correct*
    // way to get an exact tenth, and the engine's own example project uses
    // `fx.from_angle(\"0.0\")`. A lint that fires on the right answer trains
    // people to ignore it.
    assert_eq!(count("return fx.from_angle(self.aim or \"0.0\")"), 0);
    assert_eq!(count("local a = fx.parse(\"1.5\")"), 0);
}

#[test]
fn a_number_inside_an_identifier_is_not_flagged() {
    assert_eq!(count("local v = vec2(a, b)"), 0);
    assert_eq!(count("self.pos = vec2(fx.new(1), fx.new(0))"), 0);
}

#[test]
fn a_profile_read_reaching_a_state_write_is_flagged() {
    // The hazard keeping the profile out of the hash cannot fix: two players
    // with different unlocks would run different simulations.
    let found = hazards("self.spell = profile.get(\"knows_fire\")");
    assert_eq!(found.len(), 1);
    assert!(found[0].contains("profile"), "{found:?}");
}

#[test]
fn a_profile_read_reaches_state_through_a_local() {
    // The form the one-line check missed, and the form anybody would write.
    // Naming a value before using it is the normal way to write Lua, so the
    // hazard that slipped through was the *ordinary* spelling of the mistake.
    let found = hazards(
        "local known = profile.get(\"knows_fire\")\n\
         self.spell = known\n",
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("profile"), "{found:?}");
}

#[test]
fn a_profile_taint_survives_intervening_lines() {
    let found = hazards(
        "local known = profile.get(\"seen\")\n\
         local other = 1\n\
         log.info(\"thinking\")\n\
         self.spell = known\n",
    );
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn a_name_that_merely_contains_a_tainted_one_is_not_flagged() {
    // `known` is tainted; `unknown` and `knownish` are different names, and a
    // substring match would have flagged both.
    assert_eq!(
        count(
            "local known = profile.get(\"seen\")\n\
             self.spell = unknown\n"
        ),
        0
    );
    assert_eq!(
        count(
            "local known = profile.get(\"seen\")\n\
             self.spell = knownish\n"
        ),
        0
    );
}

#[test]
fn a_profile_read_that_goes_nowhere_near_state_is_not() {
    assert_eq!(count("local unlocked = profile.get(\"knows_fire\")"), 0);
    // Tainting a local is not itself a hazard: the local has to reach state.
    assert_eq!(
        count(
            "local unlocked = profile.get(\"knows_fire\")\n\
             log.info(tostring(unlocked))\n"
        ),
        0
    );
    assert_eq!(
        count("if profile.get(\"seen\") then log.info(\"hi\") end"),
        0
    );
}

#[test]
fn a_comparison_is_not_an_assignment() {
    // `==`, `<=`, `>=` and `~=` all contain an `=` and none of them writes.
    for line in [
        "if self.hp == profile.get(\"x\") then end",
        "if self.hp >= profile.get(\"x\") then end",
        "if self.hp ~= profile.get(\"x\") then end",
    ] {
        assert_eq!(count(line), 0, "{line}");
    }
}

#[test]
fn a_comment_does_not_trigger_anything() {
    assert_eq!(count("-- iterate with pairs() over 1.5 things"), 0);
}

#[test]
fn several_hazards_on_one_line_are_all_reported() {
    let found = hazards("for k in pairs(t) do self.x = 1.5 end");
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_clean_script_reports_nothing() {
    assert_eq!(
        count(
            r#"
function on_ready(self)
  self.hp = 20
  self.order = { "bolt", "nova" }
end

function on_tick(self)
  for i, name in ipairs(self.order) do
    self.last = name
  end
  self.pos = self.pos + vec2(fx.new(1), fx.new(0))
  self.angle = fx.from_angle("45.0")
end
"#
        ),
        0
    );
}

// -- a write into a copied table ------------------------------------------

/// `self.bag.b = 2` lands in a temporary and is dropped.
///
/// A script variable holding a map or a list is converted to a fresh Lua table
/// on every read, so a field written through one goes nowhere. It used to be
/// the only failure in the engine that carried nothing at all — no error, no
/// warning, no lint — which is what I9 forbids, and it cost an afternoon every
/// time somebody hit it.
///
/// Why it is a lint and not a runtime guard: see `tests/copied_table_facts.rs`,
/// which pins both reasons as tests. In short, a metatable on the table sees
/// only *absent* keys, and an empty proxy is invisible to the host's own table
/// walk, so one guard misses half the cases and the other destroys the
/// variable.
fn lost_writes(source: &str) -> Vec<String> {
    dimetric_sim::lint::check("s.lua", source)
        .into_iter()
        .filter(|d| d.code == dimetric_core::Code::SCRIPT_LOST_WRITE)
        .map(|d| d.message)
        .collect()
}

#[test]
fn a_field_written_through_a_copied_table_is_named() {
    let found = lost_writes("function on_tick(self)\n  self.bag.b = 2\nend\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("bag"), "{found:?}");
}

#[test]
fn a_nested_field_is_named_by_the_variable_it_belongs_to() {
    // The report's second real case, and the one a metatable guard would have
    // missed entirely: `at` is already in the table, so `__newindex` would
    // never fire.
    let found = lost_writes("function on_tick(self)\n  self.run.pending.at = 2\nend\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("run"), "{found:?}");
}

#[test]
fn an_indexed_slot_is_caught_too() {
    let found = lost_writes("function on_tick(self)\n  self.order[1] = \"c\"\nend\n");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("order"), "{found:?}");
}

#[test]
fn the_message_names_the_spelling_that_works() {
    let found = lost_writes("function on_tick(self)\n  self.bag.b = 2\nend\n");
    assert!(found[0].contains("local t = self.bag"), "{found:?}");
}

#[test]
fn writing_the_variable_itself_is_not_a_lost_write() {
    // `self.bag = …` is the write that works, and is how everything else in
    // this engine's example scripts does it.
    assert!(lost_writes("function on_tick(self)\n  self.bag = { a = 1 }\nend\n").is_empty());
    assert!(lost_writes("function on_tick(self)\n  self.hp = self.hp - 1\nend\n").is_empty());
}

#[test]
fn the_local_that_is_assigned_back_is_left_alone() {
    // The pattern the engine's own example game uses in three places, and the
    // one a runtime guard would have refused.
    let source = "function on_tick(self)\n\
                  \x20 local pending = self.pending or {}\n\
                  \x20 pending[#pending + 1] = { id = 1 }\n\
                  \x20 self.pending = pending\n\
                  end\n";
    assert!(lost_writes(source).is_empty(), "{:?}", lost_writes(source));
}

#[test]
fn a_comparison_left_of_an_equals_is_not_an_assignment() {
    assert!(lost_writes("if self.run.pending == nil then end\n").is_empty());
    assert!(lost_writes("local n = self.bag.count\n").is_empty());
}

// -- state kept in Lua --------------------------------------------------

fn lua_state(source: &str) -> Vec<String> {
    dimetric_sim::lint::check("s.lua", source)
        .into_iter()
        .filter(|d| d.code == dimetric_core::Code::SCRIPT_LUA_STATE)
        .map(|d| d.message)
        .collect()
}

#[test]
fn a_handle_cached_in_a_file_scope_local_is_named() {
    // The idiom three of this repository's own replay fixtures used, and the
    // one shape that does not survive a resume: `on_ready` does not fire again,
    // so the local is nil and the next use of it raises.
    let found = lua_state(
        "local mark\n\
         function on_ready(self)\n\
         \x20 mark = scene.find(\"/World/Mark\")\n\
         end\n",
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("`mark`"), "{}", found[0]);
}

#[test]
fn a_file_scope_local_assigned_where_it_is_declared_is_fine() {
    // Re-established every time the script loads, which is what makes it safe.
    // A constant, a required module and a table of data all live here.
    assert!(lua_state("local MAX = 10\nlocal helper = require(\"scripts/h.lua\")\n").is_empty());
    assert!(lua_state("local SPELLS = { \"fire\", \"frost\" }\n").is_empty());
}

#[test]
fn a_local_inside_a_function_is_not_one_of_these() {
    // Declared and used within the call, so there is nothing to restore.
    assert!(lua_state(
        "function on_tick(self)\n\
         \x20 local mark = scene.find(\"/World/Mark\")\n\
         \x20 mark.pos = mark.pos + vec2(1, 0)\n\
         end\n"
    )
    .is_empty());
}

#[test]
fn a_local_function_is_left_alone() {
    // Rebinding one is pathological rather than a cache, and naming it would be
    // noise in every script that defines a helper.
    assert!(lua_state(
        "local function step(n)\n\
         \x20 return n + 1\n\
         end\n\
         function on_tick(self)\n\
         \x20 self.n = step(self.n or 0)\n\
         end\n"
    )
    .is_empty());
}

#[test]
fn every_name_in_one_declaration_is_covered() {
    let found = lua_state(
        "local a, b\n\
         function on_ready(self)\n\
         \x20 a = 1\n\
         \x20 b = 2\n\
         end\n",
    );
    assert_eq!(found.len(), 2, "{found:#?}");
}

#[test]
fn an_author_who_has_checked_can_say_so() {
    assert!(lua_state(
        "local cursor\n\
         function on_tick(self)\n\
         \x20 cursor = ui.pointer() -- @transient\n\
         end\n"
    )
    .is_empty());
}

#[test]
fn comparing_a_file_local_is_not_assigning_to_it() {
    assert!(lua_state(
        "local mark\n\
         function on_tick(self)\n\
         \x20 if mark == nil then log.info(\"none\") end\n\
         end\n"
    )
    .is_empty());
}

#[test]
fn the_example_game_is_clean_under_this_lint() {
    // A lint whose first run finds problems in the engine's own scripts is
    // either right about them or wrong about the rule, and both are worth
    // knowing before it ships.
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sorcerer/scripts");
    let mut complaints = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("scripts") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("readable");
        for message in lost_writes(&source) {
            complaints.push(format!("{}: {message}", path.display()));
        }
    }
    assert!(complaints.is_empty(), "{complaints:#?}");
}
