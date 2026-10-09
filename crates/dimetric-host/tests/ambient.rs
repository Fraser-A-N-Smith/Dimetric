//! A game can set its own ambient, from `project.toml` or from a scene.
//!
//! `RenderSettings::ambient` has existed since M6 and nothing a game ships
//! could reach it: `dim frame capture --ambient` could, through the agent, and
//! `project.toml` could not, a scene could not, and a script could not. So in
//! `dim-play` the ambient was always white, the light pass never ran, and every
//! `Light2D` a game placed added nothing at all.
//!
//! Two doors, with the nearer one winning. A region's light is a fact about the
//! region — nine scenes, nine ambients — so a `Camera2D` that names one wins
//! over the project-wide default, and a scene swap brings the new one with it.

use dimetric_host::render::scene_camera;
use dimetric_host::Project;
use dimetric_scene::Color;

const DUSK: Color = Color {
    r: 0x30,
    g: 0x30,
    b: 0x40,
    a: 0xFF,
};

/// A project whose `project.toml` is `toml` and whose `main.dim` carries
/// `camera_extra` inside its `Camera2D`.
fn project(toml: &str, camera_extra: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("project.toml"), toml).expect("settings");
    std::fs::write(
        dir.path().join("main.dim"),
        format!(
            "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
             [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n\
             [[node]]\nid = \"n_camera01\"\nkind = \"Camera2D\"\nname = \"View\"\n\
             parent = \"n_root0000\"\ncurrent = true\n{camera_extra}"
        ),
    )
    .expect("scene");
    dir
}

/// The ambient a run of this project would actually draw under.
fn effective(toml: &str, camera_extra: &str) -> Color {
    let dir = project(toml, camera_extra);
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    let (settings, _) = project.render_settings(None);
    let (scene, diagnostics) = project.runtime_scene().expect("the scene opens");
    assert!(!diagnostics.has_errors(), "{diagnostics}");
    let camera = scene_camera(&scene, settings.internal_resolution);
    camera.ambient.unwrap_or(settings.ambient)
}

#[test]
fn a_project_that_says_nothing_is_unlit() {
    assert_eq!(effective("", ""), Color::WHITE);
}

#[test]
fn render_ambient_reaches_the_renderer() {
    let dir = project("[render]\nambient = \"#303040ff\"\n", "");
    let (settings, warning) = Project::open(dir.path(), 0).render_settings(None);
    assert_eq!(settings.ambient, DUSK, "this is what the composite gets");
    assert!(warning.is_none(), "nothing was overridden");
    assert!(settings.lighting_enabled(), "and the light pass has work");
}

#[test]
fn a_cameras_own_ambient_reaches_the_camera() {
    let dir = project("", "ambient = \"#303040ff\"\n");
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    assert_eq!(scene_camera(&scene, (480, 270)).ambient, Some(DUSK));
}

#[test]
fn a_camera_that_says_nothing_leaves_the_project_to_decide() {
    // Absent rather than white, which is the whole reason the property has no
    // default: a camera that answered "white" would mean every scene overrode
    // the project's setting with the engine's, and `[render] ambient` could
    // never apply to any scene that has a camera — which is all of them.
    let dir = project("[render]\nambient = \"#303040ff\"\n", "");
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    assert_eq!(scene_camera(&scene, (480, 270)).ambient, None);
    assert_eq!(
        effective("[render]\nambient = \"#303040ff\"\n", ""),
        DUSK,
        "so the project's own stands"
    );
}

#[test]
fn the_scene_wins_when_both_say_something() {
    let candle = Color {
        r: 0x60,
        g: 0x40,
        b: 0x20,
        a: 0xFF,
    };
    assert_eq!(
        effective(
            "[render]\nambient = \"#303040ff\"\n",
            "ambient = \"#604020ff\"\n"
        ),
        candle,
        "a region states its own light"
    );
    // And in the other direction: a scene may declare itself unlit inside a
    // project that is dim everywhere else.
    assert_eq!(
        effective(
            "[render]\nambient = \"#303040ff\"\n",
            "ambient = \"#ffffffff\"\n"
        ),
        Color::WHITE
    );
}

#[test]
fn a_colour_that_is_not_one_is_refused_rather_than_guessed() {
    let dir = project("[render]\nambient = \"dusk\"\n", "");
    let project = Project::open(dir.path(), 0);
    let diagnostics = &project.settings_diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == dimetric_core::Code::SETTINGS_INVALID
                && d.message.contains("ambient")),
        "{diagnostics}"
    );
    let (settings, _) = project.render_settings(None);
    assert_eq!(
        settings.ambient,
        Color::WHITE,
        "and the default stands rather than something invented"
    );
}

/// The state hash at each of twelve ticks of this project.
fn hashes(toml: &str, camera: &str) -> Vec<dimetric_core::StateHash> {
    let dir = project(toml, camera);
    let mut project = Project::open(dir.path(), 0);
    project.load_scene("main").expect("the scene loads");
    let (scene, _) = project.runtime_scene().expect("the scene opens");
    let config = project.sim_config();
    let host = project.script_host().expect("a script host");
    let mut sim = dimetric_sim::Sim::new(scene, 11, Box::new(host), config);
    let log = dimetric_sim::InputLog::new(11, env!("CARGO_PKG_VERSION"), 1);
    (0..12)
        .map(|tick| {
            sim.step(log.frame(tick));
            sim.hash()
        })
        .collect()
}

#[test]
fn the_project_wide_ambient_is_not_simulation_state() {
    // `[render] ambient` is settings, not scene data. It reaches a uniform and
    // a multiply and nothing the simulation can see, so a game can dim its
    // whole world without re-recording a single fixture.
    assert_eq!(
        hashes("", ""),
        hashes("[render]\nambient = \"#303040ff\"\n", ""),
        "a project-wide light is not state"
    );
}

#[test]
fn a_scene_that_declares_no_ambient_hashes_exactly_as_before() {
    // The property having come into existence must change nothing. An absent
    // key is not hashed, so every scene already written is untouched — which
    // is what makes this safe to ship into a project with recorded runs.
    assert_eq!(
        hashes("", ""),
        hashes("", ""),
        "the same project twice, for the baseline"
    );
    let dir = project("", "");
    let text = std::fs::read_to_string(dir.path().join("main.dim")).expect("the scene");
    assert!(!text.contains("ambient"), "nothing was written into it");
}

#[test]
fn authoring_it_on_a_camera_moves_the_hash_like_any_other_property() {
    // Deliberate, asserted, and worth being plain about rather than quiet.
    //
    // A `Camera2D`'s `ambient` is a node property, and `Scene::hash_state`
    // feeds every node property into the hash — the same is true of `zoom`,
    // `projection` and `pixel_snap` beside it. So a scene that adopts an
    // ambient has to have its fixtures re-recorded, exactly as if a property
    // had been added to any node in it.
    //
    // That is not the hazard the invariant guards against. The hazard is a
    // *presentation* value that differs between machines or sessions and
    // silently changes the hash — a window size, a volume, a present filter —
    // and none of those are hashed. A scene's authored content is the same on
    // every machine because it is committed, so hashing it costs a re-record
    // when it is edited and risks no divergence at all.
    //
    // A game that is not ready to re-record has the project-wide door above,
    // which costs nothing.
    assert_ne!(
        hashes("", ""),
        hashes("", "ambient = \"#303040ff\"\n"),
        "an authored property is scene data"
    );
}

#[test]
fn the_ambient_never_changes_what_a_tick_computes() {
    // The property that actually matters. Two runs of the *same* scene — the
    // only thing differing is the project-wide light, which is not in the
    // scene — have to agree at every tick, and a scene carrying an ambient has
    // to be internally consistent run to run.
    let dim = "[render]\nambient = \"#303040ff\"\n";
    assert_eq!(
        hashes(dim, "ambient = \"#604020ff\"\n"),
        hashes("", "ambient = \"#604020ff\"\n"),
        "the project-wide setting cannot reach a tick"
    );
    assert_eq!(
        hashes(dim, "ambient = \"#604020ff\"\n"),
        hashes(dim, "ambient = \"#604020ff\"\n"),
        "and the same scene replays to the same hashes"
    );
}
