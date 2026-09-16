//! Print what a scene extracts to, split by layer.
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_frame <scene.dim>");
    let text = std::fs::read_to_string(&path).expect("read");
    let out = dimetric_scene::parse(&text, &path, &dimetric_scene::KindRegistry::with_builtins());
    let scene = out.doc.unwrap().scene;

    let (font, page) = dimetric_assets::builtin_font::builtin();
    let mut fonts = std::collections::BTreeMap::new();
    fonts.insert(
        dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
        font,
    );
    let atlas = dimetric_render::Atlas::pack(
        vec![
            dimetric_render::atlas::Source {
                name: dimetric_assets::builtin_font::BUILTIN_FONT.to_string(),
                width: page.width,
                height: page.height,
                pixels: page.pixels,
            },
            dimetric_render::atlas::solid(dimetric_scene::Color::WHITE),
        ],
        512,
    )
    .with_fonts(fonts);

    let camera = dimetric_render::Camera::new((320, 180));
    let frame = dimetric_render::extract(&scene, &atlas, &camera, None);
    println!(
        "world: {} items, {} batches",
        frame.sprites.len(),
        frame.batches.len()
    );
    println!(
        "ui:    {} items, {} batches",
        frame.ui.len(),
        frame.ui_batches.len()
    );
    for item in &frame.ui {
        println!(
            "  uid={:?} pos=({}, {}) size=({}, {}) uv={:?}",
            item.node, item.pos.x, item.pos.y, item.size.x, item.size.y, item.uv
        );
    }
}
