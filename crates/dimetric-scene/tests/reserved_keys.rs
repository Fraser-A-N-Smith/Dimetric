//! The reserved keys are documented, and the documentation cannot drift.
//!
//! `docs/API.md` referred to "the reserved `scene` key", said "No properties
//! beyond the reserved keys" under several kinds, and carried a diagnostic for
//! shadowing one — without ever listing them. So `pos`, `visible`, `z` and
//! `layer` appeared in no table in the whole reference, and working out that
//! `layer` exists cost somebody an afternoon of reading the renderer.
//!
//! The table is generated from `RESERVED_KEY_DOCS` now. These tests are what
//! stop that list and `RESERVED_KEYS` parting company, the same way
//! `dimetric-sim`'s `api_doc` tests hold the Lua globals table.

use dimetric_scene::schema::{RESERVED_KEYS, RESERVED_KEY_DOCS};

#[test]
fn every_reserved_key_is_documented() {
    for key in RESERVED_KEYS {
        assert!(
            RESERVED_KEY_DOCS.iter().any(|(k, _)| k == key),
            "`{key}` is reserved and has no line in RESERVED_KEY_DOCS"
        );
    }
}

#[test]
fn nothing_is_documented_that_is_not_reserved() {
    // The other direction, which is how a reference ends up describing a key
    // the engine stopped having.
    for (key, _) in RESERVED_KEY_DOCS {
        assert!(
            RESERVED_KEYS.contains(key),
            "`{key}` is documented as reserved and is not in RESERVED_KEYS"
        );
    }
}

#[test]
fn the_documentation_is_in_the_same_order_as_the_list() {
    // So the generated table reads in the order somebody scanning the source
    // would expect, rather than in whichever order the docs happened to be
    // written.
    let documented: Vec<&str> = RESERVED_KEY_DOCS.iter().map(|(k, _)| *k).collect();
    assert_eq!(documented, RESERVED_KEYS.to_vec());
}

#[test]
fn every_line_actually_says_something() {
    // A placeholder entry would satisfy the tests above and help nobody.
    for (key, doc) in RESERVED_KEY_DOCS {
        assert!(
            doc.len() > 20,
            "`{key}`'s description is too short to be useful: {doc:?}"
        );
        assert!(
            doc.ends_with('.'),
            "`{key}`'s description is not a sentence"
        );
    }
}

#[test]
fn the_draw_order_keys_explain_how_they_relate() {
    // `z` and `layer` are the pair that was actually missing, and the question
    // somebody has is which one wins. Both lines have to answer it.
    let doc = |name: &str| {
        RESERVED_KEY_DOCS
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, d)| *d)
            .expect("documented")
    };
    assert!(doc("z").contains("layer"), "z should point at layer");
    assert!(doc("layer").contains('z'), "layer should point at z");
}
