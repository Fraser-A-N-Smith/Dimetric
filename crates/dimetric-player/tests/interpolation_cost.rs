//! What a frame pays to be drawn between two ticks.
//!
//! `Session::step` kept a whole `SimState` clone every tick, purely so the next
//! frame could interpolate positions. Cloning the state clones every node's
//! properties *and* every script variable, so the cost of drawing rose with
//! everything a game keeps — a combat log filling over a floor, a run's
//! history. The fortieth round of a floor cost more to draw than the first, for
//! a frame that reads one value per node.
//!
//! This measures the two side by side: the clone that was there, and the
//! position store that replaced it, over the same state. The assertions are
//! deliberately loose — a timing test that fails on a busy machine is worse
//! than none — but the gap is large enough that a loose bound still catches a
//! regression to cloning.
//!
//! # On build profiles
//!
//! The absolute comparison runs in release only, and that is a finding rather
//! than a convenience. Unoptimised, the fixed-point composition in
//! `Scene::world_positions` costs about twenty times what it does optimised
//! (412 ns a node against 20), while a clone is mostly `memcpy` and `malloc`
//! and barely cares — so in a debug build the store measures *slower* than the
//! clone it replaces, and a ratio asserted there would be asserting something
//! untrue of any build anybody plays.
//!
//! The test that matters does run in both: whatever the profile, what a frame
//! keeps must not grow with what the game keeps. That is the whole of G69, and
//! it is a shape claim rather than a speed one.

use std::time::Instant;

use dimetric_render::Motion;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::InputFrame;
use dimetric_sim::{LuaHost, Sim, SimConfig};

/// A scene of `count` sprites under one root, so the tree is worth walking.
fn tree(count: usize) -> Scene {
    let mut text = String::from(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/keep.lua\"\n",
    );
    for i in 0..count {
        // Ids are eight characters after the prefix; `i` padded keeps them
        // unique and legal.
        text.push_str(&format!(
            "\n[[node]]\nid = \"n_{i:08}\"\nkind = \"Node2D\"\nname = \"N{i}\"\n\
             parent = \"n_root0000\"\npos = [{}.0, {}.0]\n",
            i % 64,
            i / 64
        ));
    }
    let out = dimetric_scene::parse(&text, "c.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let mut scene = out.doc.unwrap().scene;
    scene.update_world_transforms();
    scene
}

/// A simulation holding a script variable that grows every tick, which is what
/// a combat log or a run history is.
fn sim_with_a_growing_log(nodes: usize, ticks: usize) -> Sim {
    let mut host = LuaHost::new(60).expect("lua host");
    host.load(
        "scripts/keep.lua",
        "function on_tick(self)\n\
         \x20 local log = self.log or {}\n\
         \x20 log[#log + 1] = \"the adventurer struck for seven\"\n\
         \x20 self.log = log\n\
         end\n",
    )
    .expect("loads");
    let mut sim = Sim::new(tree(nodes), 1, Box::new(host), SimConfig::default());
    for _ in 0..ticks {
        sim.step(InputFrame::idle(1));
    }
    let reported = sim.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
    sim
}

/// The best of several runs, which is the figure least disturbed by whatever
/// else the machine is doing.
fn best_of(rounds: usize, mut body: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..rounds {
        let started = Instant::now();
        body();
        best = best.min(started.elapsed().as_secs_f64());
    }
    best
}

#[test]
// Release only: see the note at the top of this file. In an unoptimised build
// the composition dominates and the ratio says nothing about a real frame.
#[cfg_attr(debug_assertions, ignore)]
fn keeping_positions_is_far_cheaper_than_cloning_the_state() {
    let nodes = 400;
    let ticks = 400;
    let sim = sim_with_a_growing_log(nodes, ticks);

    // Proof the state really is big: the log has an entry per tick, so the
    // clone below has that much to copy and the store below does not.
    let clone = best_of(5, || {
        let snapshot = sim.snapshot();
        std::hint::black_box(&snapshot);
    });

    let mut motion = Motion::new();
    let keep = best_of(5, || {
        motion.refresh(&sim.state().scene);
        std::hint::black_box(&motion);
    });

    eprintln!(
        "{nodes} nodes, a {ticks}-entry log: snapshot {:.3} ms, refresh {:.3} ms ({:.1}x)",
        clone * 1e3,
        keep * 1e3,
        clone / keep
    );
    // Measured at 7.7x on a 400-node tree with a 400-entry log, release build.
    // Three is the bound, because this is wall-clock on a shared machine.
    assert!(
        keep * 3.0 < clone,
        "keeping positions should be several times cheaper than cloning: \
         refresh {:.3} ms vs snapshot {:.3} ms",
        keep * 1e3,
        clone * 1e3
    );
}

#[test]
fn what_a_frame_keeps_does_not_grow_with_what_the_game_keeps() {
    // The complaint, directly. The same tree with a short log and a long one:
    // the store costs the same, because its size is the tree's and not the
    // state's. A clone's cost is the state's, which is what made the fortieth
    // round dearer than the first.
    let nodes = 400;
    let early = sim_with_a_growing_log(nodes, 10);
    let late = sim_with_a_growing_log(nodes, 800);

    let mut motion = Motion::new();
    let first = best_of(5, || {
        motion.refresh(&early.state().scene);
        std::hint::black_box(&motion);
    });
    let fortieth = best_of(5, || {
        motion.refresh(&late.state().scene);
        std::hint::black_box(&motion);
    });

    // And the clones, for the comparison that makes the point.
    let clone_early = best_of(5, || {
        std::hint::black_box(&early.snapshot());
    });
    let clone_late = best_of(5, || {
        std::hint::black_box(&late.snapshot());
    });

    eprintln!(
        "refresh: {:.3} ms early, {:.3} ms late ({:.2}x). \
         snapshot: {:.3} ms early, {:.3} ms late ({:.2}x)",
        first * 1e3,
        fortieth * 1e3,
        fortieth / first,
        clone_early * 1e3,
        clone_late * 1e3,
        clone_late / clone_early
    );

    // Loose, because this is wall-clock on a shared machine: the store must
    // not have become proportional to the log. Measured at 1.05x for the store
    // against 1.48x for the clone, release, and 1.01x for the store in debug —
    // so a factor of two is comfortable in either profile while still catching
    // a store that started copying the state again.
    assert!(
        fortieth < first * 2.0,
        "the store grew with the log: {:.3} ms to {:.3} ms",
        first * 1e3,
        fortieth * 1e3
    );
}
