//! The import pipeline: scanning, identity, staleness and packing.

use std::path::{Path, PathBuf};

use dimetric_assets::{cache, Catalog, SourceKind};

/// A throwaway project directory.
struct Project(PathBuf);

impl Project {
    fn new(name: &str) -> Project {
        let dir = std::env::temp_dir().join(format!(
            "dimetric-assets-{name}-{}",
            std::process::id() as u64 * 31 + name.len() as u64
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).expect("temp dir");
        Project(dir)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    /// Write a solid-colour PNG at `assets/<name>.png`.
    fn png(&self, name: &str, width: u32, height: u32, rgba: [u8; 4]) {
        let path = self.0.join("assets").join(format!("{name}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("asset dir");
        let pixels: Vec<u8> = rgba
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect();
        dimetric_assets::encode_png(&path, &pixels, width, height).expect("write png");
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
        std::fs::write(path, text).expect("write");
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_scan_names_assets_the_way_a_scene_refers_to_them() {
    let project = Project::new("names");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    project.png("tilesets/dungeon", 8, 8, [0, 255, 0, 255]);

    let catalog = Catalog::scan(project.root());
    let names: Vec<&str> = catalog.entries().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["sprites/hero", "tilesets/dungeon"]);
    assert_eq!(
        catalog.get("sprites/hero").unwrap().kind,
        SourceKind::Png,
        "classified by extension"
    );
}

#[test]
fn a_file_that_is_not_an_asset_is_not_catalogued() {
    let project = Project::new("ignored");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    project.write("assets/sprites/notes.txt", "hand-off notes\n");

    let catalog = Catalog::scan(project.root());
    assert_eq!(catalog.len(), 1);
}

#[test]
fn an_id_is_derived_once_and_then_read_back() {
    let project = Project::new("identity");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);

    let first = Catalog::scan(project.root());
    let derived = first.get("sprites/hero").unwrap().settings.id;
    let imported = dimetric_assets::import(&first, 60);
    cache::write_metas(&first, &imported).expect("write metas");

    let second = Catalog::scan(project.root());
    assert_eq!(second.get("sprites/hero").unwrap().settings.id, derived);
    assert!(
        project.root().join("assets/sprites/hero.png.meta").exists(),
        "the sidecar is written next to the source"
    );
}

#[test]
fn renaming_a_file_keeps_its_id() {
    // This is the whole reason identity lives in the sidecar. If the id were
    // derived from the path, every scene referencing the asset would break the
    // moment someone tidied a folder.
    let project = Project::new("rename");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    let catalog = Catalog::scan(project.root());
    let id = catalog.get("sprites/hero").unwrap().settings.id;
    let imported = dimetric_assets::import(&catalog, 60);
    cache::write_metas(&catalog, &imported).expect("write metas");

    let assets = project.root().join("assets/sprites");
    std::fs::rename(assets.join("hero.png"), assets.join("protagonist.png")).expect("rename");
    std::fs::rename(
        assets.join("hero.png.meta"),
        assets.join("protagonist.png.meta"),
    )
    .expect("rename meta");

    let after = Catalog::scan(project.root());
    assert!(after.get("sprites/hero").is_none());
    assert_eq!(after.get("sprites/protagonist").unwrap().settings.id, id);
}

#[test]
fn an_asset_is_stale_until_its_recorded_hash_matches_the_file() {
    let project = Project::new("staleness");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);

    let first = Catalog::scan(project.root());
    assert!(
        first.get("sprites/hero").unwrap().is_stale(),
        "never imported"
    );
    let imported = dimetric_assets::import(&first, 60);
    cache::write_metas(&first, &imported).expect("write metas");

    let second = Catalog::scan(project.root());
    assert!(!second.get("sprites/hero").unwrap().is_stale());

    project.png("sprites/hero", 4, 4, [0, 0, 255, 255]);
    let third = Catalog::scan(project.root());
    assert!(
        third.get("sprites/hero").unwrap().is_stale(),
        "content moved"
    );
}

#[test]
fn hot_reload_asks_which_assets_differ_by_content() {
    let project = Project::new("changed");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    project.png("sprites/skeleton", 4, 4, [0, 255, 0, 255]);
    let before = Catalog::scan(project.root());

    // Rewritten byte for byte: no reload. A build step that touches every file
    // should not make the engine reimport the project.
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    assert!(Catalog::scan(project.root())
        .changed_since(&before)
        .is_empty());

    project.png("sprites/hero", 4, 4, [0, 0, 255, 255]);
    project.png("sprites/ghost", 4, 4, [255, 255, 255, 128]);
    std::fs::remove_file(project.root().join("assets/sprites/skeleton.png")).expect("remove");

    assert_eq!(
        Catalog::scan(project.root()).changed_since(&before),
        ["sprites/ghost", "sprites/hero", "sprites/skeleton"],
        "changed, added and removed all count"
    );
}

#[test]
fn importing_packs_every_image_into_one_sheet() {
    let project = Project::new("packing");
    project.png("sprites/hero", 16, 16, [255, 0, 0, 255]);
    project.png("sprites/skeleton", 8, 24, [0, 255, 0, 255]);
    project.png("tilesets/dungeon", 32, 4, [0, 0, 255, 255]);

    let imported = dimetric_assets::import(&Catalog::scan(project.root()), 60);
    assert!(imported.failures.is_empty(), "{:?}", imported.failures);
    assert_eq!(imported.sheet.placements.len(), 3);
    assert_eq!(
        imported.sheet.placements["sprites/skeleton"].height, 24,
        "sizes survive packing"
    );

    // Nothing overlaps.
    let boxes: Vec<_> = imported.sheet.placements.values().collect();
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            let apart = a.x + a.width <= b.x
                || b.x + b.width <= a.x
                || a.y + a.height <= b.y
                || b.y + b.height <= a.y;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn packing_the_same_project_twice_lays_it_out_the_same_way() {
    // A golden image of a differently packed atlas is a different image, so the
    // layout has to be a function of the inputs and nothing else.
    let project = Project::new("determinism");
    for i in 0..8 {
        project.png(&format!("sprites/s{i}"), 3 + i, 5 + (i % 3), [i as u8; 4]);
    }
    let once = dimetric_assets::import(&Catalog::scan(project.root()), 60);
    let twice = dimetric_assets::import(&Catalog::scan(project.root()), 60);
    assert_eq!(once.sheet, twice.sheet);
}

#[test]
fn an_asset_marked_out_of_the_atlas_stays_out_of_it() {
    let project = Project::new("no-atlas");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    project.png("ui/cursor", 4, 4, [255, 255, 255, 255]);
    project.write(
        "assets/ui/cursor.png.meta",
        "id = \"a_cursor01\"\nnearest = true\natlas = false\n",
    );

    let imported = dimetric_assets::import(&Catalog::scan(project.root()), 60);
    assert!(
        imported.artifacts.contains_key("ui/cursor"),
        "still imported"
    );
    assert!(!imported.sheet.placements.contains_key("ui/cursor"));
}

#[test]
fn a_broken_source_is_reported_and_does_not_stop_the_rest() {
    let project = Project::new("broken");
    project.png("sprites/hero", 4, 4, [255, 0, 0, 255]);
    project.write("assets/sprites/corrupt.png", "this is not a PNG");

    let catalog = Catalog::scan(project.root());
    let imported = dimetric_assets::import(&catalog, 60);
    assert_eq!(imported.failures.len(), 1);
    assert_eq!(imported.failures[0].0, "sprites/corrupt");
    assert!(imported.sheet.placements.contains_key("sprites/hero"));

    // And the failure is not recorded as done, so the next run tries again.
    cache::write_metas(&catalog, &imported).expect("write metas");
    let after = Catalog::scan(project.root());
    assert!(after.get("sprites/corrupt").unwrap().is_stale());
    assert!(!after.get("sprites/hero").unwrap().is_stale());
}

#[test]
fn the_cache_writes_a_sheet_and_a_manifest_and_ignores_itself() {
    let project = Project::new("cache");
    project.png("sprites/hero", 6, 6, [255, 0, 0, 255]);
    let imported = dimetric_assets::import(&Catalog::scan(project.root()), 60);
    cache::write_cache(project.root(), &imported).expect("write cache");

    let sheet = cache::sheet_path(project.root());
    assert!(sheet.exists());
    let reread = dimetric_assets::decode_png(&sheet).expect("the cached sheet is a valid PNG");
    assert_eq!(
        (reread.width, reread.height),
        (imported.sheet.width, imported.sheet.height)
    );

    let manifest =
        std::fs::read_to_string(project.root().join(".import/atlas.json")).expect("manifest");
    assert!(manifest.contains("sprites/hero"));
    assert_eq!(
        std::fs::read_to_string(project.root().join(".import/.gitignore")).unwrap(),
        "*\n",
        "a derived directory ignores itself rather than trusting the project to"
    );
}

#[test]
fn indexed_pngs_decode_because_that_is_what_pixel_art_tools_export() {
    let project = Project::new("indexed");
    let path = project.root().join("assets/sprites/indexed.png");
    std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
    let file = std::fs::File::create(&path).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 2, 1);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(vec![255, 0, 0, 0, 0, 255]);
    encoder.set_trns(vec![255, 128]);
    encoder
        .write_header()
        .expect("header")
        .write_image_data(&[0, 1])
        .expect("data");

    let image = dimetric_assets::decode_png(&path).expect("indexed PNGs are supported");
    assert_eq!(image.pixels, [255, 0, 0, 255, 0, 0, 255, 128]);
}
