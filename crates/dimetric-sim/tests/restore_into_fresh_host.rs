//! What a restored state does not bring with it: whatever a script left in Lua.
//!
//! Script *state* lives in Rust, which is what lets `self.hp` survive a
//! rollback. A file-scope `local` does not: it lives in the environment the
//! host built, and a host that restores somebody else's state has never run
//! `on_ready` for those nodes, because `readied` says they are already ready.
//!
//! Three of this repository's own replay fixtures use that idiom, so it is not
//! an exotic mistake — it is the obvious way to keep a node handle, and it is
//! the one shape that does not survive a resume, a hot reload, or a rollback.

use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::InputFrame;
use dimetric_sim::{LuaHost, Sim, SimConfig};

const SCENE: &str = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
     [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
     script = \"script:main.lua\"\n\n\
     [[node]]\nid = \"n_mark0000\"\nkind = \"Node2D\"\nname = \"Mark\"\n\
     parent = \"n_root0000\"\n";

/// The idiom under test: a handle cached in a file-scope local, set once.
const CACHING: &str = "local mark\n\
     function on_ready(self)\n\
     \x20 mark = scene.find(\"/World/Mark\")\n\
     end\n\
     function on_tick(self)\n\
     \x20 mark.pos = mark.pos + vec2(1, 0)\n\
     end\n";

/// The same work, with nothing kept in Lua between ticks.
const STATELESS: &str = "function on_tick(self)\n\
     \x20 local mark = scene.find(\"/World/Mark\")\n\
     \x20 mark.pos = mark.pos + vec2(1, 0)\n\
     end\n";

fn scene() -> Scene {
    let out = dimetric_scene::parse(SCENE, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.expect("a document").scene
}

fn sim(source: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("host");
    host.load("main.lua", source).expect("loads");
    Sim::new(scene(), 1, Box::new(host), SimConfig::default())
}

/// Run `ticks`, then carry on in the same simulation and in a restored copy in
/// a fresh host, and report what each produced.
fn split(source: &str, ticks: u32, more: u32) -> (Vec<String>, Vec<String>, String) {
    let mut original = sim(source);
    for _ in 0..ticks {
        original.step(InputFrame::idle(1));
    }
    let snapshot = original.snapshot();

    let straight: Vec<String> = (0..more)
        .map(|_| {
            original.step(InputFrame::idle(1));
            original.hash().to_hex()
        })
        .collect();

    let mut restored = sim(source);
    restored.restore(snapshot);
    let resumed: Vec<String> = (0..more)
        .map(|_| {
            restored.step(InputFrame::idle(1));
            restored.hash().to_hex()
        })
        .collect();

    let reported = restored.take_diagnostics().to_string();
    (straight, resumed, reported)
}

#[test]
fn a_script_that_keeps_nothing_in_lua_carries_on_identically() {
    let (straight, resumed, reported) = split(STATELESS, 5, 5);
    assert_eq!(straight, resumed);
    assert_eq!(reported, "", "{reported}");
}

#[test]
fn a_handle_cached_in_a_file_scope_local_is_gone_and_says_so() {
    // The hazard, pinned. It is **loud**: `on_ready` does not fire again — the
    // nodes are already readied, and re-firing it would re-run a run's
    // initialisation and give the adventurer a second starting loadout — so the
    // local is nil and the first use of it raises.
    //
    // Loud is the whole reason this is a documented limitation rather than a
    // silent divergence. A script that hits it reports `DIM0502` on the tick
    // that hits it, naming the file and the line.
    let (straight, resumed, reported) = split(CACHING, 5, 5);
    assert_ne!(straight, resumed, "it survived, and this test is stale");
    assert!(reported.contains("DIM0502"), "{reported}");
    assert!(reported.contains("main.lua"), "{reported}");
}
