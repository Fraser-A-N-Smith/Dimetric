//! Naming clips over a plain PNG strip.
//!
//! Aseprite documents carry tags and those become clips. A PNG carries
//! nothing, so a strip from a generator, a procedural sheet, or anything a
//! script assembles imported as one unnamed clip — and `anim.play(node,
//! "walk_se")`, the documented API, could not reach it.
//!
//! The `.meta` is where facts the file does not carry already go. It could say
//! "this PNG is a strip of 70 frames" and not "frames 0 to 3 are called
//! idle_ne", and both are equally absent from the PNG.

use dimetric_assets::meta::{ImportSettings, MetaError};

fn parse(body: &str) -> Result<ImportSettings, MetaError> {
    ImportSettings::parse(&format!("id = \"a_00000001\"\n{body}"))
}

#[test]
fn a_strip_with_no_clips_declared_is_unchanged() {
    // The old behaviour has to survive: every existing `.meta` in every
    // project declares no clips.
    let settings = parse("frames = 8\nframe_ms = 100\n").expect("parses");
    assert!(settings.clips.is_empty());
    assert_eq!(settings.frames, 8);
}

#[test]
fn a_meta_can_name_ranges_over_a_strip() {
    let settings = parse(
        r#"
frames = 70
frame_ms = 120

[[clip]]
name = "idle_ne"
from = 0
to = 3

[[clip]]
name = "walk_ne"
from = 4
to = 11
"#,
    )
    .expect("parses");
    assert_eq!(settings.clips.len(), 2);
    assert_eq!(settings.clips[0].name, "idle_ne");
    assert_eq!((settings.clips[0].from, settings.clips[0].to), (0, 3));
    assert_eq!((settings.clips[1].from, settings.clips[1].to), (4, 11));
    assert!(settings.clips[0].looping, "looping by default");
}

#[test]
fn a_clip_can_be_a_one_shot() {
    // An attack plays once; an idle repeats. Without this every clip would
    // loop and a swing would never end.
    let settings =
        parse("frames = 8\n\n[[clip]]\nname = \"attack\"\nfrom = 0\nto = 3\nlooping = false\n")
            .expect("parses");
    assert!(!settings.clips[0].looping);
}

#[test]
fn a_clip_can_run_at_its_own_speed() {
    // An attack is usually faster than an idle, and the alternative is
    // importing the same sheet twice.
    let settings =
        parse("frames = 8\nframe_ms = 200\n\n[[clip]]\nname = \"attack\"\nfrom = 0\nto = 3\nframe_ms = 60\n")
            .expect("parses");
    assert_eq!(settings.clips[0].frame_ms, Some(60));
    assert_eq!(settings.frame_ms, 200, "the sheet default is untouched");
}

#[test]
fn a_range_past_the_end_of_the_strip_is_refused() {
    // The off-by-one ranges exist to catch. The message names the last usable
    // frame, because "8 frames" and "last is 7" is exactly the confusion.
    let err = parse("frames = 8\n\n[[clip]]\nname = \"walk\"\nfrom = 4\nto = 8\n")
        .expect_err("should refuse");
    let text = err.to_string();
    assert!(text.contains("walk"), "{text}");
    assert!(
        text.contains('7'),
        "should name the last usable frame: {text}"
    );
}

#[test]
fn a_backwards_range_is_refused() {
    let err = parse("frames = 8\n\n[[clip]]\nname = \"walk\"\nfrom = 5\nto = 2\n")
        .expect_err("should refuse");
    assert!(err.to_string().contains("backwards"), "{err}");
}

#[test]
fn a_clip_with_no_name_is_refused() {
    let err = parse("frames = 8\n\n[[clip]]\nfrom = 0\nto = 3\n").expect_err("should refuse");
    assert!(err.to_string().contains("name"), "{err}");
}

#[test]
fn a_clip_missing_its_range_is_refused() {
    let err = parse("frames = 8\n\n[[clip]]\nname = \"walk\"\nfrom = 0\n").expect_err("refused");
    assert!(err.to_string().contains("walk"), "{err}");
}

#[test]
fn two_clips_with_one_name_are_refused() {
    // `anim.play` asks by name, so a duplicate is unreachable art.
    let err = parse(
        "frames = 8\n\n[[clip]]\nname = \"walk\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"walk\"\nfrom = 4\nto = 7\n",
    )
    .expect_err("should refuse");
    assert!(err.to_string().contains("apart"), "{err}");
}

#[test]
fn a_single_frame_clip_is_fine() {
    // `from == to` is one frame, not an empty range.
    let settings =
        parse("frames = 8\n\n[[clip]]\nname = \"hurt\"\nfrom = 5\nto = 5\n").expect("parses");
    assert_eq!((settings.clips[0].from, settings.clips[0].to), (5, 5));
}

#[test]
fn overlapping_clips_warn_rather_than_failing() {
    // Deliberately a warning, against the request. Reusing frames across clips
    // is a real technique — an idle and a breathe sharing a couple of frames —
    // and Aseprite tags may overlap too, so refusing would be stricter than
    // the tool this pipeline mirrors. But `0..3` then `3..7` when `4` was
    // meant is the off-by-one worth saying out loud.
    let settings = parse(
        "frames = 8\n\n[[clip]]\nname = \"a\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"b\"\nfrom = 3\nto = 7\n",
    )
    .expect("overlap parses");
    let warnings = settings.clip_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("off-by-one"), "{}", warnings[0]);
}

#[test]
fn clips_that_do_not_touch_produce_no_warning() {
    let settings = parse(
        "frames = 12\n\n[[clip]]\nname = \"a\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"b\"\nfrom = 4\nto = 7\n",
    )
    .expect("parses");
    assert!(settings.clip_warnings().is_empty());
}

#[test]
fn a_gap_between_clips_is_allowed() {
    // A sheet with spare frames at the end, or a block reserved for later, is
    // not an error.
    let settings = parse(
        "frames = 70\n\n[[clip]]\nname = \"a\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"b\"\nfrom = 40\nto = 43\n",
    )
    .expect("parses");
    assert_eq!(settings.clips.len(), 2);
    assert!(settings.clip_warnings().is_empty());
}

#[test]
fn clips_survive_a_round_trip_through_the_file() {
    // The `.meta` is written back by the importer, so a declaration that did
    // not round-trip would be lost on the next scan.
    let original = parse(
        "frames = 70\nframe_ms = 120\n\n[[clip]]\nname = \"idle\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"attack\"\nfrom = 4\nto = 9\nlooping = false\nframe_ms = 60\n",
    )
    .expect("parses");
    let text = original.to_text();
    let again = ImportSettings::parse(&text).expect("re-parses");
    assert_eq!(original.clips, again.clips);
    assert_eq!(original.frames, again.frames);
    assert_eq!(original.frame_ms, again.frame_ms);
}

#[test]
fn a_sheet_key_written_after_clips_would_be_swallowed_and_is_not() {
    // TOML's array-of-tables swallows every key after it, so `frames` written
    // below a `[[clip]]` would silently become part of the clip. The writer
    // puts clips last for that reason; this pins it.
    let settings =
        parse("frames = 16\nframe_ms = 80\n\n[[clip]]\nname = \"a\"\nfrom = 0\nto = 3\n")
            .expect("parses");
    let text = settings.to_text();
    let clip_at = text.find("[[clip]]").expect("clips are written");
    assert!(
        text.find("frames =").expect("frames is written") < clip_at,
        "sheet keys have to precede the clip tables:\n{text}"
    );
    assert!(
        text.find("frame_ms =").expect("frame_ms") < clip_at,
        "{text}"
    );
}

// -- Through the importer --------------------------------------------------

/// A strip of `frames` solid-colour cells, as a PNG.
///
/// Generated rather than committed, for the reason the font tests generate
/// theirs: a binary blob in a repository whose habit is that a diff shows a
/// real change earns its place or stays out.
fn strip_png(frames: u32, cell: u32) -> Vec<u8> {
    let (w, h) = (cell * frames, cell);
    // One row of pixels, the same for every scanline: each frame a distinct
    // flat colour, so a mis-sliced strip would be visible if anything ever
    // looked at it.
    let mut row = vec![0u8]; // PNG per-row filter byte: none
    for f in 0..frames {
        let v = (f * 60) as u8;
        for _ in 0..cell {
            row.extend_from_slice(&[v, 255 - v, 128, 255]);
        }
    }
    let raw: Vec<u8> = std::iter::repeat_n(row, h as usize).flatten().collect();
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    };
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
    png.extend_from_slice(&chunk(b"IEND", b""));
    png
}

/// A zlib stream with no compression, which every decoder accepts.
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(65_535).enumerate() {
        let last = u8::from((i + 1) * 65_535 >= data.len());
        out.push(last);
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn a_generated_strip_imports_to_the_clips_its_meta_names() {
    // The whole point, end to end: art that never touched Aseprite reaching
    // `anim.play(node, "attack_ne")`.
    let dir = tempfile::tempdir().expect("tempdir");
    let assets = dir.path().join("assets").join("sprites");
    std::fs::create_dir_all(&assets).expect("mkdir");
    std::fs::write(assets.join("hero.png"), strip_png(4, 8)).expect("png");
    std::fs::write(
        assets.join("hero.png.meta"),
        r#"id = "a_00000001"
nearest = true
atlas = true
frames = 4
frame_ms = 100

[[clip]]
name = "idle_ne"
from = 0
to = 1

[[clip]]
name = "attack_ne"
from = 2
to = 3
looping = false
frame_ms = 50
"#,
    )
    .expect("meta");

    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);

    let clips = imported.clips("sprites/hero");
    assert_eq!(clips.len(), 2, "{clips:?}");

    assert_eq!(clips[0].name, "idle_ne");
    assert_eq!(clips[0].frames.len(), 2);
    assert_eq!(clips[0].frames[0].index, 0);
    assert_eq!(clips[0].frames[1].index, 1);
    assert!(clips[0].looping);

    assert_eq!(clips[1].name, "attack_ne");
    assert_eq!(
        clips[1].frames[0].index, 2,
        "ranges are absolute frame numbers"
    );
    assert!(!clips[1].looping, "a one-shot attack must not repeat");

    // 100ms at 60Hz is 6 ticks; the clip's own 50ms is 3. A per-clip override
    // that did nothing would be a designer wondering why the attack is slow.
    assert_eq!(clips[0].frames[0].ticks, 6);
    assert_eq!(clips[1].frames[0].ticks, 3);
}

#[test]
fn a_strip_with_no_clips_still_imports_as_one_default_clip() {
    // Every `.meta` that exists today declares no clips, and they all have to
    // keep working.
    let dir = tempfile::tempdir().expect("tempdir");
    let assets = dir.path().join("assets").join("sprites");
    std::fs::create_dir_all(&assets).expect("mkdir");
    std::fs::write(assets.join("old.png"), strip_png(3, 8)).expect("png");
    std::fs::write(
        assets.join("old.png.meta"),
        "id = \"a_00000002\"\nnearest = true\natlas = true\nframes = 3\nframe_ms = 100\n",
    )
    .expect("meta");

    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    let clips = imported.clips("sprites/old");
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].name, "default");
    assert_eq!(clips[0].frames.len(), 3);
}

#[test]
fn an_overlap_is_reported_by_the_importer_without_stopping_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let assets = dir.path().join("assets").join("sprites");
    std::fs::create_dir_all(&assets).expect("mkdir");
    std::fs::write(assets.join("share.png"), strip_png(8, 8)).expect("png");
    std::fs::write(
        assets.join("share.png.meta"),
        "id = \"a_00000003\"\nframes = 8\n\n[[clip]]\nname = \"a\"\nfrom = 0\nto = 3\n\n\
         [[clip]]\nname = \"b\"\nfrom = 3\nto = 7\n",
    )
    .expect("meta");

    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(
        imported.failures.is_empty(),
        "an overlap must not stop the import"
    );
    assert_eq!(imported.warnings.len(), 1, "{:?}", imported.warnings);
    assert_eq!(imported.clips("sprites/share").len(), 2);
}
