//! The built-in node kinds.
//!
//! These are registered exactly the way a third-party kind would be. Nothing
//! here reaches into the engine in a way a plugin could not.

use dimetric_core::Fx;

use crate::schema::{NodeKindSchema, PropertySchema, PropertyType};
use crate::value::{Color, Value};

fn scalar(v: &str) -> Option<Value> {
    Some(Value::Scalar(
        Fx::parse_exact(v).expect("built-in defaults are exact"),
    ))
}
fn int(v: i64) -> Option<Value> {
    Some(Value::Int(v))
}
fn boolean(v: bool) -> Option<Value> {
    Some(Value::Bool(v))
}
fn text(v: &str) -> Option<Value> {
    Some(Value::Str(v.into()))
}
fn variant(v: &str) -> Option<Value> {
    Some(Value::Enum(v.into()))
}
fn color(v: &str) -> Option<Value> {
    Some(Value::Color(
        Color::parse(v).expect("built-in colours are valid"),
    ))
}
fn enum_of(names: &[&str]) -> PropertyType {
    PropertyType::Enum(names.iter().map(|s| s.to_string()).collect())
}
fn prop(name: &str, ty: PropertyType, default: Option<Value>, doc: &str) -> PropertySchema {
    PropertySchema::new(name, ty, default, doc)
}

/// Collision layer and mask bits, shared by `Collider` and `Area`.
fn layer_bits() -> Vec<PropertySchema> {
    vec![
        prop(
            "collision_layer",
            PropertyType::Int,
            int(1),
            "Bitmask of layers this body occupies.",
        ),
        prop(
            "collision_mask",
            PropertyType::Int,
            int(1),
            "Bitmask of layers this body tests against.",
        ),
    ]
}

/// Shape selection, shared by `Collider` and `Area`.
fn shape_props() -> Vec<PropertySchema> {
    vec![
        prop(
            "shape",
            enum_of(&["AABB", "Circle", "Polygon"]),
            variant("AABB"),
            "Which of the size, radius or points properties applies.",
        ),
        prop(
            "size",
            PropertyType::Vec2,
            Some(Value::Vec2(dimetric_core::Vec2Fx::from_ints(16, 16))),
            "Full extents, for an AABB.",
        ),
        prop(
            "radius",
            PropertyType::Scalar,
            scalar("8.0"),
            "Radius, for a circle.",
        )
        .ranged(Fx::ZERO, Fx::MAX),
        prop(
            "points",
            PropertyType::List(Box::new(PropertyType::Vec2)),
            Some(Value::List(Vec::new())),
            "Convex hull in local space, counter-clockwise, for a polygon.",
        ),
    ]
}

/// Every kind the engine ships with.
pub fn builtin_kinds() -> Vec<NodeKindSchema> {
    let mut kinds = vec![NodeKindSchema::new(
        "Node",
        "Bare node. Groups children and runs a script; has no presence in the world.",
        vec![],
    )];

    kinds.push(NodeKindSchema::new(
        "Node2D",
        "A node with a transform. The base for anything that exists somewhere.",
        vec![],
    ));

    kinds.push(NodeKindSchema::new(
        "Sprite2D",
        "A textured quad.",
        vec![
            prop("texture", PropertyType::AssetRef, None, "Texture to draw.").required(),
            prop(
                "region",
                PropertyType::Rect,
                None,
                "Sub-rectangle of the texture, in pixels. Whole texture when absent.",
            ),
            prop("modulate", PropertyType::Color, color("#ffffffff"), "Tint."),
            prop(
                "offset",
                PropertyType::Vec2,
                Some(Value::Vec2(dimetric_core::Vec2Fx::ZERO)),
                "Draw offset from the node origin.",
            ),
            prop(
                "flip_h",
                PropertyType::Bool,
                boolean(false),
                "Mirror horizontally.",
            ),
            prop(
                "flip_v",
                PropertyType::Bool,
                boolean(false),
                "Mirror vertically.",
            ),
            prop(
                "blend",
                enum_of(&["Alpha", "Additive", "Multiply"]),
                variant("Alpha"),
                "Blend mode. Also part of the batching key.",
            ),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "AnimatedSprite2D",
        "A sprite playing frame clips imported from Aseprite tags.",
        vec![
            prop(
                "frames",
                PropertyType::AssetRef,
                None,
                "Imported sprite sheet carrying the named clips.",
            )
            .required(),
            prop(
                "animation",
                PropertyType::Str,
                text(""),
                "Clip to play. Empty means the first.",
            ),
            prop(
                "playing",
                PropertyType::Bool,
                boolean(true),
                "Advance frames on tick.",
            ),
            prop(
                "looping",
                PropertyType::Bool,
                boolean(true),
                "Restart at the end of the clip.",
            ),
            prop(
                "frame",
                PropertyType::Int,
                int(0),
                "Frame showing now. Written by the engine as the clip plays, and \
                 read by the renderer to pick a slice of the sheet — the one-way \
                 path from simulation to presentation that invariant I7 asks for. \
                 Setting it by hand pins a frame until playback moves it.",
            ),
            prop("modulate", PropertyType::Color, color("#ffffffff"), "Tint."),
            // The same two `Sprite2D` has, with the same meaning. Under a 2:1
            // shear a grid actor needs four screen facings and two of them are
            // mirrors, so a mirrored sheet halves the art for a directional
            // character. It is a render-time flip of the quad and reaches no
            // simulation state — which way a character faces is gameplay, but
            // the flip itself is not.
            prop(
                "flip_h",
                PropertyType::Bool,
                boolean(false),
                "Mirror horizontally.",
            ),
            prop(
                "flip_v",
                PropertyType::Bool,
                boolean(false),
                "Mirror vertically.",
            ),
            prop(
                "speed_numerator",
                PropertyType::Int,
                int(1),
                "Playback rate numerator. Integer ratio, not a float, because frame \
                 advance is gameplay and must land on the same tick everywhere.",
            ),
            prop(
                "speed_denominator",
                PropertyType::Int,
                int(1),
                "Playback rate denominator.",
            ),
            prop(
                "blend",
                enum_of(&["Alpha", "Additive", "Multiply"]),
                variant("Alpha"),
                "Blend mode.",
            ),
        ],
    ));

    let mut collider = shape_props();
    collider.extend(layer_bits());
    collider.push(prop(
        "is_static",
        PropertyType::Bool,
        boolean(false),
        "Immovable. Static bodies go in the broadphase but are never swept.",
    ));
    collider.push(prop(
        "one_way",
        PropertyType::Bool,
        boolean(false),
        "Blocks only motion opposing the local +Y axis.",
    ));
    collider.push(prop(
        "pushable",
        PropertyType::Bool,
        boolean(false),
        "Receives the simple impulse response instead of only blocking.",
    ));
    kinds.push(NodeKindSchema::new(
        "Collider",
        "A solid body. Blocks movement and reports contacts.",
        collider,
    ));

    let mut area = shape_props();
    area.extend(layer_bits());
    area.push(prop(
        "monitoring",
        PropertyType::Bool,
        boolean(true),
        "Report overlaps. A disabled area costs nothing in the broadphase.",
    ));
    kinds.push(NodeKindSchema::new(
        "Area",
        "A trigger volume. Reports overlaps and blocks nothing.",
        area,
    ));

    // -- UI ---------------------------------------------------------------
    //
    // A control is laid out against a fixed canvas rather than the window; see
    // `crate::ui` for why that is not an arbitrary choice.
    let control_props = |extra: Vec<PropertySchema>| {
        let mut props = vec![
            prop(
                "anchor_left",
                PropertyType::Scalar,
                scalar("0.0"),
                "Left edge as a fraction of the parent's width.",
            ),
            prop(
                "anchor_top",
                PropertyType::Scalar,
                scalar("0.0"),
                "Top edge as a fraction of the parent's height.",
            ),
            prop(
                "anchor_right",
                PropertyType::Scalar,
                scalar("0.0"),
                "Right edge as a fraction of the parent's width.",
            ),
            prop(
                "anchor_bottom",
                PropertyType::Scalar,
                scalar("0.0"),
                "Bottom edge as a fraction of the parent's height.",
            ),
            prop(
                "offset_left",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels from the left anchor.",
            ),
            prop(
                "offset_top",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels from the top anchor.",
            ),
            prop(
                "offset_right",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels from the right anchor.",
            ),
            prop(
                "offset_bottom",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels from the bottom anchor.",
            ),
            prop(
                "catches_input",
                PropertyType::Bool,
                boolean(true),
                "Whether a pointer over this control hits it. False makes it scenery.",
            ),
            prop(
                "focusable",
                PropertyType::Bool,
                boolean(false),
                "Whether keyboard or pad focus can land on this control.",
            ),
        ];
        props.extend(extra);
        props
    };

    // The engine writes this every tick from what the pointer is doing; a
    // scene that sets it is overwritten. It is a property rather than a
    // side-channel because that is how everything else the simulation decides
    // reaches the renderer, and it makes a button's appearance fall out of
    // ordinary extraction.
    let interaction = || {
        prop(
            "state",
            PropertyType::Int,
            int(0),
            "Written by the engine: 0 idle, 1 hovered, 2 pressed. Setting it has no effect.",
        )
    };

    kinds.push(NodeKindSchema::new(
        "Control",
        "A rectangle in UI space, positioned by anchors and offsets. Draws nothing itself.",
        control_props(vec![interaction()]),
    ));

    // Behaves as a `Control`, so layout, hit testing and anything else that
    // asks what a node *is* handle it without knowing the name — the same
    // machinery a project's own kinds use when they extend a built-in.
    kinds.push(
        NodeKindSchema::new(
            "Panel",
            "A control filled with a colour.",
            control_props(vec![
                interaction(),
                prop(
                    "modulate",
                    PropertyType::Color,
                    color("#000000a0"),
                    "Fill colour.",
                ),
            ]),
        )
        .based_on("Control"),
    );

    // A button is a panel that knows what the pointer is doing to it. Three
    // colours rather than a theme lookup: a project that wants one palette
    // puts the colours in a prefab and instances it, which is a mechanism the
    // engine already has and does not need a second one for.
    kinds.push(
        NodeKindSchema::new(
            "Button",
            "A control that reacts to the pointer. Its caption is a Label child.",
            control_props(vec![
                interaction(),
                prop(
                    "modulate",
                    PropertyType::Color,
                    color("#303040ff"),
                    "Fill colour when idle.",
                ),
                // Transparent means "work it out from `modulate`", so a
                // button needs one colour to look right and three only if
                // somebody wants three. A literal transparent hover would be
                // a button that vanishes under the pointer, which nobody
                // wants, so the sentinel costs nothing.
                prop(
                    "modulate_hover",
                    PropertyType::Color,
                    color("#00000000"),
                    "Fill when hovered. Transparent means lighten the fill.",
                ),
                prop(
                    "modulate_pressed",
                    PropertyType::Color,
                    color("#00000000"),
                    "Fill when held. Transparent means darken the fill.",
                ),
            ]),
        )
        .based_on("Control"),
    );

    let box_props = |extra: Vec<PropertySchema>| {
        let mut props = control_props(vec![
            prop(
                "spacing",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels between one child and the next.",
            ),
            prop(
                "padding",
                PropertyType::Scalar,
                scalar("0.0"),
                "Pixels between the container's edge and its children.",
            ),
        ]);
        props.extend(extra);
        props
    };

    kinds.push(
        NodeKindSchema::new(
            "VBox",
            "Stacks its control children top to bottom. A child keeps its own height and fills the width.",
            box_props(vec![]),
        )
        .based_on("Control"),
    );
    kinds.push(
        NodeKindSchema::new(
            "HBox",
            "Stacks its control children left to right. A child keeps its own width and fills the height.",
            box_props(vec![]),
        )
        .based_on("Control"),
    );

    kinds.push(NodeKindSchema::new(
        "Label",
        "A run of text drawn from a baked font.",
        vec![
            // Optional now that the engine carries a font of its own. It was
            // required when the alternative was a label that drew nothing;
            // making somebody import a typeface before they can print a frame
            // counter is a worse default than a small one that always works.
            prop(
                "font",
                PropertyType::AssetRef,
                Some(Value::Ref(crate::value::Reference::Asset(
                    "builtin".to_string(),
                ))),
                "Font to draw with. Defaults to the engine's built-in.",
            ),
            prop(
                "text",
                PropertyType::Str,
                Some(Value::Str(String::new())),
                "What to draw. A newline starts a new line.",
            ),
            prop("modulate", PropertyType::Color, color("#ffffffff"), "Tint."),
            prop(
                "align",
                PropertyType::Enum(vec![
                    "Left".to_string(),
                    "Center".to_string(),
                    "Right".to_string(),
                ]),
                Some(Value::Enum("Left".to_string())),
                "Horizontal alignment of each line about the node origin.",
            ),
            prop(
                "offset",
                PropertyType::Vec2,
                Some(Value::Vec2(dimetric_core::Vec2Fx::ZERO)),
                "Draw offset from the node origin.",
            ),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "Camera2D",
        "A view onto the world.",
        vec![
            prop(
                "projection",
                enum_of(&["TopDown", "Isometric"]),
                variant("TopDown"),
                "Top-down is identity; isometric is a 2:1 shear applied at render time. \
                 The simulation is free-form 2D either way — this flag never reaches it.",
            ),
            prop("zoom", PropertyType::Scalar, scalar("1.0"), "Scale factor.")
                .ranged(Fx::from_raw(1), Fx::MAX),
            prop(
                "current",
                PropertyType::Bool,
                boolean(false),
                "Use this camera for rendering.",
            ),
            prop(
                "pixel_snap",
                PropertyType::Bool,
                boolean(true),
                "Round the camera to whole pixels, for pixel-art projects.",
            ),
            prop(
                "limits",
                PropertyType::Rect,
                None,
                "Region the view is confined to. Unconstrained when absent.",
            ),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "Light2D",
        "A light contributing to the additive light buffer.",
        vec![
            prop(
                "color",
                PropertyType::Color,
                color("#ffffffff"),
                "Light colour.",
            ),
            prop(
                "radius",
                PropertyType::Scalar,
                scalar("64.0"),
                "Falloff radius.",
            )
            .ranged(Fx::ZERO, Fx::MAX),
            prop(
                "energy",
                PropertyType::Scalar,
                scalar("1.0"),
                "Brightness multiplier.",
            )
            .ranged(Fx::ZERO, Fx::MAX),
            prop(
                "shape",
                enum_of(&["Radial", "Cone"]),
                variant("Radial"),
                "Radial fills the radius; cone is limited to cone_angle.",
            ),
            prop(
                "cone_angle",
                PropertyType::Angle,
                None,
                "Cone width, for a cone light.",
            ),
            prop(
                "cast_shadows",
                PropertyType::Bool,
                boolean(false),
                "Cast shadows from nearby collider geometry.",
            ),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "TileLayer",
        "A grid of tiles, stored as run-length encoded chunks.",
        vec![
            prop(
                "tileset",
                PropertyType::AssetRef,
                None,
                "Tileset to draw from.",
            )
            .required(),
            prop(
                "cell",
                PropertyType::Vec2i,
                Some(Value::Vec2i([16, 16])),
                "The grid's step in world units. Square under Isometric, which is \
                 what makes a 2:1 diamond tessellate.",
            ),
            prop(
                "tile_size",
                PropertyType::Vec2i,
                None,
                "The sprite's size in pixels, for slicing the sheet and drawing. \
                 Absent means `cell`. A dimetric floor wants `cell = [16, 16]` with \
                 `tile_size = [32, 16]`.",
            ),
            prop(
                "collision",
                PropertyType::Bool,
                boolean(false),
                "Feed this layer into the collision grid.",
            ),
            prop("modulate", PropertyType::Color, color("#ffffffff"), "Tint."),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "Sound",
        "A sound attached to a node. Presentation only — never hashed, never snapshotted.",
        vec![
            prop("stream", PropertyType::AssetRef, None, "Audio clip.").required(),
            prop(
                "bus",
                enum_of(&["Music", "Sfx", "Ui"]),
                variant("Sfx"),
                "Mixer bus to route through.",
            ),
            prop(
                "autoplay",
                PropertyType::Bool,
                boolean(false),
                "Start on ready.",
            ),
            prop(
                "looping",
                PropertyType::Bool,
                boolean(false),
                "Repeat when finished.",
            ),
            prop(
                "volume_db",
                PropertyType::Scalar,
                scalar("0.0"),
                "Gain in decibels.",
            ),
            prop(
                "pitch_variation",
                PropertyType::Scalar,
                scalar("0.0"),
                "Random pitch spread per trigger, drawn from the presentation RNG \
                 stream so it can never perturb gameplay.",
            )
            .ranged(Fx::ZERO, Fx::ONE),
        ],
    ));

    kinds.push(NodeKindSchema::new(
        "Instance",
        "An instance of another scene, with sparse property overrides. \
         The source is named by the reserved `scene` key.",
        vec![],
    ));

    kinds
}
