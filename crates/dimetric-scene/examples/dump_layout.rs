//! Print a scene's UI layout, for working out why a menu looks wrong.
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_layout <scene.dim>");
    let text = std::fs::read_to_string(&path).expect("read");
    let out = dimetric_scene::parse(&text, &path, &dimetric_scene::KindRegistry::with_builtins());
    if out.diagnostics.has_errors() {
        println!("{}", out.diagnostics);
        return;
    }
    let scene = out.doc.unwrap().scene;
    let canvas = dimetric_scene::ui::Canvas::default();
    let layout = dimetric_scene::ui::layout(&scene, canvas);
    for id in scene.walk() {
        let Some(node) = scene.get(id) else { continue };
        match layout.get(&id) {
            Some(r) => println!(
                "{:<10} {:<10} pos=({}, {}) size=({}, {})",
                node.kind, node.name, r.pos.x, r.pos.y, r.size.x, r.size.y
            ),
            None => println!("{:<10} {:<10} (not a control)", node.kind, node.name),
        }
    }
}
