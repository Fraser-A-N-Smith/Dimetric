//! A `.meta` that does not parse is refused, not replaced.
//!
//! This is the regression test for data loss. A sidecar carrying an invalid
//! `id` had its parse error dropped by `.ok()`, a fresh default invented in its
//! place, and that default written back over the author's file — no error, no
//! warning, exit status zero. It ate 84 files in one project, each carrying
//! several hand-written `[[clip]]` declarations, and the only reason nothing
//! was lost was version control.
//!
//! The distinction the fix turns on: **absent** means no opinion, and
//! inventing one is helpful; **malformed** means an opinion that did not
//! survive parsing, and inventing one in its place destroys it. Those want
//! opposite recoveries and used to share a code path.

use std::path::Path;

/// A tiny four-frame strip, so a sidecar has something to describe.
fn strip_png() -> Vec<u8> {
    let (cell, frames) = (8u32, 4u32);
    let mut row = vec![0u8];
    for f in 0..frames {
        let v = (f * 60) as u8;
        for _ in 0..cell {
            row.extend_from_slice(&[v, 255 - v, 128, 255]);
        }
    }
    let raw: Vec<u8> = std::iter::repeat_n(row, cell as usize).flatten().collect();
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    };
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = (cell * frames).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&cell.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
    png.extend_from_slice(&chunk(b"IEND", b""));
    png
}

fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(65_535).enumerate() {
        out.push(u8::from((i + 1) * 65_535 >= data.len()));
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

/// A project with one strip and the given sidecar text, or none.
fn project(meta: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let sprites = dir.path().join("assets").join("sprites");
    std::fs::create_dir_all(&sprites).expect("mkdir");
    std::fs::write(sprites.join("bogling.png"), strip_png()).expect("png");
    if let Some(text) = meta {
        std::fs::write(sprites.join("bogling.png.meta"), text).expect("meta");
    }
    dir
}

fn meta_text(root: &Path) -> String {
    std::fs::read_to_string(root.join("assets/sprites/bogling.png.meta")).expect("meta")
}

/// What the generator writes: hand-authored clips, and an id that is not one.
const AUTHORED: &str = r#"id = "sprites_ashfen_bogling"
nearest = true
atlas = true
frames = 4
frame_ms = 120

[[clip]]
name = "idle_ne"
from = 0
to = 1

[[clip]]
name = "attack_ne"
from = 2
to = 3
looping = false
"#;

#[test]
fn a_malformed_meta_is_not_overwritten() {
    // The regression. Before the fix this assertion failed with the file
    // replaced by four lines of defaults.
    let dir = project(Some(AUTHORED));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");

    assert_eq!(
        meta_text(dir.path()),
        AUTHORED,
        "the author's sidecar must survive a failed parse byte for byte"
    );
}

#[test]
fn a_malformed_meta_fails_the_import_and_says_why() {
    // I9: every failure carries a code, a location and a payload. The parse
    // error was structured and got dropped on the floor by `.ok()`.
    let dir = project(Some(AUTHORED));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);

    assert_eq!(imported.failures.len(), 1, "{:?}", imported.failures);
    let (name, why) = &imported.failures[0];
    assert_eq!(name, "sprites/bogling");
    assert!(why.contains("bogling.png.meta"), "names the file: {why}");
    assert!(
        why.contains("sprites_ashfen_bogling"),
        "names the offending value: {why}"
    );
    assert!(
        why.contains("left as it is"),
        "says what it did about it: {why}"
    );
}

#[test]
fn the_scan_remembers_that_the_sidecar_did_not_parse() {
    let dir = project(Some(AUTHORED));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let entry = catalog.get("sprites/bogling").expect("entry");
    assert!(
        entry.meta_error.is_some(),
        "the error has to survive the scan"
    );
}

#[test]
fn an_absent_meta_is_still_invented_and_written() {
    // The other half, and the reason the two cases cannot simply both refuse.
    // Dropping a PNG into `assets/` and having a sidecar appear is documented,
    // useful behaviour that has to keep working.
    let dir = project(None);
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let entry = catalog.get("sprites/bogling").expect("entry");
    assert!(entry.meta_error.is_none(), "absent is not an error");

    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    let written = dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");
    assert_eq!(written, 1);

    let text = meta_text(dir.path());
    assert!(text.starts_with("id = \"a_"), "{text}");
}

#[test]
fn a_good_meta_is_still_updated_with_its_source_hash() {
    // The third case: a sidecar that parses is rewritten to carry the hash the
    // cache was built from, which is how staleness is detected. A fix that
    // stopped writing *any* sidecar would break incremental import.
    let dir = project(Some(
        "id = \"a_00000001\"\nnearest = true\natlas = true\nframes = 4\nframe_ms = 120\n",
    ));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");

    let text = meta_text(dir.path());
    assert!(text.contains("source_hash"), "{text}");
    assert!(text.contains("a_00000001"), "the id must be kept: {text}");
}

#[test]
fn a_meta_with_a_broken_clip_range_is_also_preserved() {
    // The `[[clip]]` errors behind DIM0604 went through the same `.ok()`, so
    // an off-by-one in one range deleted every range in the file.
    let authored =
        "id = \"a_00000001\"\nframes = 4\n\n[[clip]]\nname = \"walk\"\nfrom = 0\nto = 99\n";
    let dir = project(Some(authored));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");

    assert_eq!(imported.failures.len(), 1);
    assert_eq!(meta_text(dir.path()), authored);
}

#[test]
fn a_meta_that_is_not_toml_at_all_is_preserved() {
    let authored = "this is not toml, it is a note somebody left\n";
    let dir = project(Some(authored));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");

    assert_eq!(imported.failures.len(), 1);
    assert_eq!(meta_text(dir.path()), authored);
}

#[test]
fn an_empty_meta_is_preserved_rather_than_filled_in() {
    // Empty is not absent. A zero-byte file is something somebody created,
    // and the missing `id` is a real error rather than a licence to invent.
    let dir = project(Some(""));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    dimetric_assets::cache::write_metas(&catalog, &imported).expect("write");

    assert_eq!(imported.failures.len(), 1, "{:?}", imported.failures);
    assert_eq!(meta_text(dir.path()), "");
}

#[test]
fn fixing_the_sidecar_lets_the_import_proceed() {
    // The recovery path a person actually takes: read the error, correct the
    // id, run it again. Nothing should be left stuck.
    let dir = project(Some(AUTHORED));
    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert_eq!(imported.failures.len(), 1);

    let fixed = AUTHORED.replace("sprites_ashfen_bogling", "a_26ej92lt");
    std::fs::write(dir.path().join("assets/sprites/bogling.png.meta"), &fixed).expect("write");

    let catalog = dimetric_assets::cache::Catalog::scan(dir.path());
    let imported = dimetric_assets::cache::import(&catalog, 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    assert_eq!(
        imported.clips("sprites/bogling").len(),
        2,
        "and the clips the author wrote are the ones that import"
    );
}
