//! Offscreen rendering and readback.
//!
//! This is the mode CI runs, and the one that matters most. A renderer that can
//! only draw into a window cannot be checked by a build machine or looked at by
//! an agent, and "it looked fine on my laptop" is not a test.

use dimetric_render::Projection;

use crate::gpu::{spike_camera, Renderer, FORMAT};

/// Render one frame offscreen and return it as RGBA bytes.
pub fn render(width: u32, height: u32, projection: Projection) -> Result<Vec<u8>, String> {
    pollster::block_on(render_async(width, height, projection))
}

async fn render_async(width: u32, height: u32, projection: Projection) -> Result<Vec<u8>, String> {
    // No display handle: this path never touches a window system, which is the
    // whole reason it works on a build runner.
    //
    // `_from_env` so that `WGPU_BACKEND=gl` works. Being able to force a
    // backend is how you find out whether two of them agree, which is the
    // question the golden-image CI job depends on.
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let renderer = Renderer::new(&instance, None).await?;
    eprintln!(
        "m0: {} via {:?} ({:?})",
        renderer.adapter_info.name,
        renderer.adapter_info.backend,
        renderer.adapter_info.device_type
    );

    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("m0 offscreen target"),
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
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // Copying a texture to a buffer requires each row to start on a 256-byte
    // boundary. Forgetting this is the classic first wgpu readback bug: the
    // image comes back sheared, and only at some widths.
    let unpadded = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("m0 readback"),
        size: (padded * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    renderer.set_camera(&spike_camera((width, height), projection));

    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("m0 encoder"),
        });
    renderer.draw(&mut encoder, &view);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit([encoder.finish()]);

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    renderer
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("waiting for the GPU: {e}"))?;
    receiver
        .recv()
        .map_err(|e| format!("readback never completed: {e}"))?
        .map_err(|e| format!("could not map the readback buffer: {e}"))?;

    // Drop the padding the copy required.
    let mapped = slice
        .get_mapped_range()
        .map_err(|e| format!("could not read the mapped buffer: {e}"))?;
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    staging.unmap();
    Ok(pixels)
}

/// Write RGBA bytes out as a PNG.
pub fn write_png(path: &str, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("creating {path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|e| format!("writing {path}: {e}"))?
        .write_image_data(pixels)
        .map_err(|e| format!("writing {path}: {e}"))?;
    Ok(())
}
