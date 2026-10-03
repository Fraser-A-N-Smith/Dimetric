//! A script host built from a project knows what the project knows.
//!
//! Three inputs decide what a script does, and none of them is simulation
//! state: the tick rate behind `tick.dt`, the fonts behind `ui.measure`, and
//! the node kinds behind a write to a property the node carries no value for.
//! All three reach the state hash — `dt` through every integration, a
//! measured string through a control's rectangle, a declared type through what
//! lands in a property — and each of them had been forgotten at a call site:
//!
//!   * `dim replay` built its host with a hardcoded 60 and no fonts, while
//!     stepping the simulation at the project's own rate. Replaying a project
//!     that is not 60Hz ran its scripts on a different `dt` from the run being
//!     replayed, which is the one thing a replay must not do;
//!   * the editor's playback had no fonts either;
//!   * nobody had the kinds, because until now nothing read them.
//!
//! So there is one door. These tests are what keeps it the only one.

use dimetric_host::Project;

const KINDS: &str = r#"
[[kind]]
name = "Marker"
extends = "Node2D"
doc = "A project kind with a property that has no default."

[[kind.property]]
name = "area"
type = "rect"
doc = "Where it applies."
"#;

/// A project declaring a non-default tick rate and a kind of its own.
fn project(dir: &std::path::Path) {
    std::fs::write(dir.join("project.toml"), "[sim]\ntick_rate = 30\n").expect("settings");
    std::fs::write(dir.join("kinds.toml"), KINDS).expect("kinds");
    std::fs::write(
        dir.join("main.dim"),
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\
         script = \"script:scripts/main.lua\"\n\n\
         [[node]]\nid = \"n_mark0000\"\nkind = \"Marker\"\nname = \"Mark\"\n\
         parent = \"n_root0000\"\n",
    )
    .expect("scene");
    std::fs::create_dir_all(dir.join("scripts")).expect("scripts dir");
    std::fs::write(
        dir.join("scripts/main.lua"),
        "function on_ready(self)\n\
         \x20 scene.find(\"/Root/Mark\"):set(\"area\", { 0, 0, 12, 12 })\n\
         \x20 self.dt = tick.dt()\n\
         \x20 self.measured = ui.measure(\"builtin\", \"ab\").w\n\
         end\n",
    )
    .expect("script");
}

/// Open the project, run one tick through its own script host, and return the
/// state.
fn run(dir: &std::path::Path) -> (dimetric_sim::SimState, dimetric_core::Diagnostics) {
    let mut project = Project::open(dir, 0);
    project.load_scene("main").expect("the scene loads");
    project.import_assets();
    let (scene, mut diags) = project.runtime_scene().expect("a runtime scene");
    diags.extend(project.load_scripts());
    let mut host = project.script_host().expect("a script host");
    let script_diags = host.load_all(
        project
            .scripts
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str())),
    );
    assert!(script_diags.is_empty(), "{:?}", script_diags);
    let config = project.sim_config();
    let mut sim = dimetric_sim::Sim::new(scene, 7, Box::new(host), config);
    sim.step(dimetric_sim::InputFrame::idle(1));
    diags.extend(sim.take_diagnostics());
    let state = sim.state().clone();
    (state, diags)
}

fn var(state: &dimetric_sim::SimState, path: &str, key: &str) -> Option<dimetric_scene::Value> {
    let id = state.scene.resolve_path(path)?;
    let uid = state.scene.get(id)?.uid;
    state.vars.get(&uid)?.get(key).cloned()
}

#[test]
fn a_projects_kinds_reach_the_write_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    project(dir.path());
    let (state, diags) = run(dir.path());
    assert!(!diags.has_errors(), "{diags}");

    let id = state.scene.resolve_path("/Root/Mark").expect("the marker");
    let area = state
        .scene
        .get(id)
        .expect("the marker")
        .get("area")
        .expect("the script wrote it");
    assert_eq!(
        area.type_name(),
        "rect",
        "a declared rect took a Lua list untyped: {area}"
    );
}

#[test]
fn a_projects_tick_rate_reaches_the_scripts() {
    // `tick.dt` is 1/30 for a 30Hz project. It used to be 1/60 in a replay
    // whatever the project said.
    let dir = tempfile::tempdir().expect("tempdir");
    project(dir.path());
    let (state, _) = run(dir.path());
    let dt = var(&state, "/Root", "dt").expect("dt");
    assert_eq!(dt.as_scalar(), Some(dimetric_core::Fx::ONE / 30));
}

#[test]
fn a_projects_fonts_reach_the_scripts() {
    // A host with no fonts measures nothing, and a control sized from nothing
    // is a control a click lands outside of.
    let dir = tempfile::tempdir().expect("tempdir");
    project(dir.path());
    let (state, _) = run(dir.path());
    let measured = var(&state, "/Root", "measured").expect("measured");
    assert!(
        measured
            .as_scalar()
            .is_some_and(|w| w > dimetric_core::Fx::ZERO),
        "two characters measured {measured} wide"
    );
}

#[test]
fn a_save_of_what_a_script_wrote_round_trips() {
    // The defect in one line: live state and the state a save of it loads back
    // to have to hash the same, or a resumed run is not the run that was saved.
    let dir = tempfile::tempdir().expect("tempdir");
    project(dir.path());
    let (state, _) = run(dir.path());

    let mut registry = dimetric_scene::KindRegistry::with_builtins();
    dimetric_scene::project_kinds::merge(&mut registry, KINDS, "kinds.toml");
    let text = dimetric_scene::write::to_canonical_text(&state.scene, &registry, None);
    let out = dimetric_scene::parse(&text, "saved.dim", &registry);
    assert!(
        !out.diagnostics.has_errors(),
        "the save did not load:\n{}\n{text}",
        out.diagnostics
    );
    let hash = |scene: &dimetric_scene::Scene| {
        let mut h = dimetric_core::StateHasher::new();
        scene.hash_state(&mut h);
        h.finish()
    };
    assert_eq!(
        hash(&state.scene),
        hash(&out.doc.expect("a document").scene),
        "from:\n{text}"
    );
}
