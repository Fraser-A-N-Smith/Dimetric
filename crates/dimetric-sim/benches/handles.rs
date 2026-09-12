//! What a Lua node handle costs, at slice-game entity counts.
//!
//! §5.4 of the design document flags this as the most likely performance
//! surprise in the whole design, and M4's acceptance criterion is that it be
//! measured at 500+ scripted entities. This is that measurement.
//!
//! Run it with `cargo bench -p dimetric-sim`. It has no harness and no
//! statistical machinery: the differences being looked for here are large, and
//! a table that anyone can read beats a framework nobody re-runs.
//!
//! The method is a difference, not an absolute. Every scenario runs the same
//! scene with a different script, so subtracting one row from another isolates
//! one cost. Per-call cost comes from the slope across 1, 16 and 64 calls per
//! tick rather than from dividing a single measurement, which would fold in the
//! fixed per-hook dispatch overhead.
//!
//! # What it found
//!
//! Numbers from one developer machine, release build. Treat the ratios as the
//! finding and the absolutes as a baseline to notice regressions against.
//!
//! **Handle validation is not the problem.** It costs about 140 ns and is flat
//! from 100 to 2000 entities — it does not degrade with scene size. §5.4 named
//! this the most likely performance surprise in the design, and it is not one.
//! The slotmap lookup it worried about is real but cheap, and it is not even the
//! dominant part: a validation is an app-data fetch, an `Rc` clone, a `RefCell`
//! borrow and a hash lookup, and all four together are under 150 ns.
//!
//! **Allocating the value costs three times more than validating the handle.**
//! A call that hands Lua a new userdata — `self.pos`, a `vec2`, the result of
//! any arithmetic — adds about 430 ns on top. Scripts that touch many vectors
//! pay for the vectors, not for the handles.
//!
//! **`scene.find` is what actually costs, at roughly 3.9 us per call.** That is
//! 28 times a validation, and it has two separate causes. The fixed part is the
//! path string crossing from Lua, being split, and a fresh handle coming back.
//! The variable part is `child_named`, which walks the sibling list: a lookup
//! that resolves to the last of 2000 siblings costs 13.4 us against 4.7 us for
//! the first. With every entity calling it every tick that is quadratic.
//!
//! **500 entities fit comfortably; 2000 do not.** At 60 Hz, a realistic script
//! calling `scene.find` every tick uses about a quarter of the frame at 500
//! entities, half at 1000, and overruns at 2000.
//!
//! # What to do about it
//!
//! Resolve paths in `on_ready` and keep the id, not the path. That removes the
//! sibling walk and takes 2000 entities from 224% of frame to 113%. Note the
//! wrinkle the `TYPICAL_CACHED` script documents: a handle stored in a script
//! variable is converted to its id string, because script state has to survive a
//! snapshot and a userdata pointer cannot. So the idiom is `scene.by_id`, not a
//! cached handle.
//!
//! If that is not enough, the engine-side fix is a name-to-child map per node,
//! turning `child_named` into a hash lookup. It costs memory on every node and
//! has to keep deterministic iteration, so it is not worth doing until a shipped
//! game needs it — which is exactly what this benchmark exists to tell you.

use std::time::{Duration, Instant};

use dimetric_core::{NodeUid, Vec2Fx};
use dimetric_scene::value::Value;
use dimetric_scene::{Node, Scene};
use dimetric_sim::{InputFrame, LuaHost, NoScripts, Sim, SimConfig};

/// Entity counts to measure at. 500 is the criterion; the others are there to
/// show the shape of the curve, because a cost that is fine at 500 and
/// quadratic is not fine.
const COUNTS: &[usize] = &[100, 500, 1000, 2000];

/// Ticks per measurement, after warm-up.
const TICKS: usize = 120;
/// Ticks discarded before measuring.
const WARMUP: usize = 30;

/// One tick's budget at 60 Hz.
const FRAME_BUDGET: Duration = Duration::from_nanos(16_666_667);

fn main() {
    println!("Handle validation cost — {TICKS} ticks per measurement\n");

    for &count in COUNTS {
        println!("== {count} scripted entities ==");
        let baseline = measure(count, None);
        row("physics only, no Lua", baseline, baseline, count);

        let empty = measure(count, Some(&validations(0)));
        row("+ empty on_tick", empty, baseline, count);

        let one = measure(count, Some(&validations(1)));
        row("+ 1 validation", one, baseline, count);

        let sixteen = measure(count, Some(&validations(16)));
        row("+ 16 validations", sixteen, baseline, count);

        let sixty_four = measure(count, Some(&validations(64)));
        row("+ 64 validations", sixty_four, baseline, count);

        // `valid()` resolves and returns a bool; `self.pos` resolves and returns
        // a freshly allocated userdata. The gap between them is what building a
        // Lua value costs, separate from validating the handle.
        let allocating = measure(count, Some(&allocations(16)));
        row("+ 16 vec2 reads", allocating, baseline, count);

        let local = measure(count, Some(TYPICAL_LOCAL));
        row("typical, self only", local, baseline, count);

        let searching = measure(count, Some(TYPICAL_SEARCHING));
        row("typical, + find (first)", searching, baseline, count);

        let searching_last = measure(count, Some(TYPICAL_SEARCHING_LAST));
        row("typical, + find (last)", searching_last, baseline, count);

        let cached = measure(count, Some(TYPICAL_CACHED));
        row("typical, + cached id", cached, baseline, count);

        // The slope between 1 and 64 calls. Taking a difference cancels the
        // fixed cost of entering the hook, which a single division would
        // silently include and overstate the per-call cost by.
        let span = sixty_four.saturating_sub(one).as_secs_f64();
        let per_call = span / (63.0 * count as f64) * 1e9;
        println!("  -> {per_call:.0} ns per handle validation");
        let per_alloc =
            allocating.saturating_sub(sixteen).as_secs_f64() / (16.0 * count as f64) * 1e9;
        println!("  -> {per_alloc:.0} ns extra per call that returns a userdata");
        let find_first = searching.saturating_sub(local).as_secs_f64() / count as f64 * 1e9;
        let find_last = searching_last.saturating_sub(local).as_secs_f64() / count as f64 * 1e9;
        println!("  -> {find_first:.0} ns per scene.find hitting the first sibling");
        println!("  -> {find_last:.0} ns per scene.find hitting the last sibling");
        let by_id = cached.saturating_sub(local).as_secs_f64() / count as f64 * 1e9;
        println!("  -> {by_id:.0} ns per scene.by_id with the path resolved once");
        println!(
            "  -> frame budget: {:.0}% best case, {:.0}% worst case\n",
            searching.as_secs_f64() / FRAME_BUDGET.as_secs_f64() * 100.0,
            searching_last.as_secs_f64() / FRAME_BUDGET.as_secs_f64() * 100.0
        );
    }
}

fn row(label: &str, measured: Duration, baseline: Duration, count: usize) {
    let millis = measured.as_secs_f64() * 1e3;
    let over = measured.saturating_sub(baseline).as_secs_f64() * 1e3;
    let per_entity = measured.as_secs_f64() / count as f64 * 1e6;
    let budget = measured.as_secs_f64() / FRAME_BUDGET.as_secs_f64() * 100.0;
    println!(
        "  {label:<24} {millis:>7.3} ms/tick  (+{over:>6.3} over floor)  \
         {per_entity:>6.2} us/entity  {budget:>5.1}% of frame"
    );
}

/// Mean tick time for a scene of `count` scripted entities.
fn measure(count: usize, script: Option<&str>) -> Duration {
    let scene = build_scene(count, script.is_some());
    let mut sim = match script {
        Some(source) => {
            let mut host = LuaHost::new(60).expect("lua host");
            host.load(SCRIPT_PATH, source)
                .expect("benchmark script loads");
            Sim::new(scene, 1, Box::new(host), SimConfig::default())
        }
        None => Sim::new(scene, 1, Box::new(NoScripts), SimConfig::default()),
    };

    for _ in 0..WARMUP {
        sim.step(InputFrame::idle(1));
    }
    assert!(
        sim.diagnostics().is_empty(),
        "the benchmark script errored: {}",
        sim.diagnostics()
    );

    let start = Instant::now();
    for _ in 0..TICKS {
        sim.step(InputFrame::idle(1));
    }
    start.elapsed() / TICKS as u32
}

const SCRIPT_PATH: &str = "scripts/bench.lua";

/// A scene of `count` colliders on a grid, plus one target to search for.
///
/// Spread over a grid rather than stacked, so the broadphase does the work it
/// would do in a real room instead of degenerating into one bucket.
fn build_scene(count: usize, scripted: bool) -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid(0), "Node2D", "Bench"), None)
        .expect("root inserts");

    // Two targets: one inserted before the crowd and one after. `child_named`
    // walks the sibling list in order, so which one a script looks up decides
    // whether the lookup is O(1) or O(entities). Measuring only the lucky case
    // — as the first draft of this benchmark did — hides the whole problem.
    let mut first = Node::new(uid(1), "Node2D", "First");
    first.transform.pos = Vec2Fx::from_ints(0, 0);
    scene
        .insert(first, Some(root))
        .expect("first target inserts");

    let stride = (count as f64).sqrt().ceil() as i32;
    for i in 0..count {
        let mut node = Node::new(uid(i + 2), "Collider", format!("E{i}"));
        node.transform.pos = Vec2Fx::from_ints(
            (i as i32 % stride) * 24 - stride * 12,
            (i as i32 / stride) * 24 - stride * 12,
        );
        node.props
            .insert("shape".into(), Value::Enum("Circle".into()));
        node.props.insert(
            "radius".into(),
            Value::Scalar(dimetric_core::Fx::from_int(8)),
        );
        if scripted {
            node.script = Some(dimetric_scene::Reference::Script(SCRIPT_PATH.into()));
        }
        scene.insert(node, Some(root)).expect("entity inserts");
    }

    let mut last = Node::new(uid(1), "Node2D", "Last");
    last.transform.pos = Vec2Fx::from_ints(0, 0);
    last.uid = uid(count + 2);
    scene.insert(last, Some(root)).expect("last target inserts");

    scene.update_world_transforms();
    scene
}

/// `n_` plus eight characters, derived from an index.
fn uid(i: usize) -> NodeUid {
    NodeUid::parse(&format!("n_{i:08}")).expect("generated ids are well formed")
}

/// A script whose `on_tick` performs `n` handle validations and nothing else.
///
/// `valid()` resolves the handle and returns a boolean. It is the closest thing
/// to measuring validation on its own: no allocation, no property read, no
/// transform maths.
fn validations(n: usize) -> String {
    let mut source = String::from("function on_tick(self)\n");
    for _ in 0..n {
        source.push_str("  self:valid()\n");
    }
    source.push_str("end\n");
    source
}

/// A script whose `on_tick` reads `self.pos` `n` times.
///
/// Same validation as `valid()`, plus building a `vec2` userdata to hand back.
fn allocations(n: usize) -> String {
    let mut source = String::from("function on_tick(self)\n");
    for _ in 0..n {
        source.push_str("  local p = self.pos\n");
    }
    source.push_str("end\n");
    source
}

/// Roughly what a real enemy script does, touching only its own node.
const TYPICAL_LOCAL: &str = r#"
local SPEED = fx.new(40)

function on_ready(self)
  self.hp = 40
end

function on_tick(self)
  local here = self.pos
  if self.hp > 0 then
    self:set_velocity(vec2(SPEED, fx.new(0)))
  end
end
"#;

/// The same, plus the path lookup almost every real script starts with,
/// resolving to a node near the front of the sibling list.
const TYPICAL_SEARCHING: &str = r#"
local SPEED = fx.new(40)

function on_ready(self)
  self.hp = 40
end

function on_tick(self)
  local target = scene.find("/Bench/First")
  if not target then return end

  local to = target.pos - self.pos
  if to:length() > fx.new(8) then
    self:set_velocity(to:normalized() * SPEED)
  end
end
"#;

/// The same lookup, resolving to the last sibling instead of the first.
///
/// This is the case a real game hits: the player is spawned once and the
/// enemies stream in after it, so `scene.find("/Arena01/Player")` scans past
/// every enemy on every call, from every enemy.
const TYPICAL_SEARCHING_LAST: &str = r#"
local SPEED = fx.new(40)

function on_ready(self)
  self.hp = 40
end

function on_tick(self)
  local target = scene.find("/Bench/Last")
  if not target then return end

  local to = target.pos - self.pos
  if to:length() > fx.new(8) then
    self:set_velocity(to:normalized() * SPEED)
  end
end
"#;

/// The same work, but resolving the path once in `on_ready` and looking the
/// node up by id afterwards.
///
/// Note what this script cannot do: hold the handle itself. A handle stored in
/// a script variable is converted to its id string, because script state has to
/// survive a snapshot and a userdata pointer does not. So the idiom is to keep
/// the id and re-resolve, which skips the path walk but not the lookup.
const TYPICAL_CACHED: &str = r#"
local SPEED = fx.new(40)

function on_ready(self)
  self.hp = 40
  local target = scene.find("/Bench/Last")
  if target then
    self.target_id = target:id()
  end
end

function on_tick(self)
  local target = scene.by_id(self.target_id)
  if not target then return end

  local to = target.pos - self.pos
  if to:length() > fx.new(8) then
    self:set_velocity(to:normalized() * SPEED)
  end
end
"#;
