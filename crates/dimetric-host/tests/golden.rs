//! Golden-image tests.
//!
//! Each fixture is rendered headless and compared against a reference checked
//! in beside it. This is the second of the CI gates §8 asks for, and it is what
//! catches a renderer change nobody meant to make.
//!
//! # Why the comparison has a tolerance
//!
//! The M0 spike established that output is byte-identical between runs on one
//! adapter, and could not establish that it matches *across* platforms — one
//! machine had only a software rasteriser. GPUs agree on where a triangle's
//! interior is and differ at its edges, so an exact comparison would be a test
//! that passes on whoever generated the references and fails for everyone else.
//!
//! The thresholds below are deliberately provisional. When CI has reported what
//! three platforms actually produce, tighten them to what the evidence
//! supports.
//!
//! Regenerate references with `DIMETRIC_BLESS=1 cargo test -p dimetric-host
//! --test golden`, and look at the diff before committing it.

use std::path::{Path, PathBuf};

use dimetric_host::{capture, CaptureRequest, Project};
use dimetric_render::RenderSettings;
use dimetric_scene::Color;

/// No pixel may differ from the reference by more than this on any channel.
const MAX_CHANNEL_DELTA: u8 = 32;
/// At most this fraction of pixels may differ at all.
///
/// Edge pixels are where rasterisers disagree, and a frame full of 16-pixel
/// sprites and diamond tiles is mostly edges — hence a limit in percent rather
/// than in pixels.
const MAX_DIFFERING_FRACTION: f64 = 0.02;

struct Fixture {
    scene: &'static str,
    reference: &'static str,
    size: (u32, u32),
    internal: (u32, u32),
    ambient: Color,
}

fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            scene: "room",
            reference: "room-topdown.png",
            size: (256, 192),
            internal: (128, 96),
            ambient: Color::WHITE,
        },
        Fixture {
            scene: "room-lit",
            reference: "room-isometric-lit.png",
            size: (256, 192),
            internal: (128, 96),
            ambient: Color::rgba(0x30, 0x30, 0x40, 0xFF),
        },
    ]
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .canonicalize()
        .expect("tests/golden exists")
}

#[test]
fn every_fixture_matches_its_reference() {
    let root = golden_dir();
    let blessing = std::env::var("DIMETRIC_BLESS").is_ok();
    let mut skipped = false;

    for fixture in fixtures() {
        let mut project = Project::open(root.join("scenes"), 0);
        project
            .load_scene(fixture.scene)
            .unwrap_or_else(|d| panic!("{}: {d}", fixture.scene));

        let request = CaptureRequest {
            tick: 0,
            seed: 0,
            input: None,
            size: fixture.size,
            settings: RenderSettings {
                internal_resolution: fixture.internal,
                integer_upscale: true,
                pixel_snap: true,
                ambient: fixture.ambient,
            },
        };

        let captured = match capture(&mut project, request) {
            Ok(captured) => captured,
            Err(diagnostics) => {
                // No adapter on this machine. CI installs a software one and
                // sets DIMETRIC_REQUIRE_GPU, which turns this into a failure —
                // otherwise a runner that lost its driver goes green having
                // rendered nothing.
                assert!(
                    std::env::var("DIMETRIC_REQUIRE_GPU").is_err(),
                    "DIMETRIC_REQUIRE_GPU is set but rendering failed: {diagnostics}"
                );
                eprintln!("skipping {}: {diagnostics}", fixture.scene);
                skipped = true;
                continue;
            }
        };
        assert!(
            !captured.diagnostics.has_errors(),
            "{}: {}",
            fixture.scene,
            captured.diagnostics
        );

        let reference = root.join("references").join(fixture.reference);
        if blessing {
            dimetric_render::write_png(
                &reference,
                captured.width,
                captured.height,
                &captured.pixels,
            )
            .expect("writing the reference");
            eprintln!("blessed {}", reference.display());
            continue;
        }

        let (width, height, expected) = dimetric_render::read_png(&reference).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\n\nGenerate references with DIMETRIC_BLESS=1.",
                reference.display()
            )
        });
        assert_eq!(
            (width, height),
            (captured.width, captured.height),
            "{} is {width}x{height}, the render is {}x{}",
            fixture.reference,
            captured.width,
            captured.height
        );

        let report = compare(&captured.pixels, &expected);
        assert!(
            report.passes(),
            "{} differs from {}:\n  {}\n\nIf the change was intended, regenerate with \
             DIMETRIC_BLESS=1 and read the diff.",
            fixture.scene,
            fixture.reference,
            report
        );
        eprintln!("{} matches ({})", fixture.scene, report);
    }

    if skipped {
        eprintln!("some fixtures were skipped: no graphics adapter on this machine");
    }
}

/// How far apart two frames are.
struct Report {
    max_delta: u8,
    differing: usize,
    total: usize,
}

impl Report {
    fn passes(&self) -> bool {
        self.max_delta <= MAX_CHANNEL_DELTA && self.fraction() <= MAX_DIFFERING_FRACTION
    }

    fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.differing as f64 / self.total as f64
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} of {} pixels differ ({:.2}%), worst channel off by {} (limits: {:.0}%, {})",
            self.differing,
            self.total,
            self.fraction() * 100.0,
            self.max_delta,
            MAX_DIFFERING_FRACTION * 100.0,
            MAX_CHANNEL_DELTA
        )
    }
}

fn compare(actual: &[u8], expected: &[u8]) -> Report {
    let mut max_delta = 0u8;
    let mut differing = 0usize;
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let delta = a
            .iter()
            .zip(b)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if delta > 0 {
            differing += 1;
            max_delta = max_delta.max(delta);
        }
    }
    Report {
        max_delta,
        differing,
        total: actual.len() / 4,
    }
}
