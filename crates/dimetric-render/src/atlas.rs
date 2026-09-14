//! Runtime texture atlas.
//!
//! Sprites batch by atlas, so putting everything in one texture is what turns a
//! thousand sprites into one draw call.
//!
//! The packing itself lives in `dimetric-assets`, where it runs at import time
//! and the result is cached. What is here is the renderer's view of a packed
//! sheet: texture coordinates, and the lookup the batcher does per sprite.

use dimetric_assets::sheet::{pack, Framed, Placement, Sheet};
use dimetric_scene::Color;

/// An image waiting to be packed.
pub type Source = dimetric_assets::Image;

/// Why an image could not be read.
pub use dimetric_assets::ImageError;

/// Where one image sits in the atlas.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Region {
    /// Texture coordinates, `[u_min, v_min, u_max, v_max]`.
    pub uv: [f32; 4],
    /// Size in pixels.
    pub size: (u32, u32),
}

impl Region {
    /// A sub-rectangle of this region, in pixels relative to its own origin.
    ///
    /// Used for a `Sprite2D` with a `region` property, and for picking one
    /// frame out of a sheet.
    pub fn sub(&self, x: u32, y: u32, width: u32, height: u32) -> Region {
        let (uw, vh) = (self.uv[2] - self.uv[0], self.uv[3] - self.uv[1]);
        let (sw, sh) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        Region {
            uv: [
                self.uv[0] + uw * (x as f32 / sw),
                self.uv[1] + vh * (y as f32 / sh),
                self.uv[0] + uw * ((x + width) as f32 / sw),
                self.uv[1] + vh * ((y + height) as f32 / sh),
            ],
            size: (width, height),
        }
    }
}

/// One packed texture and the regions inside it.
pub struct Atlas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA pixel data.
    pub pixels: Vec<u8>,
    sheet: Sheet,
}

impl Atlas {
    /// Pack sources into a single texture.
    pub fn pack(sources: Vec<Source>, max_width: u32) -> Atlas {
        Atlas::from_sheet(pack(sources, max_width))
    }

    /// Pack images that may be animation strips.
    pub fn pack_framed(framed: Vec<Framed>, max_width: u32) -> Atlas {
        Atlas::from_sheet(dimetric_assets::sheet::pack_framed(framed, max_width))
    }

    /// Take a sheet the importer already packed.
    pub fn from_sheet(sheet: Sheet) -> Atlas {
        Atlas {
            width: sheet.width,
            height: sheet.height,
            pixels: sheet.pixels.clone(),
            sheet,
        }
    }

    /// Look up a region by asset name.
    pub fn region(&self, name: &str) -> Option<Region> {
        self.sheet.placements.get(name).map(|p| self.region_of(p))
    }

    /// Every region, in name order.
    pub fn regions(&self) -> impl Iterator<Item = (&str, Region)> {
        self.sheet
            .placements
            .iter()
            .map(|(name, p)| (name.as_str(), self.region_of(p)))
    }

    /// How many animation frames an image holds, side by side.
    ///
    /// One for a still, which is why a caller can slice unconditionally.
    pub fn frames(&self, name: &str) -> u32 {
        self.sheet
            .placements
            .get(name)
            .map(|p| p.frames.max(1))
            .unwrap_or(1)
    }

    /// How many images are packed.
    pub fn len(&self) -> usize {
        self.sheet.placements.len()
    }

    /// True when nothing is packed.
    pub fn is_empty(&self) -> bool {
        self.sheet.placements.is_empty()
    }

    fn region_of(&self, p: &Placement) -> Region {
        let (w, h) = (self.width.max(1) as f32, self.height.max(1) as f32);
        Region {
            uv: [
                p.x as f32 / w,
                p.y as f32 / h,
                (p.x + p.width) as f32 / w,
                (p.y + p.height) as f32 / h,
            ],
            size: (p.width, p.height),
        }
    }
}

/// Read a PNG into RGBA pixels.
///
/// A direct read, for a caller that has a path and wants pixels — golden-image
/// comparison, mostly. A project's own textures come through the import cache
/// instead, which is where `.meta` settings and content hashing apply.
pub fn load_png(path: &std::path::Path) -> Result<Source, ImageError> {
    dimetric_assets::decode_png(path)
}

/// A checkerboard standing in for a texture the project does not have.
///
/// Drawing something loud is better than drawing nothing: a missing texture
/// should be obvious in a screenshot, and a scene with one broken reference
/// should still open.
pub fn placeholder(size: u32) -> Source {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let light = (x / 4 + y / 4) % 2 == 0;
            pixels.extend_from_slice(if light {
                &[0xE0, 0x3F, 0xB0, 0xFF]
            } else {
                &[0x28, 0x2C, 0x3A, 0xFF]
            });
        }
    }
    Source {
        name: PLACEHOLDER_NAME.to_string(),
        width: size,
        height: size,
        pixels,
    }
}

/// Name the placeholder is registered under.
pub const PLACEHOLDER_NAME: &str = "__missing";

/// A single opaque pixel, so untextured geometry can share the sprite pipeline.
pub fn solid(color: Color) -> Source {
    Source {
        name: SOLID_NAME.to_string(),
        width: 1,
        height: 1,
        pixels: vec![color.r, color.g, color.b, color.a],
    }
}

/// Name the solid pixel is registered under.
pub const SOLID_NAME: &str = "__solid";
