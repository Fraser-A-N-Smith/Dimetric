//! Everything that talks to wgpu.
//!
//! Written once and shared by both modes, because the whole question M0 exists
//! to answer is whether the windowed and headless paths can be the same code.
//! If they diverge, an agent's screenshot stops being evidence about what a
//! human sees.

use dimetric_core::Vec2Fx;
use dimetric_render::{Camera, Projection};
use wgpu::util::DeviceExt as _;

/// Size of the checkerboard the quad samples.
///
/// Generated rather than loaded: M0 is about the graphics stack, and pulling in
/// an image decoder would only add a second thing that can fail.
const TEXTURE_SIZE: u32 = 16;

/// The texture format used everywhere.
///
/// `Rgba8UnormSrgb` is the one format guaranteed present on every backend, and
/// picking it explicitly avoids the windowed path silently choosing a different
/// one from the headless path and producing different pixels.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// A device, a pipeline and the buffers one quad needs.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// What the adapter reported, for the spike's report.
    pub adapter_info: wgpu::AdapterInfo,
}

impl Renderer {
    /// Build everything, optionally against a surface.
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Renderer, String> {
        let adapter = instance
            // Struct-update syntax against Default, rather than naming every
            // field: wgpu adds fields between releases, and a spike that has to
            // be edited for each one is a spike nobody re-runs.
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: surface,
                // Accepting a software adapter is what makes this runnable in
                // CI at all. A machine with no GPU is the normal case for a
                // build runner, and refusing to draw there would mean the
                // renderer is only ever tested by hand.
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no graphics adapter available: {e}"))?;

        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("m0 device"),
                required_features: wgpu::Features::empty(),
                // Downlevel defaults rather than the full set: it is the floor
                // every target supports, and asking for more here would hide a
                // limit problem until someone ran the engine on a laptop.
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("could not open the device: {e}"))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("m0 shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // A quad 64 world units across, centred on the origin.
        let half = 32.0f32;
        let vertices: [Vertex; 4] = [
            Vertex {
                position: [-half, -half],
                uv: [0.0, 0.0],
            },
            Vertex {
                position: [half, -half],
                uv: [1.0, 0.0],
            },
            Vertex {
                position: [half, half],
                uv: [1.0, 1.0],
            },
            Vertex {
                position: [-half, half],
                uv: [0.0, 1.0],
            },
        ];
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("m0 vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("m0 indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("m0 camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture = checkerboard(&device, &queue);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("m0 sampler"),
            // Nearest, because this engine is for pixel art and linear
            // filtering would blur the checkerboard into evidence of nothing.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("m0 bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("m0 bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("m0 pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("m0 pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Renderer {
            device,
            queue,
            pipeline,
            bind_group,
            camera_buffer,
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
            adapter_info,
        })
    }

    /// Push the camera matrix for this frame.
    pub fn set_camera(&self, camera: &Camera) {
        let uniform = CameraUniform {
            view_proj: view_projection(camera),
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Record one frame into `target`.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("m0 pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Build a view-projection matrix from the engine's camera.
///
/// Deliberately goes through [`Projection::matrix`] rather than reimplementing
/// the shear, so that what the GPU draws is the same transform the engine's
/// sorting and picking use. A spike that reimplemented it would prove nothing.
fn view_projection(camera: &Camera) -> [[f32; 4]; 4] {
    let (width, height) = (camera.viewport.0 as f32, camera.viewport.1 as f32);
    let [a, b, c, d] = camera.projection.matrix();
    let (cx, cy) = camera.projection.to_screen(camera.center);
    let zoom = camera.zoom;

    // Orthographic, y down, origin at the centre of the viewport.
    let sx = 2.0 * zoom / width;
    let sy = -2.0 * zoom / height;

    // WGSL matrices are column-major, and `Projection::matrix` is row-major, so
    // the columns are (a, c) and (b, d) — not (a, b) and (c, d).
    //
    // Getting this backwards was the spike's first real find. The transpose of
    // a 2:1 shear is another shear of the same determinant, so the picture
    // stayed plausible: a diamond of the right area, just the wrong diamond.
    // Every unit test still passed, because they exercise `to_screen` and
    // nothing had ever checked that `matrix` agreed with it.
    [
        [a * sx, c * sy, 0.0, 0.0],
        [b * sx, d * sy, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-cx * sx, -cy * sy, 0.0, 1.0],
    ]
}

/// A magenta-and-slate checkerboard, so a wrong sampler or a wrong UV is
/// obvious at a glance rather than plausible.
fn checkerboard(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let mut pixels = Vec::with_capacity((TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize);
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let light = (x / 4 + y / 4) % 2 == 0;
            pixels.extend_from_slice(if light {
                &[0xE0, 0x3F, 0xB0, 0xFF]
            } else {
                &[0x28, 0x2C, 0x3A, 0xFF]
            });
        }
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("m0 checkerboard"),
        size: wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(TEXTURE_SIZE * 4),
            rows_per_image: Some(TEXTURE_SIZE),
        },
        wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// A camera looking at the quad, used by both modes.
pub fn spike_camera(viewport: (u32, u32), projection: Projection) -> Camera {
    let mut camera = Camera::new(viewport);
    camera.projection = projection;
    camera.center = Vec2Fx::ZERO;
    camera.zoom = 2.0;
    camera
}
