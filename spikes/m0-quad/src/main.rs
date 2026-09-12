//! **M0 — de-risking spike. Throwaway code.**
//!
//! The design document is explicit that this gets thrown away once it has done
//! its job. Its job is to answer four questions before four weeks go into a
//! renderer:
//!
//! 1. Does wgpu come up on Windows, macOS and Linux, including on a build
//!    machine with no GPU?
//! 2. Can the windowed and headless paths be the same code? If not, an agent's
//!    screenshot stops being evidence about what a human sees.
//! 3. Does the engine's own projection matrix produce the right picture when a
//!    GPU applies it, rather than only when a unit test does?
//! 4. Is GPU output identical across platforms — which decides whether the
//!    golden-image CI job can compare exactly or needs a tolerance.
//!
//! Nothing here should be copied into `dimetric-render`. The answers should.
//!
//! ```sh
//! cargo run -- --headless --out frame.png    # what CI runs
//! cargo run                                  # a window, for a person
//! ```

mod gpu;
mod headless;
mod window;

use std::process::ExitCode;

use dimetric_render::Projection;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    if flag("--help") || flag("-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let projection = match value("--projection").as_deref() {
        Some("isometric") => Projection::Isometric,
        Some("top-down") | None => Projection::TopDown,
        Some(other) => {
            eprintln!("m0: unknown projection {other:?}; expected top-down or isometric");
            return ExitCode::FAILURE;
        }
    };
    let width: u32 = value("--width").and_then(|v| v.parse().ok()).unwrap_or(320);
    let height: u32 = value("--height")
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    let result = if flag("--headless") {
        run_headless(width, height, projection, value("--out"))
    } else {
        window::run(width, height, projection)
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("m0: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The bounding box of everything that is not background.
fn drawn_bounds(pixels: &[u8], width: u32, height: u32, background: &[u8]) -> (u32, u32) {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0u32, 0u32);
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            if pixels[i..i + 3] != *background {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if max_x < min_x {
        return (0, 0);
    }
    (max_x - min_x + 1, max_y - min_y + 1)
}

/// The bounding box the quad should produce under each projection.
///
/// The quad is 64 world units across at zoom 2, so 128 pixels square in
/// top-down. The shear maps `(x, y)` to `(x - y, (x + y) / 2)`, which stretches
/// that square into a diamond 256 across and 128 tall.
fn expected_bounds(projection: Projection) -> (u32, u32) {
    match projection {
        Projection::TopDown => (128, 128),
        Projection::Isometric => (256, 128),
    }
}

const USAGE: &str = "\
m0-quad — the M0 de-risking spike

  --headless           render offscreen instead of opening a window
  --out <path>         write the headless frame as a PNG
  --width/--height N   target size (default 320x180)
  --projection top-down|isometric
";

fn run_headless(
    width: u32,
    height: u32,
    projection: Projection,
    out: Option<String>,
) -> Result<(), String> {
    let pixels = headless::render(width, height, projection)?;

    // Assert the frame actually contains the quad, rather than trusting that a
    // clean exit means something was drawn. A silently black frame is the most
    // likely failure on a software adapter, and the easiest to miss.
    //
    // The background is sampled from a corner rather than hard-coded. The clear
    // colour is given in linear space and stored sRGB-encoded, so
    // `Color { r: 0.05, .. }` comes back as 63, not 13 — while texture bytes
    // written through `write_texture` are stored verbatim. Hard-coding the
    // expected value gets that asymmetry wrong, as the first draft of this
    // check did.
    let background = &pixels[0..3];
    let drawn = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] != *background)
        .count();
    let total = (width * height) as usize;
    let coverage = drawn as f64 / total as f64;
    if drawn == 0 {
        return Err("the frame is entirely background; nothing was drawn".into());
    }
    eprintln!(
        "m0: {drawn}/{total} pixels drawn ({:.1}% of the frame)",
        coverage * 100.0
    );

    // Area alone cannot tell the two projections apart: the 2:1 shear has a
    // determinant of 1, so it turns the square into a diamond of exactly the
    // same area. (The first draft of this check asserted half the area and was
    // simply wrong.) The bounding box does distinguish them — 128x128 becomes
    // 256x128 — so it catches a shear that never reached the GPU.
    let (box_width, box_height) = drawn_bounds(&pixels, width, height, background);
    let (want_width, want_height) = expected_bounds(projection);
    eprintln!("m0: quad bounding box {box_width}x{box_height}");
    let off_by = |a: u32, b: u32| a.abs_diff(b) > 2;
    if off_by(box_width, want_width) || off_by(box_height, want_height) {
        return Err(format!(
            "the quad's bounding box is {box_width}x{box_height}, expected \
             about {want_width}x{want_height}; the projection matrix is wrong"
        ));
    }

    if let Some(path) = out {
        headless::write_png(&path, width, height, &pixels)?;
        eprintln!("m0: wrote {path}");
    }
    Ok(())
}
