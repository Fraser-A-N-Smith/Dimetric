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
fn a_profile_read_that_goes_nowhere_near_state_is_not() {
    assert_eq!(count("local unlocked = profile.get(\"knows_fire\")"), 0);
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
