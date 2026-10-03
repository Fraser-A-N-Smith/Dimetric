//! `app.quit()`, and `input.pressed("pause")`.
//!
//! Nothing a script could call ended the process, so a Quit row in a menu could
//! not be built honestly — the game's said "Alt+F4". And the runtime returned
//! from its key handler before the pause action reached `held`, so
//! `input.pressed("pause")` was never true in Lua, which is the one key almost
//! every game wants for a menu of its own.
//!
//! The quit request is deliberately **not** simulation state. A run in which
//! somebody chose Quit has to hash the same as one where they closed the
//! window, or a recorded session would replay differently depending on how it
//! ended — and a rollback must not un-ask.

use dimetric_core::StateHash;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::InputFrame;
use dimetric_sim::{LuaHost, Sim, SimConfig};

fn scene() -> Scene {
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
         script = \"script:main.lua\"\n";
    let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.unwrap().scene
}

fn sim(source: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "{load:?}");
    Sim::new(scene(), 1, Box::new(host), SimConfig::default())
}

#[test]
fn a_script_can_ask_to_quit() {
    let mut s = sim("function on_tick(self) app.quit() end\n");
    assert!(!s.take_quit(), "nothing has asked yet");
    s.step(InputFrame::idle(1));
    assert!(s.take_quit(), "the request did not reach the host");
}

#[test]
fn the_request_is_cleared_when_it_is_taken() {
    // Taken, like the events and the log: a host that read it twice would quit
    // twice, and a host that never read it would quit on a later tick for no
    // reason anyone could trace.
    let mut s = sim("local n = 0\n\
         function on_tick(self)\n\
         \x20 n = n + 1\n\
         \x20 if n == 1 then app.quit() end\n\
         end\n");
    s.step(InputFrame::idle(1));
    assert!(s.take_quit());
    s.step(InputFrame::idle(1));
    assert!(!s.take_quit(), "the request came back on its own");
}

#[test]
fn quitting_does_not_move_the_state_hash() {
    // The property that matters. A run where somebody chose Quit and one where
    // they closed the window are the same run, so a recorded session cannot
    // replay differently depending on how it ended.
    let hash_after = |source: &str| -> StateHash {
        let mut s = sim(source);
        for _ in 0..4 {
            s.step(InputFrame::idle(1));
        }
        s.hash()
    };
    let quiet = hash_after("function on_tick(self) self.n = (self.n or 0) + 1 end\n");
    let quitting =
        hash_after("function on_tick(self) self.n = (self.n or 0) + 1; app.quit() end\n");
    assert_eq!(quiet, quitting);
}

#[test]
fn a_rollback_does_not_un_ask() {
    // The request is not snapshotted, so restoring an earlier state leaves it
    // where it was — which is right: the person already clicked Quit, and a
    // rollback is the engine's business rather than theirs.
    let mut s = sim("function on_tick(self) app.quit() end\n");
    let before = s.snapshot();
    s.step(InputFrame::idle(1));
    s.restore(before);
    assert!(s.take_quit(), "a rollback swallowed the request");
}

#[test]
fn the_pause_button_is_readable_from_a_script() {
    // The other half. `input.pressed("pause")` is how a game opens its own menu,
    // and the runtime used to eat the press before it reached the input frame.
    let mut s = sim("function on_tick(self)\n\
         \x20 if input.pressed(\"pause\") then self.opened = (self.opened or 0) + 1 end\n\
         end\n");
    let mut frame = InputFrame::idle(1);
    frame.players[0].buttons |= dimetric_sim::input::buttons::PAUSE;
    s.step(frame);

    let state = s.state();
    let id = state.scene.resolve_path("/World").expect("world");
    let uid = state.scene.get(id).expect("world").uid;
    assert_eq!(
        state
            .var(uid, "opened")
            .and_then(dimetric_scene::Value::as_int),
        Some(1),
        "a script cannot see the pause button"
    );
}
