//! The parts of playing a game that do not need a window.

use std::time::Duration;

use dimetric_core::Fx;
use dimetric_player::{Action, Bindings, Clock, Held};
use dimetric_sim::input::buttons;

// -- the clock ----------------------------------------------------------

#[test]
fn sixty_frames_of_a_sixtieth_buy_sixty_ticks() {
    // Not fifty-eight. The remainder of a frame that does not divide evenly is
    // kept, which is the whole reason the clock accumulates rather than
    // rounding.
    let mut clock = Clock::new(60);
    let frame = Duration::from_micros(16_667);
    let ticks: u32 = (0..60).map(|_| clock.advance(frame)).sum();
    assert_eq!(ticks, 60);
}

#[test]
fn a_frame_shorter_than_a_tick_buys_nothing_yet() {
    let mut clock = Clock::new(60);
    assert_eq!(clock.advance(Duration::from_millis(8)), 0);
    assert!(
        clock.alpha() > 0.4 && clock.alpha() < 0.6,
        "{}",
        clock.alpha()
    );
    assert_eq!(clock.advance(Duration::from_millis(9)), 1);
}

#[test]
fn a_long_stall_drops_ticks_rather_than_spiralling() {
    // A second owed at 60Hz is sixty ticks. Simulating sixty takes longer than
    // a frame, which owes more still; a game that tries never draws again.
    let mut clock = Clock::new(60);
    let ticks = clock.advance(Duration::from_secs(1));
    assert!(ticks <= 8, "ran {ticks} ticks in one frame");
    assert!(clock.dropped() > 0, "and said nothing about it");
}

#[test]
fn the_clock_reports_what_it_dropped() {
    let mut clock = Clock::new(60);
    clock.advance(Duration::from_secs(1));
    let dropped = clock.dropped();
    clock.advance(Duration::from_secs(1));
    assert!(clock.dropped() > dropped, "a second stall went unrecorded");
}

// -- bindings -----------------------------------------------------------

#[test]
fn both_movement_sets_work() {
    // Whichever a player tries first has to work, or the game reads as broken.
    let bindings = Bindings::wasd();
    assert_eq!(bindings.action("KeyW"), Some(Action::Up));
    assert_eq!(bindings.action("ArrowUp"), Some(Action::Up));
    assert_eq!(bindings.action("KeyQ"), None);
}

#[test]
fn a_held_button_reaches_the_input() {
    let mut held = Held::new();
    held.set(Action::Fire, true);
    assert!(held.player_input().held(buttons::FIRE));
    held.set(Action::Fire, false);
    assert!(!held.player_input().held(buttons::FIRE));
}

#[test]
fn a_diagonal_is_not_faster_than_a_straight_line() {
    let mut straight = Held::new();
    straight.set(Action::Right, true);

    let mut diagonal = Held::new();
    diagonal.set(Action::Right, true);
    diagonal.set(Action::Down, true);

    let a = straight.player_input().move_dir.length();
    let b = diagonal.player_input().move_dir.length();
    assert_eq!(a, Fx::from_int(1));
    assert!(
        (a - b).abs() <= Fx::from_raw(64),
        "straight {a}, diagonal {b}"
    );
}

#[test]
fn opposite_keys_cancel() {
    let mut held = Held::new();
    held.set(Action::Left, true);
    held.set(Action::Right, true);
    assert!(held.player_input().move_dir.is_zero());
}

#[test]
fn losing_focus_releases_everything() {
    // A key held at the moment someone alt-tabs would otherwise stay held: the
    // release event goes to whatever took the focus.
    let mut held = Held::new();
    held.set(Action::Right, true);
    held.set(Action::Fire, true);
    held.release_all();
    let input = held.player_input();
    assert!(input.move_dir.is_zero());
    assert_eq!(input.buttons, 0);
}

#[test]
fn input_from_a_session_writes_exactly_into_a_log() {
    // The value a tick reads has to survive the round trip through an input
    // log, or a session that was played cannot be replayed (I8). A diagonal is
    // the interesting case: it is a normalised vector rather than a whole
    // number.
    let mut held = Held::new();
    held.set(Action::Right, true);
    held.set(Action::Up, true);
    held.set(Action::Fire, true);

    let mut log = dimetric_sim::InputLog::new(7, "test", 1);
    log.push(dimetric_sim::InputFrame {
        players: vec![held.player_input()],
    });
    let text = log.to_text();
    let back = dimetric_sim::InputLog::parse(&text).expect("the log parses");
    assert_eq!(back.frame(0).player(0), held.player_input());
}

// -- a session ----------------------------------------------------------

fn sorcerer() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sorcerer")
        .canonicalize()
        .expect("examples/sorcerer exists")
}

/// The input a "player" produces on a given tick. Deterministic, and varied
/// enough to exercise buttons, movement and aim together.
fn scripted(tick: u64) -> Held {
    let mut held = Held::new();
    held.set(Action::Right, tick % 4 < 2);
    held.set(Action::Down, tick % 8 < 4);
    held.set(Action::Fire, tick % 3 == 0);
    held.aim_at(
        dimetric_core::Angle::from_degrees_str(&format!("{}.0", (tick * 7) % 360))
            .expect("a whole degree is representable"),
    );
    held
}

#[test]
fn a_session_that_was_played_replays() {
    // The claim the player rests on: a run someone played is an input log like
    // any other. If this fails, a bug found by playing can only be reported as
    // a description of what happened.
    use dimetric_player::{Session, SessionConfig};

    let root = sorcerer();
    let recorded = std::env::temp_dir().join("dimetric-player-session.input");
    let mut project = dimetric_host::Project::open(&root, 0);
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed: 42,
            scene: "arena01.dim".to_string(),
            record: Some(recorded.clone()),
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));
    assert!(!session.diagnostics.has_errors(), "{}", session.diagnostics);

    let mut played = Vec::new();
    for tick in 0..240 {
        session.step(&mut project, scripted(tick).player_input());
        played.push(session.hash());
    }
    let written = session
        .finish()
        .unwrap_or_else(|d| panic!("{d}"))
        .expect("a recording session writes its log");

    // Now the same project, driven by the log the session just wrote.
    let log = dimetric_sim::InputLog::parse(&std::fs::read_to_string(&written).expect("the log"))
        .expect("the log parses");
    let mut project = dimetric_host::Project::open(&root, 0);
    let mut replayed = Session::open(
        &mut project,
        SessionConfig {
            seed: log.seed,
            scene: "arena01.dim".to_string(),
            record: None,
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));

    for tick in 0..240u64 {
        replayed.step(&mut project, log.frame(tick).player(0));
        assert_eq!(
            replayed.hash(),
            played[tick as usize],
            "diverged at tick {tick}"
        );
    }
    let _ = std::fs::remove_file(written);
}

#[test]
fn a_frame_comes_out_of_a_session_without_a_gpu() {
    // Extraction is pure computation, which is what lets the ordering and the
    // batching be tested on a build runner with no adapter.
    use dimetric_player::{Session, SessionConfig};

    let root = sorcerer();
    let mut project = dimetric_host::Project::open(&root, 0);
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed: 42,
            scene: "arena01.dim".to_string(),
            record: None,
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));

    for tick in 0..30 {
        session.step(&mut project, scripted(tick).player_input());
    }
    let frame = session.frame(0.5, (480, 270));
    assert!(!frame.sprites.is_empty(), "the scene drew nothing");
}

#[test]
fn a_scene_swap_tells_the_host_its_atlas_changed() {
    // The atlas is uploaded to the GPU once, when the renderer is built. The
    // session rebuilds its own atlas when a scene swaps -- it has to, because
    // the old one holds the previous scene's art -- but a host that is never
    // told cannot re-upload it, and keeps drawing from the entry scene's.
    //
    // When the entry scene is a menu, its art is no art at all: the uploaded
    // atlas holds nothing but the placeholder, and every scene loaded after it
    // draws as magenta-and-black squares while the simulation underneath is
    // entirely correct. A packaged build shipped exactly that, and every
    // offscreen capture of it looked perfect, because a capture builds its
    // renderer after stepping and so picks up the final atlas by accident.
    use dimetric_player::{Session, SessionConfig};

    let root = std::env::temp_dir().join("dimetric-atlas-swap");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::create_dir_all(root.join("assets/sprites")).unwrap();

    // Sixteen opaque red pixels, as a PNG the importer will accept.
    dimetric_render::write_png(
        &root.join("assets/sprites/block.png"),
        4,
        4,
        &[0xC8u8, 0x28, 0x28, 0xFF].repeat(16),
    )
    .unwrap();

    // The entry scene: one node, a script, and deliberately no art.
    std::fs::write(
        root.join("menu.dim"),
        r#"format = "dimetric"
version = 1
[scene]
root = "n_menu0000"
[[node]]
id = "n_menu0000"
kind = "Node2D"
name = "Menu"
script = "script:scripts/menu.lua"
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("play.dim"),
        r#"format = "dimetric"
version = 1
[scene]
root = "n_play0000"
[[node]]
id = "n_play0000"
kind = "Node2D"
name = "Play"
[[node]]
id = "n_block001"
kind = "Sprite2D"
name = "Block"
parent = "n_play0000"
texture = "asset:sprites/block"
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("scripts/menu.lua"),
        "function on_tick(self)\n  scene.request_load(\"scene:play.dim\", {})\nend\n",
    )
    .unwrap();

    let mut project = dimetric_host::Project::open(&root, 0);
    let mut session = Session::open(
        &mut project,
        SessionConfig {
            seed: 0,
            scene: "menu.dim".to_string(),
            record: None,
            settings: dimetric_render::RenderSettings::default(),
            device: dimetric_audio::Device::Silent,
            profile: None,
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));

    assert!(!session.take_atlas_change(), "nothing has been loaded yet");
    assert!(
        session.atlas().region("sprites/block").is_none(),
        "the menu references no art, so none should be packed"
    );

    let mut swapped = false;
    for _ in 0..4 {
        session.step(&mut project, Held::new().player_input());
        swapped |= session.take_atlas_change();
    }

    assert!(swapped, "a scene swap never reported its new atlas");
    assert!(
        session.atlas().region("sprites/block").is_some(),
        "the loaded scene's art is missing from the atlas the host would upload"
    );
    assert!(
        !session.take_atlas_change(),
        "taking the change should clear it"
    );

    let _ = std::fs::remove_dir_all(&root);
}
