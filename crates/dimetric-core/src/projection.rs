//! How world space maps to screen space, in fixed point.
//!
//! This lives in `dimetric-core` rather than in the renderer, and the reason is
//! the whole of the request that prompted it. The projection used to be purely
//! presentation — the simulation never knew which one was in use, which was a
//! good property. Then the pointer became input a script can read, and picking
//! a world cell from a canvas pixel means inverting the camera.
//!
//! A script could do that arithmetic itself. It would be a second copy of the
//! renderer's maths that nothing keeps in sync, in a game where the projection
//! *is* the view, and when it drifted the symptom would be clicks landing one
//! cell off at certain camera positions — a bug that reproduces for nobody.
//!
//! So the maths is here, once, in fixed point. What a script gets is exact and
//! identical on every machine, because a picked cell decides what the
//! simulation does and is therefore hashed like any other decision.
//!
//! The float versions the drawing path needs are *not* here: they live on
//! `dimetric_render::ProjectionRender`, because this crate is one the I3 lint
//! covers and an `f32` in it would have to be excused line by line. Splitting
//! them that way means the exemption is not needed at all — the exact maths is
//! in the exact crate and the presentation maths is at the presentation
//! boundary.

use crate::{Fx, Vec2Fx};
use serde::{Deserialize, Serialize};

/// How world space maps to screen space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Projection {
    /// World units are screen pixels. The identity.
    #[default]
    TopDown,
    /// The 2:1 shear the pixel-art genre actually ships.
    ///
    /// Despite what it is universally called, this is not isometric. True
    /// isometric projection puts 120° between all three axes; a 2:1 tile ratio
    /// is *dimetric*, where one axis foreshortens differently from the others.
    /// The engine is named for the projection it really uses; the enum keeps
    /// the name people search for.
    Isometric,
}

impl Projection {
    /// Map a world position to screen space, in fixed point.
    pub fn project(self, world: Vec2Fx) -> Vec2Fx {
        match self {
            Projection::TopDown => world,
            Projection::Isometric => Vec2Fx::new(world.x - world.y, (world.x + world.y) / 2),
        }
    }

    /// Map a screen position back to world space, in fixed point.
    ///
    /// Not an exact inverse of [`project`](Projection::project) at every input,
    /// and it cannot be: the forward direction halves a sum, which discards the
    /// low bit of an odd value, and no inverse recovers a bit that is gone. It
    /// *is* exact to well within a tile, which is what picking needs — the test
    /// that matters is that a cell round-trips to itself, not that a sixteenth
    /// of a pixel does.
    pub fn unproject(self, screen: Vec2Fx) -> Vec2Fx {
        match self {
            Projection::TopDown => screen,
            Projection::Isometric => {
                let half = screen.x / 2;
                Vec2Fx::new(screen.y + half, screen.y - half)
            }
        }
    }

    /// Depth ordering for a world position under this projection.
    ///
    /// Y-sorting is the default for both projections: in top-down, something
    /// further down the screen is nearer; in dimetric, so is something further
    /// along both axes.
    pub fn depth_of(self, world: Vec2Fx) -> Fx {
        match self {
            Projection::TopDown => world.y,
            Projection::Isometric => world.x + world.y,
        }
    }
}
