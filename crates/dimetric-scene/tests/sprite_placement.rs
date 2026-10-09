//! Both sprite kinds can say where they are drawn and where they sort.
//!
//! `extract::sprite` has always read `offset` for `Sprite2D` and
//! `AnimatedSprite2D` alike, and only `Sprite2D` declared it — so authoring it
//! on the one kind a game animates was a hard `DIM0301`. A figure taller than
//! its cell has to be drawn raised for its feet to land on the floor, and the
//! node that plays its walk cycle is exactly the node that could not say so.
//!
//! `sort_offset` is the other half: the point a sprite sorts at, which need not
//! be the point it is drawn at.

use dimetric_scene::{KindRegistry, PropertyType};

/// The two kinds that draw a quad at a world position.
const SPRITE_KINDS: [&str; 2] = ["Sprite2D", "AnimatedSprite2D"];

fn declared(kind: &str, property: &str) -> Option<PropertyType> {
    KindRegistry::with_builtins()
        .get(kind)
        .unwrap_or_else(|| panic!("{kind} is a builtin"))
        .properties
        .iter()
        .find(|p| p.name == property)
        .map(|p| p.ty.clone())
}

#[test]
fn both_sprite_kinds_declare_the_offset_the_renderer_reads() {
    for kind in SPRITE_KINDS {
        assert_eq!(
            declared(kind, "offset"),
            Some(PropertyType::Vec2),
            "{kind} has to declare what the renderer already honours"
        );
    }
}

#[test]
fn both_sprite_kinds_declare_a_sort_offset() {
    for kind in SPRITE_KINDS {
        assert_eq!(
            declared(kind, "sort_offset"),
            Some(PropertyType::Vec2),
            "{kind} cannot be sorted at its feet without it"
        );
    }
}

#[test]
fn neither_has_a_default_so_every_scene_already_written_is_unchanged() {
    // This is the hash question, and it is decided here rather than in the
    // renderer. `parse` fills in every declared default so that a node's
    // property *set* does not depend on which keys an author happened to
    // write, and `Scene::hash_state` hashes every property — so a *defaulted*
    // property added to a builtin kind appears on every node of that kind and
    // changes the hash of every scene already recorded.
    //
    // Declared absent instead, with the renderer supplying the fallback. A
    // scene that says nothing is byte-identical and hash-identical to what it
    // was, which is what makes this safe to ship into a project with recorded
    // runs. `Sprite2D.offset` keeps its old zero default, because taking that
    // away would unfill it everywhere and move exactly the hashes this avoids.
    let registry = KindRegistry::with_builtins();
    for kind in SPRITE_KINDS {
        for property in ["sort_offset", "lit"] {
            let prop = registry
                .get(kind)
                .unwrap()
                .properties
                .iter()
                .find(|p| p.name == property)
                .unwrap_or_else(|| panic!("{kind}.{property}"));
            assert_eq!(
                prop.default, None,
                "{kind}.{property} must not be filled in"
            );
            assert!(!prop.required, "{kind}.{property} is not required");
        }
    }
    assert_eq!(
        declared("AnimatedSprite2D", "offset").map(|_| registry
            .get("AnimatedSprite2D")
            .unwrap()
            .properties
            .iter()
            .find(|p| p.name == "offset")
            .unwrap()
            .default
            .clone()),
        Some(None),
        "the newly declared offset is absent until authored"
    );
}

#[test]
fn a_scene_that_says_nothing_gains_no_properties() {
    // The same claim from the other side: parsing a plain sprite must not
    // materialise any of the new keys onto it.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n\
         [[node]]\nid = \"n_figure00\"\nkind = \"AnimatedSprite2D\"\nname = \"Figure\"\n\
         parent = \"n_root0000\"\nframes = \"asset:sprites/walk\"\n";
    let registry = KindRegistry::with_builtins();
    let doc = dimetric_scene::parse(text, "figure.dim", &registry)
        .doc
        .expect("the scene opens");
    let figure = doc
        .scene
        .resolve_path("/Root/Figure")
        .and_then(|id| doc.scene.get(id))
        .expect("the figure is there");
    for property in ["offset", "sort_offset", "lit"] {
        assert!(
            figure.get(property).is_none(),
            "{property} was filled in, which would move every recorded hash"
        );
    }
}

#[test]
fn a_lifted_animated_sprite_now_opens() {
    // The authored form of the defect: this scene used to fail to parse.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n\
         [[node]]\nid = \"n_figure00\"\nkind = \"AnimatedSprite2D\"\nname = \"Figure\"\n\
         parent = \"n_root0000\"\nframes = \"asset:sprites/walk\"\n\
         offset = [0.0, -6.0]\nsort_offset = [0.0, 7.0]\n";
    let registry = KindRegistry::with_builtins();
    let out = dimetric_scene::parse(text, "figure.dim", &registry);
    assert!(!out.diagnostics.has_errors(), "{}", out.diagnostics);
    let doc = out.doc.expect("the scene opens");
    let figure = doc
        .scene
        .resolve_path("/Root/Figure")
        .and_then(|id| doc.scene.get(id))
        .expect("the figure is there");
    assert_eq!(
        figure
            .get("sort_offset")
            .and_then(dimetric_scene::Value::as_vec2),
        Some(dimetric_core::Vec2Fx::from_ints(0, 7))
    );
}

#[test]
fn what_was_written_survives_a_round_trip() {
    // I2: a scene has to come back byte-identically, so a new property has to
    // be written as well as read.
    let text = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n\n\
         [[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Root\"\n\n\
         [[node]]\nid = \"n_figure00\"\nkind = \"Sprite2D\"\nname = \"Figure\"\n\
         parent = \"n_root0000\"\ntexture = \"asset:sprites/figure\"\n\
         offset = [0.0, -6.0]\nsort_offset = [0.0, 7.0]\n";
    let registry = KindRegistry::with_builtins();
    let doc = dimetric_scene::parse(text, "figure.dim", &registry)
        .doc
        .expect("the scene opens");
    let written = doc.to_text();
    assert!(written.contains("sort_offset = [0.0, 7.0]"), "{written}");
    let again = dimetric_scene::parse(&written, "figure.dim", &registry)
        .doc
        .expect("what we wrote opens");
    assert_eq!(again.to_text(), written, "a second trip changes nothing");
}
