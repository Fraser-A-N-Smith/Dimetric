//! Reading LDtk projects, which is only ever half the story: baking what comes
//! back into chunks needs the command bus and happens a layer up.

use std::path::Path;

use dimetric_assets::ldtk;

/// A project with one level, two tile layers and a tileset.
fn project() -> String {
    serde_json::json!({
        "externalLevels": false,
        "defs": { "tilesets": [
            { "uid": 7, "identifier": "Dungeon" },
            { "uid": 9, "identifier": "Props" }
        ]},
        "levels": [{
            "identifier": "Arena01",
            // LDtk lists layers front to back.
            "layerInstances": [
                {
                    "__identifier": "Entities",
                    "__type": "Entities",
                    "__gridSize": 16,
                    "__cWid": 4, "__cHei": 4
                },
                {
                    "__identifier": "Props",
                    "__type": "Tiles",
                    "__gridSize": 16,
                    "__cWid": 4, "__cHei": 4,
                    "__tilesetDefUid": 9,
                    "gridTiles": [
                        { "px": [32, 16], "t": 4, "f": 0 }
                    ]
                },
                {
                    "__identifier": "Floor",
                    "__type": "AutoLayer",
                    "__gridSize": 16,
                    "__cWid": 4, "__cHei": 4,
                    "__tilesetDefUid": 7,
                    "autoLayerTiles": [
                        { "px": [16, 0], "t": 1, "f": 0 },
                        { "px": [0, 0], "t": 0, "f": 3 }
                    ]
                }
            ]
        }]
    })
    .to_string()
}

fn read(text: &str) -> Vec<ldtk::Level> {
    ldtk::parse(text, Path::new("test.ldtk")).expect("the fixture should parse")
}

#[test]
fn layers_come_back_back_to_front() {
    // LDtk draws its list front to back and chunks draw in scene order, so the
    // reader reverses. Getting this backwards puts the floor over the props.
    let levels = read(&project());
    assert_eq!(levels.len(), 1);
    assert_eq!(levels[0].name, "Arena01");
    let names: Vec<&str> = levels[0].layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Floor", "Props"]);
}

#[test]
fn entity_layers_are_left_alone() {
    // The boundary: LDtk owns tile layers, `.dim` owns every entity. Importing
    // entities would make the scene a generated file.
    let levels = read(&project());
    assert!(levels[0].layers.iter().all(|l| l.name != "Entities"));
}

#[test]
fn painted_and_rule_placed_tiles_are_the_same_thing_here() {
    let levels = read(&project());
    let floor = &levels[0].layers[0];
    assert_eq!(floor.tiles.len(), 2, "autoLayerTiles counted");
    let props = &levels[0].layers[1];
    assert_eq!(props.tiles.len(), 1, "gridTiles counted");
}

#[test]
fn pixel_positions_become_cell_coordinates() {
    let levels = read(&project());
    let props = &levels[0].layers[1];
    assert_eq!((props.tiles[0].x, props.tiles[0].y), (2, 1));
}

#[test]
fn tile_indices_are_shifted_up_so_zero_can_mean_empty() {
    // Chunk data reserves zero for an empty cell, so LDtk's tile 0 has to
    // become 1 or the first tile in every tileset would be invisible.
    let levels = read(&project());
    let floor = &levels[0].layers[0];
    let at_origin = floor.tiles.iter().find(|t| (t.x, t.y) == (0, 0)).unwrap();
    assert_eq!(at_origin.tile, 1);
}

#[test]
fn flips_survive_the_read() {
    let levels = read(&project());
    let floor = &levels[0].layers[0];
    let flipped = floor.tiles.iter().find(|t| (t.x, t.y) == (0, 0)).unwrap();
    assert!(flipped.flip_x && flipped.flip_y, "f: 3 is both axes");
}

#[test]
fn tiles_come_back_in_row_major_order() {
    let levels = read(&project());
    let floor = &levels[0].layers[0];
    let coords: Vec<(i32, i32)> = floor.tiles.iter().map(|t| (t.x, t.y)).collect();
    assert_eq!(coords, [(0, 0), (1, 0)]);
}

#[test]
fn the_tileset_a_layer_draws_from_is_named() {
    let levels = read(&project());
    assert_eq!(levels[0].layers[0].tileset.as_deref(), Some("Dungeon"));
    assert_eq!(levels[0].layers[1].tileset.as_deref(), Some("Props"));
}

#[test]
fn a_stack_of_tiles_in_one_cell_resolves_to_the_last_one() {
    // LDtk paints its list in order, so the last tile written to a cell is the
    // one you see. A chunk holds one tile per cell.
    let text = serde_json::json!({
        "defs": { "tilesets": [] },
        "levels": [{ "identifier": "L", "layerInstances": [{
            "__identifier": "Floor", "__type": "Tiles",
            "__gridSize": 8, "__cWid": 2, "__cHei": 2,
            "gridTiles": [
                { "px": [0, 0], "t": 3, "f": 0 },
                { "px": [0, 0], "t": 9, "f": 0 }
            ]
        }]}]
    })
    .to_string();
    let levels = read(&text);
    assert_eq!(levels[0].layers[0].tiles.len(), 1);
    assert_eq!(levels[0].layers[0].tiles[0].tile, 10);
}

#[test]
fn separate_level_files_are_refused_with_the_setting_to_change() {
    let text = serde_json::json!({ "externalLevels": true, "levels": [] }).to_string();
    let err = ldtk::parse(&text, Path::new("world.ldtk")).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Save levels to separate files"),
        "{message}"
    );
}

#[test]
fn something_that_is_not_an_ldtk_project_says_so() {
    let err = ldtk::parse("{\"levels\": 4}", Path::new("world.ldtk")).unwrap_err();
    assert!(err.to_string().contains("cannot read"));
}

#[test]
fn fields_this_build_does_not_know_about_are_ignored() {
    // LDtk's JSON carries a great deal besides tiles, and naming all of it
    // would mean a new field breaking import every time LDtk ships.
    let text = serde_json::json!({
        "iid": "whatever", "jsonVersion": "1.5.3", "worldLayout": "Free",
        "defs": { "tilesets": [], "entities": [{ "identifier": "Player" }] },
        "levels": [{
            "identifier": "L", "bgColor": "#000000", "worldX": 0,
            "layerInstances": [{
                "__identifier": "Floor", "__type": "Tiles", "__gridSize": 8,
                "__cWid": 1, "__cHei": 1, "seed": 12345, "overrideTilesetUid": null,
                "gridTiles": [{ "px": [0, 0], "src": [16, 0], "t": 2, "f": 0, "d": [0], "a": 1 }]
            }]
        }]
    })
    .to_string();
    let levels = read(&text);
    assert_eq!(levels[0].layers[0].tiles[0].tile, 3);
}
