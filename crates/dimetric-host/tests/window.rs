//! How big a window a game opens, and that it is never smaller than the game.
//!
//! `dim-play` opened a fixed 1440×810 whatever the project said. For a
//! 1920×1080 game that is smaller than the game, and the renderer's integer
//! upscaling clamped the scale to 1 rather than shrinking the frame — so 240
//! pixels came off each side, a menu button was outside the window, and a click
//! aimed at it landed on nothing.
//!
//! Both halves are fixed. The frame is never cropped (see
//! `dimetric-render/tests/placement.rs`), and the window a game opens is the
//! game's own choice rather than one number in the runtime.
//!
//! None of this is the replay contract. A window size reaches a window manager
//! and a viewport call; the simulation never sees it, so a player who resizes
//! cannot change what a recorded run replays to. The last test here asserts
//! that structurally.

use dimetric_host::settings::{Presentation, Settings, WindowSize};

/// A 1080p monitor, which is what most of these are about.
const FHD: Option<(u32, u32)> = Some((1920, 1080));

fn presentation(window: WindowSize, integer_upscale: Option<bool>) -> Presentation {
    Presentation {
        window,
        integer_upscale,
        ..Default::default()
    }
}

#[test]
fn a_game_drawn_at_the_screens_size_opens_as_large_as_it_comfortably_can() {
    // The reported case. 1920×1080 does not fit a 1080p screen once a title bar
    // is taken, so the window is trimmed and the frame scales to fit it — which
    // is the behaviour that keeps every pixel of the interface on screen. What
    // it must never be is *smaller than the game with the frame at full size*.
    let size = presentation(WindowSize::Internal, None).window_size((1920, 1080), FHD);
    assert_eq!(size, (1728, 972));

    // The shape is kept, so the frame has no letterbox either.
    assert_eq!(size.0 * 1080, size.1 * 1920);
}

#[test]
fn a_pixel_art_game_opens_at_a_whole_multiple() {
    // The scale at which its pixels are square. A window that forced a
    // fractional scale on launch would make `integer_upscale` look broken.
    //
    // 480×270 on a 1080p screen is 3×, which is 1440×810 — exactly the fixed
    // default this replaced, arrived at from the project rather than guessed.
    let p = presentation(WindowSize::Internal, None);
    assert_eq!(p.window_size((480, 270), FHD), (1440, 810));
    // 5× rather than 6×, because the sixth would spill past the allowance the
    // default keeps for a title bar.
    assert_eq!(p.window_size((320, 180), FHD), (1600, 900));
    assert_eq!(p.window_size((160, 90), Some((1280, 720))), (1120, 630));
}

#[test]
fn a_game_that_is_not_pixel_locked_opens_at_its_own_size() {
    // Nothing to round to, so there is no reason to grow or shrink it.
    let p = presentation(WindowSize::Internal, Some(false));
    assert_eq!(p.window_size((1280, 720), FHD), (1280, 720));
    assert_eq!(p.window_size((640, 360), FHD), (640, 360));
}

#[test]
fn a_game_bigger_than_the_screen_is_trimmed_to_fit_it() {
    let p = presentation(WindowSize::Internal, None);
    // A 4K game on a 1080p laptop.
    assert_eq!(p.window_size((3840, 2160), FHD), (1728, 972));
    // And on a 1366×768 one, where the limit is the height.
    let size = p.window_size((1920, 1080), Some((1366, 768)));
    assert_eq!(size, (1229, 691));
    assert!(
        size.0 <= 1366 && size.1 <= 768,
        "{size:?} is off the screen"
    );
}

#[test]
fn fit_monitor_takes_the_whole_screen_at_the_games_shape() {
    // Asked for the screen, so no allowance is kept back — and no rounding to a
    // whole multiple either, because a project that said "monitor" wants the
    // screen filled rather than square pixels.
    let p = presentation(WindowSize::Monitor, None);
    assert_eq!(p.window_size((1920, 1080), FHD), (1920, 1080));
    assert_eq!(p.window_size((480, 270), FHD), (1920, 1080));
    // A 4:3 game on a 16:9 screen fits to the height and is pillarboxed by the
    // renderer, which is the part that was already right.
    assert_eq!(p.window_size((1024, 768), FHD), (1440, 1080));
}

#[test]
fn a_declared_size_is_honoured_exactly() {
    // Not clamped. A project that names a number means it, and a number quietly
    // changed here is a project being argued with — the frame inside is never
    // cropped either way.
    let p = presentation(WindowSize::Fixed(1280, 720), None);
    assert_eq!(p.window_size((1920, 1080), FHD), (1280, 720));
    assert_eq!(p.window_size((1920, 1080), Some((800, 600))), (1280, 720));
    assert_eq!(p.window_size((1920, 1080), None), (1280, 720));
}

#[test]
fn a_platform_that_will_not_name_a_monitor_gets_the_games_own_size() {
    // Never smaller than the game, which is the property that was broken.
    for window in [WindowSize::Internal, WindowSize::Monitor] {
        let p = presentation(window, None);
        assert_eq!(p.window_size((1920, 1080), None), (1920, 1080));
        assert_eq!(p.window_size((480, 270), None), (480, 270));
    }
}

#[test]
fn nothing_resolves_to_a_window_with_no_area() {
    for window in [
        WindowSize::Internal,
        WindowSize::Monitor,
        WindowSize::Fixed(0, 0),
    ] {
        let p = presentation(window, None);
        for internal in [(0, 0), (1, 1), (1920, 1080)] {
            for monitor in [None, Some((0, 0)), Some((1, 1)), Some((1920, 1080))] {
                let size = p.window_size(internal, monitor);
                assert!(
                    size.0 > 0 && size.1 > 0,
                    "{window:?} {internal:?} {monitor:?}"
                );
            }
        }
    }
}

// -- what a project declares --------------------------------------------

#[test]
fn a_project_can_declare_a_window_size() {
    let (settings, diagnostics) = Settings::parse("[window]\nsize = [1280, 720]\n", "project.toml");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(settings.presentation.window, WindowSize::Fixed(1280, 720));
}

#[test]
fn a_project_can_ask_for_the_monitor() {
    let (settings, diagnostics) = Settings::parse("[window]\nfit = \"monitor\"\n", "project.toml");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(settings.presentation.window, WindowSize::Monitor);

    let (settings, _) = Settings::parse("[window]\nfit = \"internal\"\n", "project.toml");
    assert_eq!(settings.presentation.window, WindowSize::Internal);
}

#[test]
fn declaring_both_is_reported_rather_than_resolved_by_a_rule() {
    // They answer the same question. A precedence rule here is a rule somebody
    // has to remember, and getting it wrong looks like the setting not working.
    let (_, diagnostics) = Settings::parse(
        "[window]\nsize = [1280, 720]\nfit = \"monitor\"\n",
        "project.toml",
    );
    assert!(
        diagnostics
            .0
            .iter()
            .any(|d| d.code == dimetric_core::Code::SETTINGS_INVALID),
        "{diagnostics}"
    );
}

#[test]
fn a_window_section_of_the_wrong_shape_is_reported() {
    for text in [
        "[window]\nsize = 1280\n",
        "[window]\nsize = [1280]\n",
        "[window]\nsize = [0, 720]\n",
        "[window]\nfit = \"biggest\"\n",
        "[window]\nfit = true\n",
    ] {
        let (settings, diagnostics) = Settings::parse(text, "project.toml");
        assert!(diagnostics.has_errors(), "{text:?} was accepted");
        assert_eq!(
            settings.presentation.window,
            WindowSize::Internal,
            "{text:?} changed the setting anyway"
        );
    }
}

#[test]
fn a_project_can_turn_whole_multiple_scaling_off() {
    let (settings, diagnostics) =
        Settings::parse("[render]\ninteger_upscale = false\n", "project.toml");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(settings.presentation.integer_upscale, Some(false));

    let (settings, _) = Settings::parse("[render]\ninteger_upscale = true\n", "project.toml");
    assert_eq!(settings.presentation.integer_upscale, Some(true));

    // Absent means the engine's default rather than false, so a project that
    // never mentions it keeps square pixels.
    let (settings, _) = Settings::parse("", "project.toml");
    assert_eq!(settings.presentation.integer_upscale, None);
}

#[test]
fn integer_upscale_of_the_wrong_type_is_reported() {
    let (settings, diagnostics) =
        Settings::parse("[render]\ninteger_upscale = \"yes\"\n", "project.toml");
    assert!(diagnostics.has_errors(), "{diagnostics}");
    assert_eq!(settings.presentation.integer_upscale, None);
}

#[test]
fn none_of_this_is_part_of_the_replay_contract() {
    // Structural, not a convention: everything a run depends on is a field on
    // `Settings`, and these are behind `Settings::presentation`. A change here
    // cannot reach the simulation's configuration at all.
    let placed = Settings::parse(
        "[window]\nfit = \"monitor\"\n\n[render]\nresolution = [1920, 1080]\n\
         integer_upscale = false\n\n[sim]\ntick_rate = 30\n",
        "project.toml",
    )
    .0;
    let bare = Settings::parse(
        "[render]\nresolution = [1920, 1080]\n\n[sim]\ntick_rate = 30\n",
        "project.toml",
    )
    .0;
    assert_eq!(placed.tick_rate, bare.tick_rate);
    assert_eq!(placed.canvas, bare.canvas);
    assert_eq!(placed.resolution, bare.resolution);
    assert_eq!(placed.bindings, bare.bindings);
    assert_ne!(placed.presentation, bare.presentation);
}

#[test]
fn a_project_can_choose_how_the_last_blit_is_sampled() {
    use dimetric_render::PresentFilter;

    for (text, want) in [
        ("auto", PresentFilter::Auto),
        ("nearest", PresentFilter::Nearest),
        ("linear", PresentFilter::Linear),
    ] {
        let (settings, diagnostics) = Settings::parse(
            &format!("[render]\npresent_filter = \"{text}\"\n"),
            "project.toml",
        );
        assert!(!diagnostics.has_errors(), "{diagnostics}");
        assert_eq!(settings.presentation.present_filter, Some(want));
    }

    // Absent means the engine's default, which is `auto`.
    let (settings, _) = Settings::parse("", "project.toml");
    assert_eq!(settings.presentation.present_filter, None);
}

#[test]
fn a_present_filter_the_engine_does_not_have_is_reported() {
    for text in [
        "[render]\npresent_filter = \"smooth\"\n",
        "[render]\npresent_filter = true\n",
    ] {
        let (settings, diagnostics) = Settings::parse(text, "project.toml");
        assert!(diagnostics.has_errors(), "{text:?} was accepted");
        assert_eq!(settings.presentation.present_filter, None);
    }
}
