//! Runtime texture atlas.
//!
//! Sprites batch by atlas, so putting everything in one texture is what turns a
//! thousand sprites into one draw call. This packs images at load time.
//!
//! The *offline* import cache — content hashing into `.import/`, `.meta`
//! settings, Aseprite tags — is M6 and lives in `dimetric-assets`. What is here
//! is only the part M3 needs: get pixels onto the GPU in a layout the batcher
//! can use.

use std::collections::BTreeMap;

use dimetric_scene::Color;

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

/// An image waiting to be packed.
pub struct Source {
    /// Name the scene refers to it by, without the `asset:` prefix.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA, row-major, 8 bits per channel.
    pub pixels: Vec<u8>,
}

/// One packed texture and the regions inside it.
pub struct Atlas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA pixel data.
    pub pixels: Vec<u8>,
    regions: BTreeMap<String, Region>,
}

/// Space left between packed images.
///
/// One pixel, so that linear filtering or a half-pixel sampling error cannot
/// pull a neighbour's colour into a sprite's edge. Nearest sampling does not
/// need it, but a project can turn nearest off.
const PADDING: u32 = 1;

impl Atlas {
    /// Pack sources into a single texture.
    ///
    /// Shelf packing: tallest first, laid left to right in rows. Not the
    /// tightest algorithm, but it is simple and — sorted by height and then by
    /// name — completely deterministic, which matters because a golden image
    /// of a differently-packed atlas is a different image.
    pub fn pack(mut sources: Vec<Source>, max_width: u32) -> Atlas {
        sources.sort_by(|a, b| b.height.cmp(&a.height).then(a.name.cmp(&b.name)));

        let width = max_width.max(
            sources
                .iter()
                .map(|s| s.width + PADDING * 2)
                .max()
                .unwrap_or(1),
        );

        // First pass: decide where everything goes and how tall the result is.
        let mut placements = Vec::with_capacity(sources.len());
        let (mut x, mut y, mut shelf_height) = (PADDING, PADDING, 0u32);
        for source in &sources {
            if x + source.width + PADDING > width && x > PADDING {
                x = PADDING;
                y += shelf_height + PADDING;
                shelf_height = 0;
            }
            placements.push((x, y));
            x += source.width + PADDING;
            shelf_height = shelf_height.max(source.height);
        }
        let height = (y + shelf_height + PADDING).max(1);

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut regions = BTreeMap::new();
        for (source, (ox, oy)) in sources.iter().zip(placements) {
            for row in 0..source.height {
                let from = (row * source.width * 4) as usize;
                let to = (((oy + row) * width + ox) * 4) as usize;
                let span = (source.width * 4) as usize;
                if from + span <= source.pixels.len() && to + span <= pixels.len() {
                    pixels[to..to + span].copy_from_slice(&source.pixels[from..from + span]);
                }
            }
            regions.insert(
                source.name.clone(),
                Region {
                    uv: [
                        ox as f32 / width as f32,
                        oy as f32 / height as f32,
                        (ox + source.width) as f32 / width as f32,
                        (oy + source.height) as f32 / height as f32,
                    ],
                    size: (source.width, source.height),
                },
            );
        }

        Atlas {
            width,
            height,
            pixels,
            regions,
        }
    }

    /// Look up a region by asset name.
    pub fn region(&self, name: &str) -> Option<Region> {
        self.regions.get(name).copied()
    }

    /// Every region, in name order.
    pub fn regions(&self) -> impl Iterator<Item = (&str, &Region)> {
        self.regions.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// How many images are packed.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// True when nothing is packed.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

/// Why an image could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The file could not be opened.
    #[error("cannot read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The file was not a PNG this build can decode.
    #[error("cannot decode {path}: {detail}")]
    Decode {
        /// Path that failed.
        path: String,
        /// What the decoder said.
        detail: String,
    },
}

/// Read a PNG into RGBA pixels.
///
/// Direct decoding, not the import pipeline: no caching, no `.meta`, no atlas
/// on disk. M6 replaces the call site, not this function's job.
pub fn load_png(path: &std::path::Path) -> Result<Source, ImageError> {
    let file = std::fs::File::open(path).map_err(|e| ImageError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().map_err(|e| ImageError::Decode {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| ImageError::Decode {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    buffer.truncate(info.buffer_size());

    let pixels =
        to_rgba(&buffer, info.color_type, info.bit_depth).ok_or_else(|| ImageError::Decode {
            path: path.display().to_string(),
            detail: format!(
                "unsupported {:?} at {:?} bits; convert the file to 8-bit RGBA",
                info.color_type, info.bit_depth
            ),
        })?;

    Ok(Source {
        name: String::new(),
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Expand the colour types worth supporting into RGBA.
fn to_rgba(data: &[u8], color: png::ColorType, depth: png::BitDepth) -> Option<Vec<u8>> {
    if depth != png::BitDepth::Eight {
        return None;
    }
    Some(match color {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    })
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
