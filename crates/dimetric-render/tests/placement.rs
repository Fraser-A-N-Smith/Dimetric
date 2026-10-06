//! Where the internal image lands in a window, and that none of it is lost.
//!
//! Integer upscaling exists so a pixel-art frame scaled *up* has square
//! pixels: a fractional multiple makes some pixels one screen-pixel wider than
//! their neighbours, which is visible and ugly. Below one it buys nothing,
//! because there is no whole multiple below one — and clamping to 1 there drew
//! the frame at full size in a smaller window and let the edges fall outside
//! it.
//!
//! A 1920×1080 game in the old default 1440×810 window lost 240 pixels from
//! each side and 135 from the top and bottom: a menu button, a right-hand rail
//! and half a bottom bar, with a click aimed at any of them landing on nothing.

use dimetric_render::RenderSettings;

fn settings(internal: (u32, u32), integer: bool) -> RenderSettings {
    RenderSettings {
        internal_resolution: internal,
        integer_upscale: integer,
        ..Default::default()
    }
}

/// The rectangle the frame occupies in the output, as `(x, y, w, h)`.
fn drawn(settings: &RenderSettings, output: (u32, u32)) -> (f32, f32, f32, f32) {
    let (scale, x, y) = settings.placement(output);
    let (iw, ih) = settings.internal_resolution;
    (x, y, iw as f32 * scale, ih as f32 * scale)
}

#[test]
fn a_window_smaller_than_the_game_shows_all_of_it() {
    // The reported case: a 1920x1080 game in a 1440x810 window.
    let s = settings((1920, 1080), true);
    let (scale, x, y) = s.placement((1440, 810));
    assert_eq!(
        scale, 0.75,
        "the frame was drawn at {scale}x in a smaller window"
    );
    assert_eq!((x, y), (0.0, 0.0), "and offset into the corner");
    assert_eq!(drawn(&s, (1440, 810)), (0.0, 0.0, 1440.0, 810.0));
}

#[test]
fn nothing_falls_outside_the_output_at_any_size() {
    // Every window a player might drag to, against both scaling modes. The
    // property is the one that matters and the clamp broke: the frame is
    // inside the output, always.
    for integer in [true, false] {
        for internal in [(1920, 1080), (480, 270), (320, 180), (1024, 768)] {
            let s = settings(internal, integer);
            for output in [
                (1, 1),
                (17, 9),
                (320, 180),
                (640, 480),
                (1279, 719),
                (1440, 810),
                (1920, 1080),
                (1921, 1081),
                (2560, 1440),
                (3840, 2160),
            ] {
                let (x, y, w, h) = drawn(&s, output);
                assert!(
                    x >= 0.0 && y >= 0.0,
                    "{internal:?} in {output:?} starts at ({x}, {y})"
                );
                assert!(
                    x + w <= output.0 as f32 + 0.5 && y + h <= output.1 as f32 + 0.5,
                    "{internal:?} in {output:?} draws {w}x{h} at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn scaling_up_is_still_whole_when_a_project_asks_for_it() {
    // The reason integer upscaling exists, unchanged. 480x270 in 1920x1080 is
    // exactly 4x; in 1900x1070 it is 3x with a border rather than 3.95x.
    let s = settings((480, 270), true);
    assert_eq!(s.placement((1920, 1080)).0, 4.0);
    assert_eq!(s.placement((1900, 1070)).0, 3.0);
    let (x, y, w, h) = drawn(&s, (1900, 1070));
    assert_eq!((w, h), (1440.0, 810.0));
    assert_eq!((x, y), (230.0, 130.0), "the border is not centred");
}

#[test]
fn a_project_that_turns_integer_upscaling_off_fills_the_window() {
    let s = settings((480, 270), false);
    assert_eq!(s.placement((1900, 1070)).0, 1900.0 / 480.0);
    let (_, _, w, h) = drawn(&s, (1900, 1070));
    assert!((w - 1900.0).abs() < 0.01, "{w}");
    assert!((h - 1068.75).abs() < 0.01, "{h}");
}

#[test]
fn the_letterbox_still_appears_when_the_shape_differs() {
    // Fitting to the narrower axis and centring the rest is right, and stays.
    let s = settings((1920, 1080), false);
    let (_, x, y) = s.placement((1000, 1000));
    assert_eq!(y, 218.0, "a wide game in a square window is letterboxed");
    assert_eq!(x, 0.0);

    let (_, x, y) = s.placement((4000, 1080));
    assert_eq!(x, 1040.0, "and pillarboxed the other way");
    assert_eq!(y, 0.0);
}

#[test]
fn an_exact_fit_is_untouched() {
    // What every captured frame and every golden image goes through.
    for integer in [true, false] {
        for internal in [(1920, 1080), (480, 270), (64, 64)] {
            let s = settings(internal, integer);
            assert_eq!(s.placement(internal), (1.0, 0.0, 0.0), "{internal:?}");
        }
    }
}

#[test]
fn a_window_with_no_area_does_not_divide_by_zero() {
    // Minimised, or mid-resize. The frame is skipped anyway; this only has to
    // not produce a NaN that reaches a viewport call.
    let s = settings((1920, 1080), true);
    for output in [(0, 0), (0, 1080), (1920, 0)] {
        let (scale, x, y) = s.placement(output);
        assert!(scale.is_finite() && scale > 0.0, "{output:?}: {scale}");
        assert!(x.is_finite() && y.is_finite());
    }
}

#[test]
fn a_project_with_no_internal_resolution_is_not_a_crash() {
    let s = settings((0, 0), true);
    assert_eq!(s.placement((1440, 810)), (1.0, 0.0, 0.0));
}

// -- and back again -----------------------------------------------------

/// Every window size a player might drag to, against a 1080p game.
const WINDOWS: [(u32, u32); 8] = [
    (320, 180),
    (640, 480),
    (1280, 720),
    (1440, 810),
    (1920, 1080),
    (1921, 1081),
    (2560, 1440),
    (1000, 1000),
];

#[test]
fn the_point_a_click_lands_on_is_the_pixel_drawn_under_it() {
    // The property the crop broke: a click aimed at the frame's bottom-right
    // corner used to land 240 pixels to the left of it, because the frame was
    // drawn at full size in a smaller window and the cursor was unprojected
    // through the same wrong scale.
    //
    // Checked by walking the *drawn* rectangle rather than the window: the
    // corners and centre of what is on screen have to map to the corners and
    // centre of the internal resolution.
    for integer in [true, false] {
        let s = settings((1920, 1080), integer);
        let (iw, ih) = (1920.0, 1080.0);
        for output in WINDOWS {
            let (x, y, w, h) = drawn(&s, output);
            let cases = [
                ((x, y), (0.0, 0.0)),
                ((x + w, y + h), (iw, ih)),
                ((x + w * 0.5, y + h * 0.5), (iw * 0.5, ih * 0.5)),
                // The spot the reported click was aimed at: a button 200x64
                // pixels in from the bottom-right of the interface.
                (
                    (x + w - 100.0 / iw * w, y + h - 48.0 / ih * h),
                    (iw - 100.0, ih - 48.0),
                ),
            ];
            for (point, want) in cases {
                let got = s.window_to_internal(point, output);
                let slack = 1.0 / drawn(&s, output).2 * iw + 0.001;
                assert!(
                    (got.0 - want.0).abs() <= slack && (got.1 - want.1).abs() <= slack,
                    "{output:?} integer={integer}: {point:?} landed on {got:?}, wanted {want:?}"
                );
            }
        }
    }
}

#[test]
fn a_click_on_the_letterbox_lands_outside_the_frame() {
    // Not clamped to the edge. Nothing is drawn on the border, so nothing there
    // should be hit — a click clamped onto the nearest button would press one
    // the player was deliberately aiming past.
    let s = settings((1920, 1080), false);
    let (_, _, _, h) = drawn(&s, (1000, 1000));
    let above = s.window_to_internal((500.0, 10.0), (1000, 1000));
    assert!(above.1 < 0.0, "{above:?}");
    let below = s.window_to_internal((500.0, 990.0), (1000, 1000));
    assert!(below.1 > 1080.0, "{below:?} with the frame {h} tall");
}

#[test]
fn the_two_directions_agree_at_every_size() {
    // Placement draws the frame and `window_to_internal` reads the cursor back.
    // They are each other's inverse, which is the only reason a click can be
    // trusted to land where it looks.
    for integer in [true, false] {
        for internal in [(1920, 1080), (480, 270), (1024, 768)] {
            let s = settings(internal, integer);
            for output in WINDOWS {
                let (scale, ox, oy) = s.placement(output);
                for pixel in [
                    (0.0, 0.0),
                    (7.0, 3.0),
                    (internal.0 as f32, internal.1 as f32),
                ] {
                    let in_window = (pixel.0 * scale + ox, pixel.1 * scale + oy);
                    let back = s.window_to_internal(in_window, output);
                    assert!(
                        (back.0 - pixel.0).abs() < 0.01 && (back.1 - pixel.1).abs() < 0.01,
                        "{internal:?} in {output:?}: {pixel:?} -> {in_window:?} -> {back:?}"
                    );
                }
            }
        }
    }
}
