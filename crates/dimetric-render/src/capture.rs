//! Offscreen rendering and readback.
//!
//! The same [`Renderer`] and the same passes as a window; only the target
//! differs. That equivalence is the whole point of the mode: a frame an agent
//! captures is the frame a person would have seen, so a screenshot is evidence
//! rather than an approximation.

use crate::extract::Frame;
use crate::gpu::{GpuError, Renderer, Target, FORMAT};

/// An offscreen target and the buffer its pixels are read back through.
pub struct Capture {
    texture: wgpu::Texture,
    staging: wgpu::Buffer,
    size: (u32, u32),
    /// Bytes per row in the staging buffer, rounded up to the copy alignment.
    padded_row: u32,
}

impl Capture {
    /// Allocate a capture target.
    pub fn new(renderer: &Renderer, size: (u32, u32)) -> Capture {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let device = renderer.device();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Copying a texture into a buffer requires every row to begin on a
        // 256-byte boundary. Forgetting this produces an image that is sheared,
        // and only at some widths, which makes it a memorable afternoon.
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Capture {
            texture,
            staging,
            size: (width, height),
            padded_row,
        }
    }

    /// The captured size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Render a frame and read it back as RGBA bytes.
    pub fn render(&self, renderer: &mut Renderer, frame: &Frame) -> Result<Vec<u8>, GpuError> {
        let view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render(
            frame,
            &Target {
                view: &view,
                size: self.size,
            },
        )?;

        let (width, height) = self.size;
        let mut encoder =
            renderer
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("capture readback"),
                });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        renderer.queue().submit([encoder.finish()]);

        let slice = self.staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        renderer
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Draw(format!("waiting for the GPU: {e}")))?;
        receiver
            .recv()
            .map_err(|e| GpuError::Draw(format!("readback never completed: {e}")))?
            .map_err(|e| GpuError::Draw(format!("could not map the readback buffer: {e}")))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| GpuError::Draw(format!("could not read the mapped buffer: {e}")))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * self.padded_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        self.staging.unmap();
        Ok(pixels)
    }
}

/// Write RGBA bytes out as a PNG.
pub fn write_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), GpuError> {
    dimetric_assets::encode_png(path, pixels, width, height)
        .map_err(|e| GpuError::Draw(format!("writing {}: {e}", path.display())))
}

/// Read a PNG back as RGBA bytes, for comparing against a golden image.
pub fn read_png(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), GpuError> {
    let source = crate::atlas::load_png(path).map_err(|e| GpuError::Draw(e.to_string()))?;
    Ok((source.width, source.height, source.pixels))
}

/// An instance suitable for headless rendering.
///
/// No display handle, which is exactly why it works on a build runner. Reads
/// `WGPU_BACKEND`, so a specific backend can be forced when comparing two.
pub fn headless_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}
