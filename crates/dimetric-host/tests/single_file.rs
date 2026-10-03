//! A game folded into one file is the same game.
//!
//! `package.rs` stages files beside the runtime and argues for it: the files a
//! game ships are the files it was developed against, byte for byte, which is
//! what makes "it worked on my machine" checkable. But "send me the game" means
//! a folder of several hundred files that breaks the moment somebody drags the
//! executable out of it, and every fix on the game's side — a self-extractor, an
//! unpacker writing to a temporary directory — breaks the property the module
//! protects.
//!
//! So this is a read path. The staged files are appended to a copy of the
//! runtime, unmodified and uncompressed, and read where they lie. The test that
//! matters is the one below: an input log replays to the same hashes against the
//! folder build and the single-file build. If those ever differ, the archive is
//! not a read path any more.

use std::path::Path;

use dimetric_core::Source;
use dimetric_host::archive;
use dimetric_host::{package, Project};
use dimetric_sim::{InputLog, LuaHost, Sim};

/// A project with a scene, a script, a prefab, settings and an asset.
///
/// Every kind of file the runtime reads, because the point is that the archive
/// serves all of them and not just the easy ones.
fn write_project(root: &Path) {
    std::fs::create_dir_all(root.join("scripts")).expect("mkdir");
    std::fs::create_dir_all(root.join("prefabs")).expect("mkdir");
    std::fs::create_dir_all(root.join("assets/sprites")).expect("mkdir");

    std::fs::write(
        root.join("project.toml"),
        "[sim]\ntick_rate = 30\n\n[ui]\ncanvas = [640, 360]\n\n[render]\nresolution = [640, 360]\n",
    )
    .expect("settings");
    std::fs::write(
        root.join("kinds.toml"),
        "[[kind]]\nname = \"Walker\"\nextends = \"Node2D\"\n\n\
         [[kind.property]]\nname = \"speed\"\ntype = \"int\"\ndefault = 3\n",
    )
    .expect("kinds");
    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Stage\"\n\
         script = \"script:scripts/stage.lua\"\n\n\
         [[node]]\nid = \"n_walk0000\"\nkind = \"Walker\"\nname = \"Walker\"\n\
         parent = \"n_root0000\"\nspeed = 7\n",
    )
    .expect("scene");
    std::fs::write(
        root.join("prefabs/mote.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_mote0000\"\n\n\
         [[node]]\nid = \"n_mote0000\"\nkind = \"Node2D\"\nname = \"Mote\"\n",
    )
    .expect("prefab");
    std::fs::write(
        root.join("scripts/stage.lua"),
        "local helper = require(\"scripts/helper.lua\")\n\
         function on_tick(self)\n\
         \x20 self.n = (self.n or 0) + helper.step\n\
         \x20 if tick.count() == 2 then scene.spawn(\"prefabs/mote\", vec2(1, 2)) end\n\
         \x20 self.speed = scene.find(\"/Stage/Walker\"):get(\"speed\")\n\
         \x20 self.roll = rng.range(\"test\", 1, 100)\n\
         end\n",
    )
    .expect("script");
    std::fs::write(root.join("scripts/helper.lua"), "return { step = 3 }\n").expect("module");
    // A real PNG, so the importer has something to chew on.
    std::fs::write(root.join("assets/sprites/dot.png"), tiny_png()).expect("asset");
}

/// A 2x2 opaque PNG.
fn tiny_png() -> Vec<u8> {
    let raw: Vec<u8> = (0..2)
        .flat_map(|_| {
            let mut row = vec![0u8];
            row.extend_from_slice(&[0x40, 0x80, 0xC0, 0xFF].repeat(2));
            row
        })
        .collect();
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    };
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = 2u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&2u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
    png.extend_from_slice(&chunk(b"IEND", b""));
    png
}

fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= data.len();
        out.push(if last { 1 } else { 0 });
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    let mut a: u32 = 1;
    let mut b: u32 = 0;
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
            crc = match crc & 1 != 0 {
                true => (crc >> 1) ^ 0xedb8_8320,
                false => crc >> 1,
            };
        }
    }
    !crc
}

/// Stage the project, then fold it into one file. Returns both.
fn build(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let project_root = dir.join("game");
    write_project(&project_root);

    // Something to stand in for a built `dim-play`. Folding appends to a copy of
    // it, so its contents never matter — only that the bytes in front of the
    // archive are left alone.
    let runtime = dir.join("dim-play");
    std::fs::write(&runtime, b"#!/bin/sh\necho not a real runtime\n").expect("runtime");

    let out = dir.join("staged");
    let mut project = Project::open(&project_root, 0);
    project
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    let staged = package::stage(
        &mut project,
        package::PackageRequest {
            platform: package::platform("linux").expect("linux"),
            scene: "main.dim".to_string(),
            seed: 11,
            out: Some(out.clone()),
            runtime: Some(runtime),
        },
    )
    .unwrap_or_else(|d| panic!("{d}"));

    let folded = package::fold(&staged).unwrap_or_else(|d| panic!("{d}"));
    (out, folded.path)
}

/// Replay a fixed input log over a project and collect a hash per tick.
fn hashes(project: &mut Project, ticks: u64) -> Vec<dimetric_core::StateHash> {
    project.import_assets();
    project
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    let (scene, diagnostics) = project.runtime_scene().unwrap_or_else(|d| panic!("{d}"));
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let script_diagnostics = project.load_scripts();
    assert!(!script_diagnostics.has_errors(), "{script_diagnostics}");

    let mut host = LuaHost::new(project.settings.tick_rate).expect("lua");
    host.set_fonts(project.fonts());
    let failures = host.load_all(
        project
            .scripts
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str())),
    );
    assert!(failures.is_empty(), "{failures:?}");

    let templates = project.templates().0;
    let mut sim = Sim::new(scene, 11, Box::new(host), project.sim_config())
        .with_clips(project.clips())
        .with_templates(templates);
    let log = InputLog::new(11, "test", 1);
    (0..ticks)
        .map(|tick| {
            sim.step(log.frame(tick));
            sim.hash()
        })
        .collect()
}

#[test]
fn a_single_file_build_replays_to_the_same_hashes_as_the_folder() {
    // The acceptance criterion, and the one that makes this a read path rather
    // than a bundler: same seed, same log, same hash every tick, out of a
    // directory and out of an archive appended to an executable.
    let dir = tempfile::tempdir().expect("tempdir");
    let (staged, single) = build(dir.path());

    let folder = hashes(&mut Project::open(&staged, 0), 12);
    let archive = archive::Archive::open(&single)
        .expect("readable")
        .expect("the game is appended");
    let mut packed = Project::open_from(Box::new(archive), &single, 0);
    let one_file = hashes(&mut packed, 12);

    assert_eq!(folder.len(), 12);
    assert_eq!(
        folder, one_file,
        "the folder build and the single-file build are different games"
    );
}

#[test]
fn the_settings_the_replay_contract_lives_in_survive_the_fold() {
    // `project.toml` decides what a run *means*, and this project declares a
    // tick rate and a canvas that are not the defaults — which is how a dropped
    // settings file would show up here rather than as a mystery later.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, single) = build(dir.path());
    let archive = archive::Archive::open(&single)
        .expect("readable")
        .expect("appended");
    let packed = Project::open_from(Box::new(archive), &single, 0);
    assert_eq!(packed.settings.tick_rate, 30);
    assert_eq!(packed.settings.canvas.width, 640);
    assert_eq!(packed.settings.resolution, (640, 360));
}

#[test]
fn a_projects_own_kinds_survive_the_fold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, single) = build(dir.path());
    let archive = archive::Archive::open(&single)
        .expect("readable")
        .expect("appended");
    let mut packed = Project::open_from(Box::new(archive), &single, 0);
    assert!(
        !packed.kind_diagnostics.has_errors(),
        "{}",
        packed.kind_diagnostics
    );
    packed
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("a project kind did not survive: {d}"));
}

#[test]
fn a_shipped_game_knows_it_cannot_be_written_to() {
    // There is nowhere to put a re-imported cache, a saved scene or a written
    // script, and saying so beats writing beside the executable.
    let dir = tempfile::tempdir().expect("tempdir");
    let (staged, single) = build(dir.path());
    let archive = archive::Archive::open(&single)
        .expect("readable")
        .expect("appended");
    assert!(!Project::open_from(Box::new(archive), &single, 0).writable());
    assert!(Project::open(&staged, 0).writable());
}

#[test]
fn the_runtimes_own_bytes_are_untouched() {
    // The whole point of appending: the executable in front of the archive is
    // the executable that was built, so a signed or stripped runtime stays what
    // it was.
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = b"#!/bin/sh\necho not a real runtime\n";
    let (_, single) = build(dir.path());
    let bytes = std::fs::read(&single).expect("readable");
    assert_eq!(&bytes[..runtime.len()], runtime);
}

#[test]
fn an_ordinary_file_has_no_game_in_it() {
    // Asked on every start, so it has to be cheap and it has to be right. A
    // runtime with nothing appended is not an error.
    let dir = tempfile::tempdir().expect("tempdir");
    let plain = dir.path().join("plain");
    std::fs::write(&plain, b"just a file").expect("write");
    assert!(archive::read_manifest(&plain).expect("readable").is_none());
    assert!(archive::Archive::open(&plain).expect("readable").is_none());
    // Including one too short to hold a footer at all.
    let tiny = dir.path().join("tiny");
    std::fs::write(&tiny, b"x").expect("write");
    assert!(archive::read_manifest(&tiny).expect("readable").is_none());
}

#[test]
fn the_payload_hashes_and_a_truncated_build_is_caught() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, single) = build(dir.path());
    let manifest = archive::read_manifest(&single)
        .expect("readable")
        .expect("appended");
    assert!(
        archive::verify(&single, &manifest).expect("readable"),
        "a build does not match its own hash"
    );

    // Change one byte of the payload, which is what a bad copy or an edit looks
    // like, and the hash has to notice.
    let mut bytes = std::fs::read(&single).expect("readable");
    let at = manifest.payload_at as usize;
    bytes[at] ^= 0xff;
    let damaged = dir.path().join("damaged");
    std::fs::write(&damaged, &bytes).expect("write");
    let manifest = archive::read_manifest(&damaged)
        .expect("readable")
        .expect("appended");
    assert!(!archive::verify(&damaged, &manifest).expect("readable"));
}

#[test]
fn every_staged_file_but_the_runtime_is_in_the_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (staged, single) = build(dir.path());
    let archive = archive::Archive::open(&single)
        .expect("readable")
        .expect("appended");

    // Walk the staged directory and expect each file, minus the runtime, which
    // is not *in* the archive — it is the file the archive is appended to.
    let on_disk = dimetric_core::Directory::new(&staged);
    let mut missing = Vec::new();
    for path in on_disk.list("") {
        if path == "game" {
            continue;
        }
        if !archive.exists(&path) {
            missing.push(path);
        }
    }
    assert!(missing.is_empty(), "not folded in: {missing:?}");
    assert!(archive.exists("project.toml"));
    assert!(archive.exists("scripts/helper.lua"));
    assert!(archive.exists("prefabs/mote.dim"));
    assert!(archive.exists(package::MANIFEST));
}
