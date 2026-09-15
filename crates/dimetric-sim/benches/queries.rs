//! What a spatial query costs, at the density the stress scene reaches.
//!
//! The slice's profiling was done through Lua, which measures the query and the
//! call that reached it together. That was enough while the query was most of a
//! frame and it is not enough now: two changes in a row measured as noise
//! against the boundary they were behind. This measures the query on its own.
//!
//! Run it with `cargo bench -p dimetric-sim --bench queries`. No harness, for
//! the same reason the handle bench has none.
//!
//! # What it found
//!
//! Numbers from one machine, release build; the ratios are the finding.
//!
//! The scene is the stress scene's shape: 400 projectiles and 40 enemies in the
//! same few cells, every projectile asking for the nearest enemy within 160
//! units. That is the worst case for a query that has to sort the tagged out of
//! the untagged, because the untagged outnumber them ten to one and share their
//! cells.
//!
//! A tagged query walks an index holding only that tag, so its cost follows how
//! many bodies carry the tag rather than how many are nearby.
//!
//! | Query | Per call |
//! |---|---|
//! | `nearest`, tagged, over the main grid with a bloom filter | 5.47 us |
//! | `nearest`, tagged, over the tag's own index | 1.92 us |
//! | `near`, tagged, over the main grid with a bloom filter | 9.78 us |
//! | `near`, tagged, over the tag's own index | 2.30 us |
//! | `nearest`, untagged | 11.4 us |
//! | rebuilding every index | 71.5 us |
//!
//! Two and a half to four times faster, for an index rebuilt twice a tick at
//! 0.14 ms. At the stress scene's four hundred queries a tick that trades
//! 0.14 ms for 1.4 ms.
//!
//! It does not show up end to end, and that is the other finding: the same
//! scene measures the same either way through Lua, because the boundary the
//! query sits behind costs more than the query. A `scene.nearest` call that
//! returns without looking at anything still costs about 21 us. The broadphase
//! is no longer where the time goes.

use std::time::Instant;

use dimetric_core::{Fx, NodeUid, Vec2Fx};
use dimetric_scene::{KindRegistry, Node, Scene, Value};
use dimetric_sim::world::PhysicsWorld;

/// Bodies carrying the tag being searched for.
const ENEMIES: usize = 40;
/// Bodies that do not, sharing their cells.
const PROJECTILES: usize = 400;
/// Queries per measurement.
const QUERIES: usize = 20_000;

fn main() {
    let scene = crowded();
    let world = PhysicsWorld::build(&scene);
    println!(
        "Spatial queries — {} bodies ({ENEMIES} tagged `enemy`), {QUERIES} queries per row\n",
        world.bodies().len()
    );

    let radius = Fx::from_int(160);
    let points: Vec<Vec2Fx> = (0..QUERIES)
        .map(|i| {
            let t = (i % 320) as i32;
            Vec2Fx::from_ints(t - 160, (t * 7) % 320 - 160)
        })
        .collect();

    row("nearest, tagged", || {
        let mut found = 0usize;
        for at in &points {
            if world
                .nearest(*at, radius, Some("enemy"), |_| true)
                .is_some()
            {
                found += 1;
            }
        }
        found
    });

    row("nearest, untagged", || {
        let mut found = 0usize;
        for at in &points {
            if world.nearest(*at, radius, None, |_| true).is_some() {
                found += 1;
            }
        }
        found
    });

    row("nearest, a tag nothing carries", || {
        let mut found = 0usize;
        for at in &points {
            if world
                .nearest(*at, radius, Some("nobody"), |_| true)
                .is_some()
            {
                found += 1;
            }
        }
        found
    });

    row("near, tagged", || {
        let mut found = 0usize;
        for at in &points {
            found += world.within_tagged(*at, radius, Some("enemy")).len();
        }
        found
    });

    row("near, untagged", || {
        let mut found = 0usize;
        for at in &points {
            found += world.within(*at, radius).len();
        }
        found
    });

    // The index is rebuilt every tick, twice, so what it costs to build is part
    // of what it costs to have.
    let mut rebuilt = PhysicsWorld::build(&scene);
    let start = Instant::now();
    for _ in 0..1000 {
        rebuilt.reindex();
    }
    let elapsed = start.elapsed();
    println!(
        "{:<34} {:>9.2} us per reindex",
        "reindex",
        elapsed.as_secs_f64() * 1e6 / 1000.0
    );
}

fn row(label: &str, f: impl Fn() -> usize) {
    // Once to warm, then measured.
    let hits = f();
    let start = Instant::now();
    let again = f();
    let elapsed = start.elapsed();
    assert_eq!(hits, again, "the query is not reproducible");
    println!(
        "{label:<34} {:>9.3} us per query   {hits} hits",
        elapsed.as_secs_f64() * 1e6 / QUERIES as f64
    );
}

/// The stress scene's shape: a small arena, ten times as many untagged bodies
/// as tagged ones, all of them sharing cells.
fn crowded() -> Scene {
    let mut scene = Scene::new();
    let root = scene
        .insert(Node::new(uid(0), "Node2D", "Bench"), None)
        .expect("root");

    for i in 0..ENEMIES {
        let mut node = collider(i + 1, "Enemy");
        node.tags = vec!["enemy".to_string(), "skeleton".to_string()];
        node.transform.pos = ring(i, 96);
        scene.insert(node, Some(root)).expect("enemy");
    }
    for i in 0..PROJECTILES {
        let mut node = collider(ENEMIES + i + 1, "Bolt");
        node.tags = vec!["projectile".to_string()];
        node.transform.pos = ring(i, 64);
        scene.insert(node, Some(root)).expect("projectile");
    }
    scene.update_world_transforms();
    let _ = KindRegistry::with_builtins();
    scene
}

fn collider(index: usize, name: &str) -> Node {
    let mut node = Node::new(uid(index), "Collider", format!("{name}{index}"));
    node.props
        .insert("shape".to_string(), Value::Str("Circle".to_string()));
    node.props
        .insert("radius".to_string(), Value::Scalar(Fx::from_int(6)));
    node
}

/// Spread over a few cells, the way an arena full of fighting does.
fn ring(i: usize, radius: i32) -> Vec2Fx {
    let step = (i as i32 * 37) % 360;
    let x = (step - 180) * radius / 180;
    let y = ((step * 3) % 360 - 180) * radius / 180;
    Vec2Fx::from_ints(x, y)
}

fn uid(index: usize) -> NodeUid {
    NodeUid::parse(&format!("n_{index:08}")).expect("a valid id")
}
