//! Choosing a canvas height for the window's shape.
//!
//! A project fixes one canvas and the engine scales it to the window with
//! letterboxing, which is right for a pixel-art game on a monitor and wasteful
//! on a 16:10 handheld: a 16:9 canvas on a 1280x800 screen is drawn at
//! 1280x720 with bars above and below, and a game cannot lay its rails into
//! the eighty rows it is not allowed to use.
//!
//! The arithmetic is integer on purpose. This value decides which control a
//! click lands on, and a replay has to reproduce the choice exactly — so the
//! comparison is the aspect difference cross-multiplied rather than a ratio
//! with a rounding question in it.

use dimetric_scene::ui::Canvas;

/// 1920 wide, which is the width every height below shares.
fn wide() -> Canvas {
    Canvas {
        width: 1920,
        height: 1080,
    }
}

#[test]
fn no_declared_heights_is_exactly_the_project_canvas() {
    // What every project until now gets, and the reason the default is empty
    // rather than a list of one.
    assert_eq!(wide().for_window(&[], (1280, 800)), wide());
    assert_eq!(wide().for_window(&[], (3840, 2160)), wide());
}

#[test]
fn a_sixteen_by_nine_window_takes_the_sixteen_by_nine_height() {
    let got = wide().for_window(&[1080, 1200], (1920, 1080));
    assert_eq!(got.height, 1080);
    assert_eq!(got.width, 1920, "the width never varies");
    // And at another size of the same shape, because it is the shape that
    // decides and not the size.
    assert_eq!(wide().for_window(&[1080, 1200], (1280, 720)).height, 1080);
    assert_eq!(wide().for_window(&[1080, 1200], (3840, 2160)).height, 1080);
}

#[test]
fn a_sixteen_by_ten_window_takes_the_taller_height() {
    // The Steam Deck, which is the case this exists for: 1920x1200 is 16:10,
    // so the whole screen is used and there are no bars.
    let got = wide().for_window(&[1080, 1200], (1280, 800));
    assert_eq!(got.height, 1200);
    assert_eq!(got.width, 1920);
}

#[test]
fn an_odd_window_takes_the_nearest_rather_than_refusing() {
    // Letterboxing still happens for a shape nobody declared; what the choice
    // buys is the *least* of it. 1280x768 is 5:3, between the two, and nearer
    // 16:10.
    assert_eq!(wide().for_window(&[1080, 1200], (1280, 768)).height, 1200);
    // 4:3 is nearer 16:10 than 16:9 as well, and taller is the right answer:
    // a squarer screen wants more rows.
    assert_eq!(wide().for_window(&[1080, 1200], (1024, 768)).height, 1200);
    // And a very wide screen wants the shortest declared height.
    assert_eq!(wide().for_window(&[1080, 1200], (3440, 1440)).height, 1080);
}

#[test]
fn a_tie_keeps_the_earlier_declaration() {
    // So the order a project writes its heights in is the order it means.
    // 1920x1000 is exactly between 1080 and 920 in this measure.
    let base = Canvas {
        width: 1920,
        height: 1000,
    };
    assert_eq!(base.for_window(&[1080, 920], (1920, 1000)).height, 1080);
    assert_eq!(base.for_window(&[920, 1080], (1920, 1000)).height, 920);
}

#[test]
fn a_window_with_no_area_is_the_project_canvas() {
    // A window can be reported as zero-sized while it is being created, and a
    // canvas of nothing would divide a layout by zero.
    assert_eq!(wide().for_window(&[1080, 1200], (0, 800)), wide());
    assert_eq!(wide().for_window(&[1080, 1200], (1280, 0)), wide());
}

#[test]
fn a_declared_height_of_zero_is_skipped_rather_than_chosen() {
    // The settings reader refuses one, so this is belt and braces -- but a
    // canvas of no height draws nothing and lays out nothing, and silently
    // choosing it would be the worst available answer.
    assert_eq!(wide().for_window(&[0, 1200], (1280, 800)).height, 1200);
    assert_eq!(
        wide().for_window(&[0], (1280, 800)),
        wide(),
        "nothing usable declared is the project's own"
    );
}

#[test]
fn one_declared_height_is_always_that_height() {
    // A project may declare exactly one, which is a project saying "this shape
    // whatever the screen" in the new vocabulary rather than the old.
    assert_eq!(wide().for_window(&[1200], (1920, 1080)).height, 1200);
    assert_eq!(wide().for_window(&[1200], (1280, 800)).height, 1200);
}

#[test]
fn the_choice_is_the_same_whatever_order_the_window_arrives_in() {
    // Total and deterministic: the same window and the same list give the same
    // answer every time, which is what lets a recording carry it.
    for _ in 0..8 {
        assert_eq!(
            wide().for_window(&[1080, 1200, 1440], (1280, 800)).height,
            1200
        );
    }
}
