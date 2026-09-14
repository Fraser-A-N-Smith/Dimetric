//! Decoded pixels, and the decoders that produce them.
//!
//! Everything above this works in RGBA, row-major, eight bits a channel. The
//! variety in the source formats — PNG colour types, Aseprite's indexed and
//! grayscale modes — is flattened here, once, at import, rather than being
//! carried around as a case the renderer has to know about.

/// Decoded pixels.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    /// Name the scene refers to it by, without the `asset:` prefix.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA, row-major, 8 bits per channel.
    pub pixels: Vec<u8>,
}

impl Image {
    /// An image of a given size, fully transparent.
    pub fn blank(name: impl Into<String>, width: u32, height: u32) -> Image {
        Image {
            name: name.into(),
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Copy `src` into this image with its top-left at `(x, y)`.
    ///
    /// Straight replacement, not a blend: the callers are laying frames out
    /// side by side on a sheet, where nothing overlaps by construction.
    pub fn blit(&mut self, src: &Image, x: u32, y: u32) {
        let span = (src.width as usize) * 4;
        for row in 0..src.height {
            let from = (row as usize) * span;
            let to = ((y + row) as usize * self.width as usize + x as usize) * 4;
            if from + span <= src.pixels.len() && to + span <= self.pixels.len() {
                self.pixels[to..to + span].copy_from_slice(&src.pixels[from..from + span]);
            }
        }
    }

    /// The rectangle `(x, y, width, height)` as an image of its own.
    pub fn crop(&self, name: impl Into<String>, x: u32, y: u32, width: u32, height: u32) -> Image {
        let mut out = Image::blank(name, width, height);
        for row in 0..height.min(self.height.saturating_sub(y)) {
            let span = (width.min(self.width.saturating_sub(x)) as usize) * 4;
            let from = ((y + row) as usize * self.width as usize + x as usize) * 4;
            let to = (row as usize) * (width as usize) * 4;
            out.pixels[to..to + span].copy_from_slice(&self.pixels[from..from + span]);
        }
        out
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
    /// The file was not something this build can decode.
    #[error("cannot decode {path}: {detail}")]
    Decode {
        /// Path that failed.
        path: String,
        /// What the decoder said.
        detail: String,
    },
}

impl ImageError {
    /// An I/O failure against a path.
    pub fn io(path: &std::path::Path, source: std::io::Error) -> ImageError {
        ImageError::Io {
            path: path.display().to_string(),
            source,
        }
    }

    /// A decode failure against a path.
    pub fn decode(path: &std::path::Path, detail: impl std::fmt::Display) -> ImageError {
        ImageError::Decode {
            path: path.display().to_string(),
            detail: detail.to_string(),
        }
    }
}

/// Read a PNG into RGBA pixels.
pub fn decode_png(path: &std::path::Path) -> Result<Image, ImageError> {
    let file = std::fs::File::open(path).map_err(|e| ImageError::io(path, e))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|e| ImageError::decode(path, e))?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| ImageError::decode(path, e))?;
    buffer.truncate(info.buffer_size());

    let palette = reader.info().palette.clone();
    let transparency = reader.info().trns.clone();
    let pixels = to_rgba(
        &buffer,
        info.color_type,
        info.bit_depth,
        palette.as_deref(),
        transparency.as_deref(),
    )
    .ok_or_else(|| {
        ImageError::decode(
            path,
            format!(
                "unsupported {:?} at {:?} bits; convert the file to 8-bit RGBA",
                info.color_type, info.bit_depth
            ),
        )
    })?;

    Ok(Image {
        name: String::new(),
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Expand the colour types worth supporting into RGBA.
fn to_rgba(
    data: &[u8],
    color: png::ColorType,
    depth: png::BitDepth,
    palette: Option<&[u8]>,
    transparency: Option<&[u8]>,
) -> Option<Vec<u8>> {
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
        // Indexed PNGs are what most pixel-art tools export by default, so
        // refusing them would refuse the common case.
        png::ColorType::Indexed => {
            let palette = palette?;
            let mut out = Vec::with_capacity(data.len() * 4);
            for index in data {
                let i = *index as usize;
                let rgb = palette.get(i * 3..i * 3 + 3)?;
                let alpha = transparency.and_then(|t| t.get(i)).copied().unwrap_or(255);
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
            }
            out
        }
    })
}

/// Write RGBA pixels out as an 8-bit PNG.
pub fn encode_png(
    path: &std::path::Path,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::other(e.to_string()))
}
