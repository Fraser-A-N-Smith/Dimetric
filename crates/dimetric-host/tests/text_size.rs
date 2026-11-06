//! A script can point a label at a larger face, and the label draws in it.
//!
//! A *Text size* setting needs two things: a second set of faces imported at a
//! larger `font_size`, and a way to choose one at runtime. The report said the
//! second was missing — "a script cannot point a label at another font: the
//! write is refused as a change of the property's type".
//!
//! It is not refused. A script may write an `asset:` string over an
//! `asset-ref` property, and the typed-write guard re-reads it as a reference
//! by the same parser the scene format uses; `a_script_can_repoint_a_font`
//! below is that, with no engine change behind it. That half of G17 was
//! already closed.
//!
//! What was missing was the **atlas**. It packs the pages the scene and its
//! prefabs *author*, while inserting every imported font's *metrics*
//! unconditionally — so a face nothing authored arrived half present:
//! `Atlas::font` answered and `Atlas::region` did not. `extract::label` needs
//! both, so it silently fell back to the built-in face, and a label repointed
//! at the large font drew in the engine's own 5-pixel one.
//!
//! These need a real font file. Rather than commit one, the test finds one on
//! the machine and says so plainly if it cannot — it does not silently pass.

use std::path::Path;

use dimetric_host::Project;
use dimetric_render::{extract, Camera};
use dimetric_scene::ui::Canvas;
use dimetric_scene::Value;

/// Somewhere to find a TTF, in the order they are usually installed.
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "C:/Windows/Fonts/arial.ttf",
];

fn a_font() -> Vec<u8> {
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            return bytes;
        }
    }
    panic!(
        "no system font found; looked in:\n  {}\nInstall one (on Debian: \
         `apt-get install fonts-dejavu-core`) — this test imports a real font \
         rather than committing one.",
        CANDIDATES.join("\n  ")
    );
}

/// A project with two faces of the same typeface, one small and one large, and
/// a label authored to use the small one.
///
/// The large face is named by **nothing**: not the scene, not a prefab. That is
/// the whole point — it is the face a text-size setting switches to.
fn write_project(root: &Path) {
    std::fs::create_dir_all(root.join("assets/fonts")).expect("fonts");
    std::fs::create_dir_all(root.join("scripts")).expect("scripts");
    std::fs::write(root.join("project.toml"), "[game]\nname = \"Legible\"\n").expect("settings");

    let bytes = a_font();
    // An `id` in the sidecar, so the asset keeps its identity across renames:
    // the importer refuses a sidecar without one rather than inventing one
    // over the top of a file somebody wrote.
    for (name, size, id) in [
        ("body", 12u32, "a_bodysml1"),
        ("body-large", 24u32, "a_bodylrg1"),
    ] {
        let ttf = root.join(format!("assets/fonts/{name}.ttf"));
        std::fs::write(&ttf, &bytes).expect("a font file");
        std::fs::write(
            root.join(format!("assets/fonts/{name}.ttf.meta")),
            format!("id = \"{id}\"\nfont_size = {size}\n"),
        )
        .expect("a sidecar");
    }

    std::fs::write(
        root.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"World\"\n\
         script = \"script:scripts/options.lua\"\n\n\
         [[node]]\nid = \"n_text0000\"\nkind = \"Label\"\nname = \"Body\"\n\
         parent = \"n_root0000\"\ntext = \"Wield it\"\n\
         font = \"asset:fonts/body\"\n",
    )
    .expect("scene");

    // The Options screen choosing Large, on cue.
    std::fs::write(
        root.join("scripts/options.lua"),
        "function on_tick(self)\n\
         \x20 if tick.count() == 2 then\n\
         \x20   scene.find(\"/World/Body\"):set(\"font\", \"asset:fonts/body-large\")\n\
         \x20 end\n\
         \x20 local label = scene.find(\"/World/Body\")\n\
         \x20 self.face = label:get(\"font\")\n\
         \x20 self.width = ui.measure(label:get(\"font\"), \"Wield it\").w\n\
         end\n",
    )
    .expect("script");
}

/// Open the project, import, and run `ticks` ticks.
fn run(root: &Path, ticks: u64) -> (dimetric_host::Project, dimetric_sim::Sim) {
    let mut project = Project::open(root, 0);
    project.import_assets();
    assert!(
        !project.settings_diagnostics.has_errors(),
        "{}",
        project.settings_diagnostics
    );
    project.load_scene("main").expect("the scene loads");
    let (scene, mut diags) = project.runtime_scene().expect("a runtime scene");
    diags.extend(project.load_scripts());
    assert!(!diags.has_errors(), "{diags}");
    // The project's own host -- fonts and node kinds included -- and then the
    // scripts loaded into it. `script_host` builds the host; `load_scripts`
    // reads the files off disk into `project.scripts`; neither puts one in the
    // other.
    let mut host = project.script_host().expect("a host");
    let script_diags = host.load_all(
        project
            .scripts
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str())),
    );
    assert!(script_diags.is_empty(), "{script_diags:?}");
    let mut sim = dimetric_sim::Sim::new(
        scene,
        1,
        Box::new(host),
        dimetric_sim::SimConfig {
            tick_rate: project.tick_rate,
            canvas: Canvas {
                width: 320,
                height: 180,
            },
            ..Default::default()
        },
    )
    .with_clips(project.clips());
    let log = dimetric_sim::InputLog::new(1, "test", 1);
    for tick in 0..ticks {
        sim.step(log.frame(tick));
    }
    let reported = sim.take_diagnostics();
    assert!(!reported.has_errors(), "{reported}");
    (project, sim)
}

/// The width of the drawn text, and how many glyphs it took, from the frame.
fn drawn(project: &Project, sim: &dimetric_sim::Sim) -> (i32, usize) {
    let state = sim.state();
    let (atlas, diags) = dimetric_host::render::build_atlas(project, &state.scene);
    assert!(!diags.has_errors(), "{diags}");
    let camera = Camera::new((320, 180));
    let frame = extract(&state.scene, &atlas, &camera, None);
    let want = dimetric_core::NodeUid::parse("n_text0000").expect("an id");
    let mine: Vec<_> = frame.sprites.iter().filter(|i| i.node == want).collect();
    assert!(!mine.is_empty(), "the label was not drawn");
    let height = mine
        .iter()
        .map(|i| i.size.y.to_int_trunc())
        .max()
        .expect("a glyph");
    (height, mine.len())
}

#[test]
fn a_script_can_repoint_a_font() {
    // The half that was already built, asserted so the report's premise is on
    // the record: the write is not refused, and it lands as a reference.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (_, sim) = run(dir.path(), 3);
    let state = sim.state();
    let id = state.scene.resolve_path("/World/Body").expect("the label");
    assert_eq!(
        state
            .scene
            .get(id)
            .and_then(|n| n.get("font"))
            .and_then(Value::as_ref_value)
            .map(|r| r.target().to_string()),
        Some("fonts/body-large".to_string()),
        "the write landed as a reference, not a string"
    );
}

#[test]
fn a_repointed_label_draws_in_the_face_it_asked_for() {
    // The half that was missing. Before this, the large face was not in the
    // atlas, so the label fell back to the engine's built-in 5-pixel font —
    // which is both the wrong size and the wrong typeface, silently.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());

    let (project, small) = run(dir.path(), 1);
    let (small_height, small_glyphs) = drawn(&project, &small);

    let (project, large) = run(dir.path(), 3);
    let (large_height, large_glyphs) = drawn(&project, &large);

    assert_eq!(
        small_glyphs, large_glyphs,
        "the same words, so the same number of inked glyphs"
    );
    assert!(
        large_height > small_height,
        "a 24-pixel face should draw taller glyphs than a 12-pixel one: \
         {large_height} against {small_height}"
    );
    // And it is really the large face rather than the built-in fallback, which
    // is five pixels tall and would be *smaller* than the small face.
    assert!(
        large_height >= small_height * 3 / 2,
        "roughly twice the size, not a fallback: {large_height} against {small_height}"
    );
}

#[test]
fn an_unauthored_face_is_in_the_atlas_at_all() {
    // The fix, stated as the thing it changed. The metrics were always there
    // and the page was not, and `extract::label` needs both — so the symptom
    // was a silent fallback rather than a missing-asset diagnostic.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let (project, sim) = run(dir.path(), 1);
    let state = sim.state();
    let (atlas, diags) = dimetric_host::render::build_atlas(&project, &state.scene);
    assert!(!diags.has_errors(), "{diags}");
    assert!(
        atlas.font("fonts/body-large").is_some(),
        "the metrics were already unconditional"
    );
    assert!(
        atlas.region("fonts/body-large").is_some(),
        "and now so is the page, which is what the label needs to draw"
    );
    // The authored one is packed once, not twice.
    assert!(atlas.region("fonts/body").is_some());
}

#[test]
fn ui_measure_answers_for_the_face_a_label_was_repointed_to() {
    // Layout stays the game's job, so the measure has to follow the switch or
    // a game laying its rails from `ui.measure` would lay them for the old
    // face. Read out of the script's own variable, which is where a game
    // would read it.
    let dir = tempfile::tempdir().expect("tempdir");
    write_project(dir.path());
    let width = |ticks: u64| {
        let (_, sim) = run(dir.path(), ticks);
        let state = sim.state();
        let id = state.scene.resolve_path("/World").expect("root");
        let uid = state.scene.get(id).expect("root").uid;
        state
            .vars
            .get(&uid)
            .and_then(|v| v.get("width"))
            .and_then(Value::as_int)
            .expect("a width")
    };
    let small = width(1);
    let large = width(3);
    assert!(
        large > small,
        "the measure has to follow the face: {large} against {small}"
    );
}
