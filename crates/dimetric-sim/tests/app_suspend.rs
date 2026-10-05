//! `app.suspend()`, `app.resume()`, `app.discard_suspended()`, `app.suspended()`.
//!
//! All four are requests or answers that cross the boundary between the
//! simulation and whatever is running it, and none of them is simulation
//! state. A run that was suspended has to hash identically to one whose window
//! was closed, or how a session *ended* would change what it replayed to — and
//! a rollback must not un-ask.
//!
//! `suspended` is the mirror image: an input the host supplies, like the
//! profile and the fonts. A simulation nobody told is told false, which is the
//! answer a replay and a headless run must give — a recorded session that read
//! a real save would reproduce only on a machine that happened to have one.

use dimetric_core::StateHash;
use dimetric_scene::{KindRegistry, Scene};
use dimetric_sim::input::InputFrame;
use dimetric_sim::suspend::SuspendRequest;
use dimetric_sim::{LuaHost, Sim, SimConfig};

fn scene() -> Scene {
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"World\"\n\
         script = \"script:main.lua\"\n";
    let out = dimetric_scene::parse(text, "f.dim", &KindRegistry::with_builtins());
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    out.doc.expect("a document").scene
}

fn sim(source: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "{load:?}");
    Sim::new(scene(), 1, Box::new(host), SimConfig::default())
}

/// The same, with the host reporting a run waiting to be continued.
fn sim_with_save(source: &str) -> Sim {
    let mut host = LuaHost::new(60).expect("host");
    let load = host.load_all([("main.lua", source)]);
    assert!(load.is_empty(), "{load:?}");
    host.set_suspended(true);
    Sim::new(scene(), 1, Box::new(host), SimConfig::default())
}

#[test]
fn a_script_can_ask_to_suspend() {
    let mut s = sim("function on_tick(self) app.suspend() end\n");
    assert_eq!(s.take_suspend_request(), None, "nothing has asked yet");
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), Some(SuspendRequest::Suspend));
}

#[test]
fn suspending_carries_the_quit_with_it() {
    // One call rather than a convention the game has to remember. A run that
    // wrote its save and kept playing would let somebody suspend, play on,
    // die, and resume the save they had already left behind.
    let mut s = sim("function on_tick(self) app.suspend() end\n");
    s.step(InputFrame::idle(1));
    assert!(s.take_quit(), "the game was not asked to stop");
}

#[test]
fn resuming_and_discarding_do_not_ask_to_quit() {
    let mut s = sim("function on_tick(self) app.resume() end\n");
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), Some(SuspendRequest::Resume));
    assert!(!s.take_quit());

    let mut s = sim("function on_tick(self) app.discard_suspended() end\n");
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), Some(SuspendRequest::Discard));
    assert!(!s.take_quit());
}

#[test]
fn the_request_is_cleared_when_it_is_taken() {
    // Taken, like the quit flag and the events: a host that read it twice would
    // write two saves, and a host that never read it would suspend on a later
    // tick for no reason anyone could trace.
    let mut s = sim("local n = 0\n\
         function on_tick(self)\n\
         \x20 n = n + 1\n\
         \x20 if n == 1 then app.discard_suspended() end\n\
         end\n");
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), Some(SuspendRequest::Discard));
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), None, "it came back on its own");
}

#[test]
fn two_requests_in_one_tick_leave_the_last() {
    // A script that asks twice has made a mistake, and the alternative is a
    // queue whose order is a second thing to reason about. A scene load
    // resolves the same way.
    let mut s = sim("function on_tick(self)\n\
         \x20 app.discard_suspended()\n\
         \x20 app.resume()\n\
         end\n");
    s.step(InputFrame::idle(1));
    assert_eq!(s.take_suspend_request(), Some(SuspendRequest::Resume));
}

#[test]
fn a_simulation_nobody_told_reports_no_suspended_run() {
    // The answer a replay and a headless run must give, and the default rather
    // than something a caller has to remember to set.
    let mut s = sim("function on_tick(self) self.waiting = app.suspended() end\n");
    s.step(InputFrame::idle(1));
    let state = s.state();
    let root = state.scene.root().expect("root");
    let uid = state.scene.get(root).expect("root").uid;
    assert_eq!(
        state.vars[&uid]["waiting"],
        dimetric_scene::Value::Bool(false)
    );
}

#[test]
fn a_host_that_has_one_says_so() {
    let mut s = sim_with_save("function on_tick(self) self.waiting = app.suspended() end\n");
    s.step(InputFrame::idle(1));
    let state = s.state();
    let root = state.scene.root().expect("root");
    let uid = state.scene.get(root).expect("root").uid;
    assert_eq!(
        state.vars[&uid]["waiting"],
        dimetric_scene::Value::Bool(true)
    );
}

#[test]
fn the_host_can_change_its_answer_between_ticks() {
    // Which is what consuming a save looks like from inside: true on the tick
    // that asked, false ever after.
    let mut s = sim_with_save("function on_tick(self) self.waiting = app.suspended() end\n");
    s.step(InputFrame::idle(1));
    s.set_suspended(false);
    s.step(InputFrame::idle(1));
    let state = s.state();
    let root = state.scene.root().expect("root");
    let uid = state.scene.get(root).expect("root").uid;
    assert_eq!(
        state.vars[&uid]["waiting"],
        dimetric_scene::Value::Bool(false)
    );
}

/// Hashes for `ticks` ticks of `source`.
fn hashes(mut s: Sim, ticks: u32) -> Vec<StateHash> {
    (0..ticks)
        .map(|_| {
            s.step(InputFrame::idle(1));
            s.hash()
        })
        .collect()
}

#[test]
fn asking_does_not_move_the_state_hash() {
    // The property the whole shape rests on. A run in which somebody chose
    // "save and quit" must hash identically to one where they closed the
    // window, or a recorded session would replay differently depending on how
    // it ended.
    let plain = "function on_tick(self) self.n = (self.n or 0) + 1 end\n";
    let asking = "function on_tick(self)\n\
         \x20 self.n = (self.n or 0) + 1\n\
         \x20 app.suspend()\n\
         \x20 app.resume()\n\
         \x20 app.discard_suspended()\n\
         end\n";
    assert_eq!(hashes(sim(plain), 6), hashes(sim(asking), 6));
}

#[test]
fn being_told_a_run_is_waiting_does_not_move_the_hash_on_its_own() {
    // Reading the answer is free. What a game *does* with it — showing a
    // Continue row, which writes `visible` on a node — is hashed like any other
    // write, exactly as a decision taken on a profile value is. The flag itself
    // is not in the state.
    let source = "function on_tick(self) local _ = app.suspended() end\n";
    assert_eq!(
        hashes(sim(source), 4),
        hashes(sim_with_save(source), 4),
        "the answer leaked into the state"
    );
}
