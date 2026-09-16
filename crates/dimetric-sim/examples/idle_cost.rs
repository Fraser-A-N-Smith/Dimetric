//! What an idle tick costs.
//!
//! A turn-based game spends most of its wall-clock time waiting for a person
//! to decide while the engine ticks at 60Hz regardless. The stress scene
//! measures a *busy* tick; this measures the other kind, which is the one a
//! tactics game has 144,000 of.
//!
//! ```text
//! cargo run --release -p dimetric-sim --example idle_cost
//! ```
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::{input::InputFrame, NoScripts, Sim, SimConfig};
use std::time::Instant;

fn scene(actors: usize) -> Scene {
    let mut s = String::from("format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n[[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n");
    for i in 0..actors {
        s.push_str(&format!("\n[[node]]\nid = \"n_a{:07}\"\nkind = \"Collider\"\nname = \"A{i}\"\nparent = \"n_root0000\"\npos = [{}.0, {}.0]\nsize = [16.0, 16.0]\n", i, (i % 20) * 24, (i / 20) * 24));
    }
    let out = dimetric_scene::parse(&s, "b.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn main() {
    for actors in [20usize, 100] {
        let mut sim = Sim::new(scene(actors), 1, Box::new(NoScripts), SimConfig::default());
        // Warm up.
        for _ in 0..100 {
            sim.step(InputFrame::idle(1));
        }

        let n = 20_000;
        let t = Instant::now();
        for _ in 0..n {
            sim.step(InputFrame::idle(1));
        }
        let step = t.elapsed();

        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(sim.hash());
        }
        let hash = t.elapsed();

        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(sim.snapshot());
        }
        let snap = t.elapsed();

        println!("{actors} actors, idle:");
        println!("  step     {:>8.2} us", step.as_secs_f64() * 1e6 / n as f64);
        println!("  hash     {:>8.2} us", hash.as_secs_f64() * 1e6 / n as f64);
        println!("  snapshot {:>8.2} us", snap.as_secs_f64() * 1e6 / n as f64);
        let total = (step + hash + snap).as_secs_f64() / n as f64;
        println!(
            "  all three per tick: {:.2} us -> 144,000 ticks = {:.1} s",
            total * 1e6,
            total * 144_000.0
        );
    }
}
