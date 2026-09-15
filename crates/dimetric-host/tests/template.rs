//! The project `dim new` writes.
//!
//! A template nobody can run is worse than no template, so these check that
//! what comes out loads, simulates and is already in canonical form.

use dimetric_host::{template, Project};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dimetric-template-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_new_project_loads_and_runs() {
    let root = scratch("runs");
    let created = template::create(&root, "Testbed").unwrap_or_else(|d| panic!("{d}"));
    assert!(created.files.iter().any(|f| f == "main.dim"));

    let mut project = Project::open(&root, 0);
    let diagnostics = project
        .load_scene("main.dim")
        .unwrap_or_else(|d| panic!("{d}"));
    assert!(!diagnostics.has_errors(), "{diagnostics}");

    let scene = project.open.as_ref().expect("a scene is open");
    assert_eq!(
        scene
            .scene
            .root()
            .and_then(|id| scene.scene.get(id))
            .map(|n| n.name.as_str()),
        Some("Testbed")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_project_is_already_canonical() {
    // The first thing a contributor runs is the gate. A template that fails it
    // teaches that the gate is noise.
    let root = scratch("canonical");
    template::create(&root, "Testbed").unwrap_or_else(|d| panic!("{d}"));

    let registry = dimetric_scene::KindRegistry::with_builtins();
    let text = std::fs::read_to_string(root.join("main.dim")).expect("the scene");
    let out = dimetric_scene::parse(&text, "main.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let doc = out.doc.expect("it parses");
    let mut formatted = doc.doc.clone();
    dimetric_scene::write::format_in_place(&mut formatted, &doc.scene, &registry, None);
    assert_eq!(formatted.to_string(), text, "the template is not canonical");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_project_draws_something() {
    // Two flat squares, generated rather than checked in — but they have to be
    // real PNGs that the importer accepts.
    let root = scratch("art");
    template::create(&root, "Testbed").unwrap_or_else(|d| panic!("{d}"));
    for sprite in ["assets/sprites/hero.png", "assets/sprites/wall.png"] {
        let path = root.join(sprite);
        let image =
            dimetric_render::atlas::load_png(&path).unwrap_or_else(|e| panic!("{sprite}: {e}"));
        assert_eq!((image.width, image.height), (16, 16));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn writing_into_somebody_elses_directory_is_refused() {
    // Typing the wrong path should not cost someone their work.
    let root = scratch("occupied");
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(root.join("important.txt"), "mine").expect("write");
    let result = template::create(&root, "Testbed");
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("important.txt")).expect("still there"),
        "mine"
    );
    let _ = std::fs::remove_dir_all(&root);
}
