//! Atlas packing.
//!
//! Sprites batch by texture, so putting everything in one image is what turns a
//! thousand sprites into one draw call. This runs at import, and the result is
//! cached: startup uploads a sheet rather than computing one.
//!
//! It lives here rather than in the renderer because it is arithmetic over
//! rectangles. Nothing about it needs a GPU, and a packer that needs a GPU
//! cannot run in the importer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::image::Image;

/// Space left between packed images.
///
/// One pixel, so that linear filtering or a half-pixel sampling error cannot
/// pull a neighbour's colour into a sprite's edge. Nearest sampling does not
/// need it, but a project can turn nearest off.
pub const PADDING: u32 = 1;

/// Where one image ended up on the sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Placement {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// One packed image and the map of what is where in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sheet {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA pixel data.
    pub pixels: Vec<u8>,
    /// Placement by asset name, in name order.
    pub placements: BTreeMap<String, Placement>,
}

/// Pack images into a single sheet.
///
/// Shelf packing: tallest first, laid left to right in rows. Not the tightest
/// algorithm, but it is simple and — sorted by height and then by name —
/// completely deterministic, which matters because a golden image of a
/// differently-packed atlas is a different image.
pub fn pack(mut images: Vec<Image>, max_width: u32) -> Sheet {
    images.sort_by(|a, b| b.height.cmp(&a.height).then(a.name.cmp(&b.name)));

    let width = max_width.max(
        images
            .iter()
            .map(|s| s.width + PADDING * 2)
            .max()
            .unwrap_or(1),
    );

    // First pass: decide where everything goes and how tall the result is.
    let mut positions = Vec::with_capacity(images.len());
    let (mut x, mut y, mut shelf_height) = (PADDING, PADDING, 0u32);
    for image in &images {
        if x + image.width + PADDING > width && x > PADDING {
            x = PADDING;
            y += shelf_height + PADDING;
            shelf_height = 0;
        }
        positions.push((x, y));
        x += image.width + PADDING;
        shelf_height = shelf_height.max(image.height);
    }
    let height = (y + shelf_height + PADDING).max(1);

    // Shrink to what was actually used. `max_width` is a ceiling, not a target,
    // and a project with six sprites should not ship a 2048-wide texture. Only
    // the denominator of the texture coordinates changes, so the texels a
    // sprite samples are the same ones either way.
    let width = positions
        .iter()
        .zip(images.iter())
        .map(|((x, _), image)| x + image.width + PADDING)
        .max()
        .unwrap_or(1)
        .min(width)
        .max(1);

    let mut sheet = Image::blank("", width, height);
    let mut placements = BTreeMap::new();
    for (image, (ox, oy)) in images.iter().zip(positions) {
        sheet.blit(image, ox, oy);
        placements.insert(
            image.name.clone(),
            Placement {
                x: ox,
                y: oy,
                width: image.width,
                height: image.height,
            },
        );
    }

    Sheet {
        width,
        height,
        pixels: sheet.pixels,
        placements,
    }
}
