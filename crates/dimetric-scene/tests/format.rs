//! The scene format's acceptance criteria: an exact round trip, working
//! instancing, and every validation code firing on crafted bad input.

use dimetric_core::{Code, Fx, NodeUid};
use dimetric_scene::{parse, KindRegistry, Scene, SceneSource, Value};

/// The example from the design document, used as the round-trip fixture.
const ARENA: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

# ── Environment ───────────────────────────────────

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena01"

[[node]]
id = "n_k3xq7a2p"
kind = "TileLayer"
name = "Floor"
parent = "n_root0000"          # /Arena01
tileset = "asset:tilesets/dungeon"
cell = [16, 16]
collision = false

[[node]]
id = "n_m9v2ht5w"
kind = "Sprite2D"
name = "Brazier"
parent = "n_root0000"          # /Arena01
pos = [96.0, 48.0]
texture = "asset:sprites/props/brazier"
z = 10

[[node]]
id = "n_p4rr01ez"
kind = "Light2D"
name = "Glow"
parent = "n_m9v2ht5w"          # /Arena01/Brazier
color = "#ffb347e0"
radius = 72.0
"##;

fn registry() -> KindRegistry {
    KindRegistry::with_builtins()
}

fn load(src: &str) -> (Option<dimetric_scene::SceneDoc>, dimetric_core::Diagnostics) {
    let out = parse(src, "test.dim", &registry());
    (out.doc, out.diagnostics)
}

fn codes(d: &dimetric_core::Diagnostics) -> Vec<&'static str> {
    d.iter().map(|x| x.code.0).collect()
}

#[test]
fn load_then_save_is_byte_identical() {
    let (doc, diags) = load(ARENA);
    assert!(!diags.has_errors(), "{diags}");
    let doc = doc.unwrap();
    assert_eq!(doc.to_text(), ARENA, "round trip changed the file");
}

#[test]
fn comments_and_spacing_survive_a_property_edit() {
    let (doc, _) = load(ARENA);
    let mut doc = doc.unwrap();
    // An engine edit rewrites one value in the document; everything else,
    // including the box-drawing comment, is untouched.
    let nodes = doc.doc["node"].as_array_of_tables_mut().unwrap();
    nodes.get_mut(3).unwrap()["radius"] = toml_edit::value(80.0);
    let text = doc.to_text();
    assert!(text.contains("# ── Environment"), "comment was lost");
    assert!(text.contains("radius = 80.0"));
    assert_eq!(text.lines().count(), ARENA.lines().count());
}

#[test]
fn the_tree_is_rebuilt_from_the_flat_list() {
    let (doc, _) = load(ARENA);
    let scene = doc.unwrap().scene;
    assert_eq!(scene.len(), 4);
    let glow = scene
        .resolve_path("/Arena01/Brazier/Glow")
        .expect("path resolves");
    assert_eq!(scene.path_of(glow).unwrap(), "/Arena01/Brazier/Glow");
    assert_eq!(scene.get(glow).unwrap().kind, "Light2D");
    // Depth-first, parents before children.
    let names: Vec<String> = scene
        .walk()
        .iter()
        .map(|id| scene.get(*id).unwrap().name.clone())
        .collect();
    assert_eq!(names, ["Arena01", "Floor", "Brazier", "Glow"]);
}

#[test]
fn world_transforms_compose_down_the_tree() {
    let (doc, _) = load(ARENA);
    let mut scene = doc.unwrap().scene;
    scene.update_world_transforms();
    let glow = scene.resolve_path("/Arena01/Brazier/Glow").unwrap();
    // The light inherits the brazier's position.
    assert_eq!(
        scene.get(glow).unwrap().world().pos,
        dimetric_core::Vec2Fx::from_ints(96, 48)
    );
}

#[test]
fn canonical_form_is_stable_under_reformatting() {
    let (doc, _) = load(ARENA);
    let scene = doc.unwrap().scene;
    let once = dimetric_scene::write::to_canonical_text(&scene, &registry(), None);
    let (doc2, diags) = load(&once);
    assert!(!diags.has_errors(), "canonical output must parse: {diags}");
    let twice = dimetric_scene::write::to_canonical_text(&doc2.unwrap().scene, &registry(), None);
    assert_eq!(once, twice, "formatting is not idempotent");
    assert!(
        once.contains("# /Arena01/Brazier"),
        "path comments regenerate"
    );
    // Defaults are omitted.
    assert!(!once.contains("collision = false"));
    assert!(!once.contains("visible = true"));
}

// -- validation ---------------------------------------------------------

fn scene_with(nodes: &str) -> String {
    format!("format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_root0000\"\n{nodes}")
}

fn root() -> String {
    "\n[[node]]\nid = \"n_root0000\"\nkind = \"Node2D\"\nname = \"Arena01\"\n".to_string()
}

#[track_caller]
fn expect_code(src: &str, want: Code) {
    let (_, diags) = load(src);
    assert!(
        diags.iter().any(|d| d.code == want),
        "expected {want}, got {:?}\n{diags}",
        codes(&diags)
    );
}

#[test]
fn dim0100_malformed_toml() {
    expect_code("format = \"dimetric\"\n[[node\n", Code::PARSE_FAILED);
}

#[test]
fn dim0101_unknown_node_kind() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Widget3D\"\nname = \"W\"\nparent = \"n_root0000\"\n")),
        Code::UNKNOWN_KIND,
    );
}

#[test]
fn dim0102_duplicate_node_id() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_root0000\"\nkind = \"Node\"\nname = \"Twin\"\nparent = \"n_root0000\"\n")),
        Code::DUPLICATE_ID,
    );
}

#[test]
fn dim0103_dangling_parent() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Node\"\nname = \"Lost\"\nparent = \"n_zzzzzzzz\"\n")),
        Code::DANGLING_PARENT,
    );
}

#[test]
fn dim0104_property_shadows_a_reserved_key() {
    use dimetric_scene::schema::{NodeKindSchema, PropertySchema, PropertyType};
    let mut reg = KindRegistry::empty();
    let err = reg
        .register(NodeKindSchema::new(
            "Bad",
            "declares a reserved key",
            vec![PropertySchema::new(
                "pos",
                PropertyType::Vec2,
                None,
                "clash",
            )],
        ))
        .unwrap_err();
    assert_eq!(err.code, Code::RESERVED_KEY);
}

#[test]
fn dim0105_duplicate_sibling_name() {
    let twin = |id: &str| {
        format!("\n[[node]]\nid = \"{id}\"\nkind = \"Node\"\nname = \"Same\"\nparent = \"n_root0000\"\n")
    };
    expect_code(
        &scene_with(&(root() + &twin("n_aaaaaaaa") + &twin("n_bbbbbbbb"))),
        Code::DUPLICATE_NAME,
    );
}

#[test]
fn dim0106_child_declared_before_parent() {
    let src = scene_with(
        &(root()
            + "\n[[node]]\nid = \"n_cccccccc\"\nkind = \"Node\"\nname = \"Kid\"\nparent = \"n_dddddddd\"\n"
            + "\n[[node]]\nid = \"n_dddddddd\"\nkind = \"Node\"\nname = \"Parent\"\nparent = \"n_root0000\"\n"),
    );
    expect_code(&src, Code::CHILD_BEFORE_PARENT);
}

#[test]
fn dim0108_scene_names_a_root_that_is_not_there() {
    expect_code(
        "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_nosuchid\"\n",
        Code::MISSING_ROOT,
    );
}

#[test]
fn dim0109_malformed_node_id() {
    expect_code(
        &scene_with("\n[[node]]\nid = \"nope\"\nkind = \"Node\"\nname = \"X\"\n"),
        Code::BAD_ID_FORM,
    );
}

#[test]
fn dim0110_missing_format_header() {
    expect_code("version = 1\n", Code::BAD_HEADER);
    expect_code("format = \"dimetric\"\nversion = 99\n", Code::BAD_HEADER);
}

#[test]
fn dim0201_property_type_mismatch() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Sprite2D\"\nname = \"S\"\nparent = \"n_root0000\"\ntexture = \"asset:a\"\nflip_h = 3\n")),
        Code::TYPE_MISMATCH,
    );
}

#[test]
fn dim0202_value_outside_the_schema_range() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Light2D\"\nname = \"L\"\nparent = \"n_root0000\"\nradius = -5.0\n")),
        Code::OUT_OF_RANGE,
    );
    // An enum outside its allowed set is the same failure.
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Camera2D\"\nname = \"C\"\nparent = \"n_root0000\"\nprojection = \"Hexagonal\"\n")),
        Code::OUT_OF_RANGE,
    );
}

#[test]
fn dim0203_scalar_not_exactly_representable() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Light2D\"\nname = \"L\"\nparent = \"n_root0000\"\nradius = 0.1\n")),
        Code::NOT_REPRESENTABLE,
    );
}

#[test]
fn dim0204_malformed_reference() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Sprite2D\"\nname = \"S\"\nparent = \"n_root0000\"\ntexture = \"sprites/hero\"\n")),
        Code::BAD_REFERENCE,
    );
    // Right syntax, wrong kind of reference.
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_bbbbbbbb\"\nkind = \"Sprite2D\"\nname = \"T\"\nparent = \"n_root0000\"\ntexture = \"scene:prefabs/x\"\n")),
        Code::BAD_REFERENCE,
    );
}

#[test]
fn dim0205_malformed_chunk_data() {
    let src = scene_with(
        &(root()
            + "\n[[node]]\nid = \"n_k3xq7a2p\"\nkind = \"TileLayer\"\nname = \"Floor\"\nparent = \"n_root0000\"\ntileset = \"asset:t\"\n"
            + "\n[[chunk]]\nlayer = \"n_k3xq7a2p\"\nat = [0, 0]\ndata = \"18:12 4:7\"\n"),
    );
    expect_code(&src, Code::BAD_CHUNK_DATA);
}

#[test]
fn dim0301_unknown_property_is_a_hard_error_and_suggests_the_right_one() {
    let src = scene_with(
        &(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Light2D\"\nname = \"L\"\nparent = \"n_root0000\"\nraduis = 72.0\n"),
    );
    let (_, diags) = load(&src);
    let d = diags
        .iter()
        .find(|d| d.code == Code::UNKNOWN_PROPERTY)
        .expect("the raduis typo must be caught");
    assert_eq!(
        d.fields.get("suggestion").and_then(|v| v.as_str()),
        Some("radius"),
        "{d}"
    );
}

#[test]
fn dim0303_required_property_missing() {
    expect_code(
        &scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Sprite2D\"\nname = \"S\"\nparent = \"n_root0000\"\n")),
        Code::MISSING_REQUIRED,
    );
}

// -- instancing ---------------------------------------------------------

const SKELETON: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_sk_root0"

[[node]]
id = "n_sk_root0"
kind = "Node2D"
name = "Skeleton"

[[node]]
id = "n_sk_stats"
kind = "Node"
name = "Stats"
parent = "n_sk_root0"

[[node]]
id = "n_sk_torch"
kind = "Light2D"
name = "Torch"
parent = "n_sk_root0"
radius = 40.0

[[node]]
id = "n_sk_hand0"
kind = "Node2D"
name = "Hand"
parent = "n_sk_root0"
pos = [8.0, 0.0]
"##;

struct Prefabs;

impl SceneSource for Prefabs {
    fn load(
        &self,
        reference: &dimetric_scene::Reference,
    ) -> Result<Scene, dimetric_core::Diagnostic> {
        match reference.target() {
            "prefabs/skeleton" => Ok(parse(SKELETON, "skeleton.dim", &registry())
                .doc
                .unwrap()
                .scene),
            other => Err(dimetric_core::Diagnostic::new(
                Code::ASSET_MISSING,
                format!("no prefab {other}"),
            )),
        }
    }
}

/// A room instancing the skeleton: one property override, one removal, and one
/// node added inside the instance.
const ROOM: &str = r##"format = "dimetric"
version = 1

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "Arena01"

[[node]]
id = "n_e1x4k0qb"
kind = "Instance"
name = "Skeleton_01"
parent = "n_root0000"
scene = "scene:prefabs/skeleton"
pos = [120.0, 64.0]

[[node]]
id = "n_z7wq2m8c"
kind = "Light2D"
name = "Cursed"
parent = "n_e1x4k0qb/n_sk_hand0"
color = "#7b2fbfff"

[[override]]
instance = "n_e1x4k0qb"
target = "n_sk_torch"
removed = true

[[override]]
instance = "n_e1x4k0qb"
target = "n_sk_stats"
name = "Vitals"
"##;

#[test]
fn an_instance_resolves_with_sparse_overrides() {
    let (doc, diags) = load(ROOM);
    assert!(!diags.has_errors(), "{diags}");
    let (flat, diags) = dimetric_scene::resolve(&doc.unwrap().scene, &Prefabs, &registry());
    assert!(!diags.has_errors(), "{diags}");

    // The instance node's own pos overrides the prefab root's transform.
    let skeleton = flat
        .resolve_path("/Arena01/Skeleton_01")
        .expect("instance root");
    assert_eq!(
        flat.get(skeleton).unwrap().transform.pos,
        dimetric_core::Vec2Fx::from_ints(120, 64)
    );

    // An override renamed Stats, and the removal took the torch out.
    assert!(flat.resolve_path("/Arena01/Skeleton_01/Vitals").is_some());
    assert!(flat.resolve_path("/Arena01/Skeleton_01/Stats").is_none());
    assert!(
        flat.resolve_path("/Arena01/Skeleton_01/Torch").is_none(),
        "removed = true should drop the inherited child"
    );

    // A node added inside the instance lands under the prefab's own child.
    let cursed = flat
        .resolve_path("/Arena01/Skeleton_01/Hand/Cursed")
        .expect("added node attaches inside the instance");
    assert_eq!(flat.get(cursed).unwrap().kind, "Light2D");

    // Nothing named Instance survives resolution.
    assert!(flat
        .walk()
        .iter()
        .all(|id| flat.get(*id).unwrap().kind != "Instance"));
}

#[test]
fn two_instances_of_one_prefab_do_not_collide() {
    let two = ROOM.replace(
        "[[override]]\ninstance = \"n_e1x4k0qb\"\ntarget = \"n_sk_torch\"\nremoved = true\n\n",
        "",
    );
    let two = two.replace(
        "[[node]]\nid = \"n_z7wq2m8c\"",
        "[[node]]\nid = \"n_e2x4k0qb\"\nkind = \"Instance\"\nname = \"Skeleton_02\"\nparent = \"n_root0000\"\nscene = \"scene:prefabs/skeleton\"\npos = [200.0, 64.0]\n\n[[node]]\nid = \"n_z7wq2m8c\"",
    );
    let (doc, diags) = load(&two);
    assert!(!diags.has_errors(), "{diags}");
    let (flat, diags) = dimetric_scene::resolve(&doc.unwrap().scene, &Prefabs, &registry());
    assert!(!diags.has_errors(), "{diags}");
    assert!(flat.resolve_path("/Arena01/Skeleton_01/Hand").is_some());
    assert!(flat.resolve_path("/Arena01/Skeleton_02/Hand").is_some());
    // Derived ids differ per instance, so nothing is shadowed.
    let a = flat.resolve_path("/Arena01/Skeleton_01/Hand").unwrap();
    let b = flat.resolve_path("/Arena01/Skeleton_02/Hand").unwrap();
    assert_ne!(flat.get(a).unwrap().uid, flat.get(b).unwrap().uid);
}

#[test]
fn dim0302_orphaned_override_is_kept_and_warned_about() {
    let src = ROOM.replace("target = \"n_sk_stats\"", "target = \"n_sk_gone0\"");
    let (doc, _) = load(&src);
    let doc = doc.unwrap();
    let (_, diags) = dimetric_scene::resolve(&doc.scene, &Prefabs, &registry());
    assert!(
        diags.iter().any(|d| d.code == Code::ORPHANED_OVERRIDE),
        "{diags}"
    );
    assert!(
        !diags.has_errors(),
        "an orphaned override is a warning, not an error"
    );
    // Still in the file, ready for the prefab to grow the node back.
    assert!(doc.to_text().contains("n_sk_gone0"));
}

#[test]
fn dim0107_instance_cycle() {
    struct SelfRef;
    impl SceneSource for SelfRef {
        fn load(&self, _r: &dimetric_scene::Reference) -> Result<Scene, dimetric_core::Diagnostic> {
            let src = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_loop0000\"\n\n[[node]]\nid = \"n_loop0000\"\nkind = \"Instance\"\nname = \"Loop\"\nscene = \"scene:loop\"\n";
            Ok(parse(src, "loop.dim", &registry()).doc.unwrap().scene)
        }
    }
    let src = "format = \"dimetric\"\nversion = 1\n\n[scene]\nroot = \"n_top00000\"\n\n[[node]]\nid = \"n_top00000\"\nkind = \"Instance\"\nname = \"Top\"\nscene = \"scene:loop\"\n";
    let doc = parse(src, "top.dim", &registry()).doc.unwrap();
    let (_, diags) = dimetric_scene::resolve(&doc.scene, &SelfRef, &registry());
    assert!(
        diags.iter().any(|d| d.code == Code::INSTANCE_CYCLE),
        "{diags}"
    );
}

// -- tiles --------------------------------------------------------------

#[test]
fn chunk_data_round_trips_through_run_length_encoding() {
    let mut cells = [0u16; dimetric_scene::CHUNK_CELLS];
    cells[..18].fill(12);
    cells[18..22].fill(7);
    cells[22..].fill(3);
    let text = dimetric_scene::chunk::encode_rle(&cells);
    assert_eq!(text, "18:12 4:7 1002:3");
    let back = dimetric_scene::chunk::decode_rle(&text).unwrap();
    assert_eq!(back.as_slice(), cells.as_slice());
}

#[test]
fn tile_coordinates_split_correctly_either_side_of_the_origin() {
    use dimetric_scene::chunk::split_coord;
    assert_eq!(split_coord(0, 0), ([0, 0], [0, 0]));
    assert_eq!(split_coord(33, 5), ([1, 0], [1, 5]));
    // Floor division, so -1 lands in the chunk to the left, not chunk zero.
    assert_eq!(split_coord(-1, -1), ([-1, -1], [31, 31]));
    assert_eq!(split_coord(-32, 0), ([-1, 0], [0, 0]));
}

#[test]
fn scenes_hash_by_content_not_by_authoring_order() {
    let (a, _) = load(ARENA);
    let reordered = ARENA.replace(
        "color = \"#ffb347e0\"\nradius = 72.0",
        "radius = 72.0\ncolor = \"#ffb347e0\"",
    );
    let (b, _) = load(&reordered);
    let mut ha = dimetric_core::StateHasher::new();
    let mut hb = dimetric_core::StateHasher::new();
    a.unwrap().scene.hash_state(&mut ha);
    b.unwrap().scene.hash_state(&mut hb);
    assert_eq!(
        ha.finish(),
        hb.finish(),
        "key order must not change the hash"
    );
}

#[test]
fn a_node_cannot_be_moved_inside_its_own_subtree() {
    let (doc, _) = load(ARENA);
    let mut scene = doc.unwrap().scene;
    let brazier = scene.resolve_path("/Arena01/Brazier").unwrap();
    let glow = scene.resolve_path("/Arena01/Brazier/Glow").unwrap();
    let err = scene.reparent(brazier, glow).unwrap_err();
    assert_eq!(err.code, Code::ILLEGAL_REPARENT);
}

#[test]
fn reparenting_moves_a_subtree_and_keeps_ids() {
    let (doc, _) = load(ARENA);
    let mut scene = doc.unwrap().scene;
    let floor = scene.resolve_path("/Arena01/Floor").unwrap();
    let brazier = scene.resolve_path("/Arena01/Brazier").unwrap();
    let uid_before = scene.get(brazier).unwrap().uid;
    scene.reparent(brazier, floor).unwrap();
    assert_eq!(scene.path_of(brazier).unwrap(), "/Arena01/Floor/Brazier");
    assert_eq!(
        scene.resolve_path("/Arena01/Floor/Brazier/Glow"),
        Some(scene.resolve_path("/Arena01/Floor/Brazier/Glow").unwrap())
    );
    assert_eq!(
        scene.get(brazier).unwrap().uid,
        uid_before,
        "ids are permanent"
    );
}

#[test]
fn integers_widen_to_scalars_but_scalars_do_not_narrow_to_integers() {
    // `radius = 72` is unambiguous and lossless, so it is accepted and
    // canonical form rewrites it as `72.0`.
    let src = scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Light2D\"\nname = \"L\"\nparent = \"n_root0000\"\nradius = 72\n"));
    let (doc, diags) = load(&src);
    assert!(!diags.has_errors(), "{diags}");
    let scene = doc.unwrap().scene;
    let light = scene.resolve_path("/Arena01/L").unwrap();
    assert_eq!(
        scene.get(light).unwrap().get("radius"),
        Some(&Value::Scalar(Fx::from_int(72)))
    );
    let canonical = dimetric_scene::write::to_canonical_text(&scene, &registry(), None);
    assert!(canonical.contains("radius = 72.0"));

    // The other direction loses information, so it is refused.
    let bad = scene_with(&(root() + "\n[[node]]\nid = \"n_aaaaaaaa\"\nkind = \"Light2D\"\nname = \"L\"\nparent = \"n_root0000\"\nz = 1.5\n"));
    let (_, diags) = load(&bad);
    assert!(
        diags.iter().any(|d| d.code == Code::TYPE_MISMATCH),
        "{diags}"
    );
}

#[test]
fn hand_written_ids_survive_a_round_trip_unchanged() {
    // `n_sk_stats` is not in the generation alphabet, and must still come back
    // out spelled exactly as it went in.
    let uid = NodeUid::parse("n_sk_stats").unwrap();
    assert_eq!(uid.to_text(), "n_sk_stats");
    let (doc, diags) = load(SKELETON);
    assert!(!diags.has_errors(), "{diags}");
    assert_eq!(doc.unwrap().to_text(), SKELETON);
}

#[test]
fn formatting_a_scene_does_not_change_its_state_hash() {
    // Canonical form omits properties equal to their default. If the loader
    // did not fill those defaults back in, `dim scene fmt` would silently
    // change every recorded replay — a formatter that alters behaviour is
    // worse than no formatter.
    let (doc, _) = load(ARENA);
    let scene = doc.unwrap().scene;
    let mut before = dimetric_core::StateHasher::new();
    scene.hash_state(&mut before);

    let canonical = dimetric_scene::write::to_canonical_text(&scene, &registry(), None);
    assert!(
        !canonical.contains("collision = false"),
        "the formatter should be omitting defaults, or this test proves nothing"
    );
    let (reloaded, diags) = load(&canonical);
    assert!(!diags.has_errors(), "{diags}");
    let mut after = dimetric_core::StateHasher::new();
    reloaded.unwrap().scene.hash_state(&mut after);

    assert_eq!(before.finish(), after.finish());
}

#[test]
fn a_node_carries_its_defaults_whether_or_not_the_file_spells_them_out() {
    let (doc, _) = load(ARENA);
    let scene = doc.unwrap().scene;
    let floor = scene.resolve_path("/Arena01/Floor").unwrap();
    // `collision` is absent from the file; the model still has it.
    assert_eq!(
        scene.get(floor).unwrap().get("collision"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        scene.get(floor).unwrap().get("modulate"),
        Some(&Value::Color(dimetric_scene::Color::WHITE))
    );
}

// -- formatting in place ------------------------------------------------
//
// `dim scene fmt` canonicalises the parsed document rather than rendering a
// new one, because §6.1 picked TOML partly so that comments survive. A
// formatter that deletes the notes an author wrote is not a formatter.

/// Format a scene the way `dim scene fmt` does.
fn fmt(src: &str) -> String {
    let (doc, diags) = load(src);
    assert!(!diags.has_errors(), "fixture must load: {diags}");
    let mut doc = doc.unwrap();
    let scene = std::mem::take(&mut doc.scene);
    dimetric_scene::write::format_in_place(&mut doc.doc, &scene, &registry(), None);
    doc.to_text()
}

#[test]
fn formatting_keeps_a_comment_written_above_the_scene_block() {
    // The one place a header comment naturally goes, and the one place it used
    // to be deleted: `[scene]`'s prefix was overwritten with a bare newline.
    let noted = ARENA.replace(
        "[scene]",
        "# What this room is for, and what not to break in it.\n[scene]",
    );
    let out = fmt(&noted);
    assert!(
        out.contains("# What this room is for, and what not to break in it."),
        "the scene's own header comment was dropped:\n{out}"
    );
}

#[test]
fn formatting_keeps_the_comments_the_author_wrote() {
    let messy = ARENA.replace(
        "[[node]]\nid = \"n_m9v2ht5w\"",
        "# Lit from above so the floor tiles read.\n# Do not move: the walk test lands here.\n\
         [[node]]\nid = \"n_m9v2ht5w\"",
    );
    let out = fmt(&messy);
    assert!(out.contains("# ── Environment"), "section comment survives");
    assert!(out.contains("# Lit from above so the floor tiles read."));
    assert!(out.contains("# Do not move: the walk test lands here."));
    // And the comment stayed with the node it was written above.
    let block = out.split("[[node]]").find(|b| b.contains("n_m9v2ht5w"));
    let preceding = out
        .split("[[node]]")
        .take_while(|b| !b.contains("n_m9v2ht5w"))
        .last()
        .unwrap();
    assert!(block.is_some());
    assert!(
        preceding.contains("# Do not move"),
        "the comment block should sit directly above its node:\n{out}"
    );
}

#[test]
fn formatting_keeps_a_trailing_comment_on_a_value() {
    let messy = ARENA.replace(
        "radius = 72.0",
        "radius = 72.0  # tuned against the brazier sprite",
    );
    let out = fmt(&messy);
    assert!(
        out.contains("radius = 72.0  # tuned against the brazier sprite"),
        "{out}"
    );
}

#[test]
fn formatting_in_place_is_idempotent() {
    let messy = ARENA.replace(
        "[[node]]\nid = \"n_p4rr01ez\"\nkind = \"Light2D\"\nname = \"Glow\"",
        "# A glow.\n[[node]]\nname = \"Glow\"\nkind = \"Light2D\"\nid = \"n_p4rr01ez\"",
    );
    let once = fmt(&messy);
    let twice = fmt(&once);
    assert_eq!(once, twice, "formatting must settle:\n{once}");
}

#[test]
fn a_file_without_comments_formats_to_the_same_text_either_way() {
    // The two writers must agree, or `fmt` and a fresh render would fight.
    let (doc, _) = load(ARENA);
    let scene = doc.unwrap().scene;
    let rendered = dimetric_scene::write::to_canonical_text(&scene, &registry(), None);
    assert_eq!(fmt(&rendered), rendered);
}

#[test]
fn formatting_sorts_keys_and_drops_defaults_without_touching_comments() {
    let messy = ARENA.replace(
        "kind = \"Sprite2D\"\nname = \"Brazier\"\nparent = \"n_root0000\"          # /Arena01\npos = [96.0, 48.0]",
        "pos = [96.0, 48.0]\nvisible = true\nname = \"Brazier\"\nparent = \"n_root0000\"\nkind = \"Sprite2D\"",
    );
    let out = fmt(&messy);
    let brazier = out
        .split("[[node]]")
        .find(|b| b.contains("n_m9v2ht5w"))
        .unwrap();
    let keys: Vec<&str> = brazier
        .lines()
        .filter_map(|l| {
            l.split(" =")
                .next()
                .filter(|k| !k.starts_with('#') && !k.is_empty())
        })
        .collect();
    // Reserved keys first in their fixed order, then kind properties alphabetically.
    assert_eq!(
        keys,
        ["id", "kind", "name", "parent", "pos", "z", "texture"]
    );
    assert!(!out.contains("visible = true"), "defaults are dropped");
    assert!(out.contains("# ── Environment"));
}

#[test]
fn formatting_moves_a_block_that_is_in_the_wrong_place() {
    // Nodes come before chunks whatever order the file had them in, and the
    // comment above a block travels with it.
    let chunk =
        "# Ground floor.\n[[chunk]]\nlayer = \"n_k3xq7a2p\"\nat = [0, 0]\ndata = \"1024:0\"\n";
    let out = fmt(&ARENA.replace("# ── Environment", &format!("{chunk}\n# ── Environment")));

    let chunk_at = out.find("[[chunk]]").expect("chunk survives");
    let last_node = out.rfind("[[node]]").expect("nodes survive");
    assert!(chunk_at > last_node, "chunks belong after nodes:\n{out}");
    assert!(
        out[..chunk_at].ends_with("# Ground floor.\n"),
        "the comment moves with its block:\n{out}"
    );
}

#[test]
fn formatting_rewrites_a_stale_path_comment() {
    let stale = ARENA.replace("# /Arena01/Brazier", "# /Arena01/SomethingElse");
    let out = fmt(&stale);
    assert!(
        out.contains("parent = \"n_m9v2ht5w\"          # /Arena01/Brazier"),
        "{out}"
    );
    assert!(!out.contains("SomethingElse"));
}

#[test]
fn formatting_in_place_does_not_change_the_state_hash() {
    let messy = ARENA.replace("radius = 72.0", "visible = true\nradius = 72.0  # keep");
    let (before_doc, _) = load(&messy);
    let mut before = dimetric_core::StateHasher::new();
    before_doc.unwrap().scene.hash_state(&mut before);

    let (after_doc, diags) = load(&fmt(&messy));
    assert!(!diags.has_errors(), "formatted output must parse: {diags}");
    let mut after = dimetric_core::StateHasher::new();
    after_doc.unwrap().scene.hash_state(&mut after);

    assert_eq!(before.finish(), after.finish());
}
