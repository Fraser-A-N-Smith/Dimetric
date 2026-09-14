//! The viewport: a camera over the scene, and the handles drawn on it.
//!
//! Hit-testing and drag arithmetic live here rather than in a client, because
//! "clicking a gizmo picks the nearest node within the handle's radius" is a
//! rule worth a test, and a rule inside a paint callback is not one.

use dimetric_core::{Fx, NodeUid, Vec2Fx};

use crate::editor::Editor;

/// Where the editor is looking.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    /// Centre of the view, in world space.
    pub centre: Vec2Fx,
    /// World units per screen pixel, inverted: 2.0 draws everything twice size.
    pub zoom: Fx,
    /// Size of the drawing area, in pixels.
    pub size: (u32, u32),
}

impl Viewport {
    /// The viewport a sidecar describes.
    pub fn from_sidecar(editor: &Editor, size: (u32, u32)) -> Viewport {
        let parse = |s: &str| Fx::parse_exact(s).unwrap_or(Fx::ZERO);
        let zoom = Fx::parse_exact(&editor.sidecar.zoom).unwrap_or(Fx::ONE);
        Viewport {
            centre: Vec2Fx::new(
                parse(&editor.sidecar.camera[0]),
                parse(&editor.sidecar.camera[1]),
            ),
            // A zoom of zero would divide by nothing and put every node in the
            // same place, so it is treated as unset.
            zoom: if zoom == Fx::ZERO { Fx::ONE } else { zoom },
            size,
        }
    }

    /// Where a world point lands on screen, in pixels from the top left.
    pub fn to_screen(&self, world: Vec2Fx) -> (f32, f32) {
        let half = (self.size.0 as f32 / 2.0, self.size.1 as f32 / 2.0);
        let zoom = self.zoom.to_f32();
        (
            half.0 + (world.x - self.centre.x).to_f32() * zoom,
            half.1 + (world.y - self.centre.y).to_f32() * zoom,
        )
    }

    /// Where a screen point lands in the world.
    ///
    /// The inverse of [`Viewport::to_screen`], and the reason a drag can be
    /// turned back into a position: the pointer is in pixels and the scene is
    /// in fixed point, and the conversion happens once, here, at the boundary.
    pub fn to_world(&self, screen: (f32, f32)) -> Vec2Fx {
        let half = (self.size.0 as f32 / 2.0, self.size.1 as f32 / 2.0);
        let zoom = self.zoom.to_f32().max(f32::MIN_POSITIVE);
        Vec2Fx::new(
            self.centre.x + Fx::from_f64_lossy(((screen.0 - half.0) / zoom) as f64),
            self.centre.y + Fx::from_f64_lossy(((screen.1 - half.1) / zoom) as f64),
        )
    }
}

/// A handle drawn on the viewport.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gizmo {
    /// Node it belongs to.
    pub node: NodeUid,
    /// Where it sits in world space.
    pub at: Vec2Fx,
    /// Whether the node is selected.
    pub selected: bool,
}

/// How close, in pixels, a click has to be to pick a gizmo.
pub const HANDLE_RADIUS: f32 = 8.0;

/// A handle for every node that has a position.
pub fn gizmos(editor: &Editor) -> Vec<Gizmo> {
    let Some(doc) = editor.project.open.as_ref() else {
        return Vec::new();
    };
    doc.scene
        .walk()
        .into_iter()
        .filter_map(|id| {
            let node = doc.scene.get(id)?;
            Some(Gizmo {
                node: node.uid,
                at: doc.scene.world_of(id)?.pos,
                selected: editor.sidecar.selection.contains(&node.uid),
            })
        })
        .collect()
}

/// The node a click at a screen point picks, if any.
///
/// The nearest handle within the radius, and a selected node wins a tie so that
/// clicking something already selected does not jump to whatever shares its
/// position.
pub fn pick(gizmos: &[Gizmo], viewport: &Viewport, screen: (f32, f32)) -> Option<NodeUid> {
    let mut best: Option<(f32, bool, NodeUid)> = None;
    for gizmo in gizmos {
        let (x, y) = viewport.to_screen(gizmo.at);
        let distance = ((x - screen.0).powi(2) + (y - screen.1).powi(2)).sqrt();
        if distance > HANDLE_RADIUS {
            continue;
        }
        let better = match best {
            None => true,
            Some((d, selected, _)) => {
                (distance, gizmo.selected) < (d, selected)
                    || (distance == d && gizmo.selected && !selected)
            }
        };
        if better {
            best = Some((distance, gizmo.selected, gizmo.node));
        }
    }
    best.map(|(_, _, node)| node)
}
