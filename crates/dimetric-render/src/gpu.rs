//! The wgpu backend.
//!
//! Three passes, in order: sprites into a world texture at the project's
//! internal resolution, lights added into an accumulation buffer, then a
//! composite that multiplies one by the other and scales the result to
//! whatever it is being shown on.
//!
//! Windowed and headless rendering go through exactly this code and differ only
//! in what [`Target`] they are handed. That is deliberate and load-bearing: if
//! the two paths diverged, an agent's screenshot would stop being evidence
//! about what a person sees, and the golden-image tests would be checking
//! something nobody looks at.

use dimetric_core::Angle;
use dimetric_scene::Color;

use crate::atlas::Atlas;
use crate::batch::Blend;
use crate::extract::Frame;
use crate::settings::RenderSettings;

/// Format used for every texture the renderer owns.
///
/// One format everywhere so the windowed and headless paths cannot pick
/// different ones and produce different pixels.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Why the renderer could not start or draw.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter at all.
    #[error("no graphics adapter available: {0}")]
    NoAdapter(String),
    /// An adapter exists but a device could not be opened on it.
    #[error("could not open a graphics device: {0}")]
    NoDevice(String),
    /// Something went wrong while drawing or reading back.
    #[error("{0}")]
    Draw(String),
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    /// `xy` only; padded to a vec4 for alignment.
    pixel_scale: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeUniform {
    ambient: [f32; 4],
    /// `x` enables the light buffer; the rest is unused padding. See the note
    /// in `composite.wgsl` about vec3 alignment.
    lighting: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteInstance {
    center: [f32; 2],
    size: [f32; 2],
    rotation: [f32; 2],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightInstance {
    center: [f32; 2],
    radius: f32,
    energy: f32,
    color: [f32; 4],
    direction: [f32; 2],
    cone_cos: f32,
    _padding: f32,
}

/// Where a frame is being drawn.
pub struct Target<'a> {
    /// The view to composite into.
    pub view: &'a wgpu::TextureView,
    /// Its size in pixels.
    pub size: (u32, u32),
}

/// Device, pipelines and the buffers a frame needs.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,

    sprite_pipelines: [wgpu::RenderPipeline; 3],
    light_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,

    camera_buffer: wgpu::Buffer,
    sprite_bind_group: wgpu::BindGroup,
    light_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    composite_buffer: wgpu::Buffer,

    sprite_instances: wgpu::Buffer,
    sprite_capacity: usize,
    light_instances: wgpu::Buffer,
    light_capacity: usize,

    world: wgpu::Texture,
    light: wgpu::Texture,

    settings: RenderSettings,
    /// What the adapter reported, for diagnostics and for CI logs.
    pub adapter_info: wgpu::AdapterInfo,
}

impl Renderer {
    /// Build a renderer, blocking until the device is ready.
    pub fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        atlas: &Atlas,
        settings: RenderSettings,
    ) -> Result<Renderer, GpuError> {
        pollster::block_on(Renderer::new_async(instance, surface, atlas, settings))
    }

    /// Build a renderer.
    pub async fn new_async(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        atlas: &Atlas,
        settings: RenderSettings,
    ) -> Result<Renderer, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: surface,
                // A software adapter is accepted. Build runners have no GPU,
                // and a renderer that refuses to draw on one can only ever be
                // checked by hand.
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("dimetric device"),
                required_features: wgpu::Features::empty(),
                // The downlevel floor rather than the adapter's own limits, so
                // a scene that renders on a developer's machine also renders on
                // a modest laptop instead of failing there first.
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| GpuError::NoDevice(e.to_string()))?;

        let atlas_texture = upload_atlas(&device, &queue, atlas);
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas sampler"),
            // Nearest by default: this engine is for pixel art, and linear
            // filtering turns a crisp sprite into a smear at any scale that is
            // not exactly 1:1.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let composite_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite settings"),
            size: std::mem::size_of::<CompositeUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (world, light) = internal_targets(&device, settings.internal_resolution);

        let sprite_layout = sprite_bind_group_layout(&device);
        let sprite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite bindings"),
            layout: &sprite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let light_layout = camera_only_layout(&device);
        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light bindings"),
            layout: &light_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let composite_layout = composite_bind_group_layout(&device);
        let composite_bind_group = composite_bindings(
            &device,
            &composite_layout,
            &world,
            &light,
            &sampler,
            &composite_buffer,
        );

        let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sprite.wgsl").into()),
        });
        let light_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/light.wgsl").into()),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });

        // Blend state is baked into a pipeline, so a blend mode is a pipeline.
        // Three of them, selected per batch, which is why blend mode is part of
        // the batching key in the first place.
        let sprite_pipelines = [
            sprite_pipeline(&device, &sprite_shader, &sprite_layout, Blend::Alpha),
            sprite_pipeline(&device, &sprite_shader, &sprite_layout, Blend::Additive),
            sprite_pipeline(&device, &sprite_shader, &sprite_layout, Blend::Multiply),
        ];
        let light_pipeline = light_pipeline(&device, &light_shader, &light_layout);
        let composite_pipeline = composite_pipeline(&device, &composite_shader, &composite_layout);

        let sprite_capacity = 1024;
        let light_capacity = 64;
        Ok(Renderer {
            sprite_instances: instance_buffer::<SpriteInstance>(
                &device,
                sprite_capacity,
                "sprites",
            ),
            light_instances: instance_buffer::<LightInstance>(&device, light_capacity, "lights"),
            sprite_capacity,
            light_capacity,
            device,
            queue,
            sprite_pipelines,
            light_pipeline,
            composite_pipeline,
            camera_buffer,
            sprite_bind_group,
            light_bind_group,
            composite_bind_group,
            composite_buffer,
            world,
            light,
            settings,
            adapter_info,
        })
    }

    /// The device, for callers that own a surface.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue, for callers that present their own frames.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Current settings.
    pub fn settings(&self) -> RenderSettings {
        self.settings
    }

    /// Draw a frame into `target`.
    pub fn render(&mut self, frame: &Frame, target: &Target<'_>) -> Result<(), GpuError> {
        // The camera is bound to the *internal* resolution, not the output's.
        // Everything renders at the project's chosen size and the composite
        // scales it, which is what makes a pixel-art project look the same on
        // every monitor.
        let mut camera = frame.camera;
        camera.viewport = self.settings.internal_resolution;
        camera.pixel_snap = self.settings.pixel_snap;
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: camera.view_projection(),
                pixel_scale: {
                    let [x, y] = camera.pixel_scale();
                    [x, y, 0.0, 0.0]
                },
            }),
        );

        let lighting = self.settings.lighting_enabled() && !frame.lights.is_empty();
        self.queue.write_buffer(
            &self.composite_buffer,
            0,
            bytemuck::bytes_of(&CompositeUniform {
                ambient: to_linear(self.settings.ambient),
                lighting: [if lighting { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            }),
        );

        self.upload_sprites(frame);
        if lighting {
            self.upload_lights(frame);
        }

        let world_view = self
            .world
            .create_view(&wgpu::TextureViewDescriptor::default());
        let light_view = self
            .light
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        self.sprite_pass(&mut encoder, &world_view, frame);
        if lighting {
            self.light_pass(&mut encoder, &light_view, frame);
        }
        self.composite_pass(&mut encoder, target);

        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn sprite_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: &Frame,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.sprite_bind_group, &[]);
        pass.set_vertex_buffer(0, self.sprite_instances.slice(..));

        // One instanced draw per batch, in the order the sort decided.
        for batch in &frame.batches {
            let pipeline = match batch.blend {
                Blend::Alpha => &self.sprite_pipelines[0],
                Blend::Additive => &self.sprite_pipelines[1],
                Blend::Multiply => &self.sprite_pipelines[2],
            };
            pass.set_pipeline(pipeline);
            let start = batch.start as u32;
            pass.draw(0..4, start..start + batch.count as u32);
        }
    }

    fn light_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame: &Frame,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lights"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.light_pipeline);
        pass.set_bind_group(0, &self.light_bind_group, &[]);
        pass.set_vertex_buffer(0, self.light_instances.slice(..));
        pass.draw(0..4, 0..frame.lights.len() as u32);
    }

    fn composite_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &Target<'_>) {
        let (scale, offset_x, offset_y) = self.settings.placement(target.size);
        let (iw, ih) = self.settings.internal_resolution;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The border left by integer upscaling. Black rather than
                    // transparent, so a captured frame is a picture rather than
                    // a picture on an undefined background.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            offset_x,
            offset_y,
            iw as f32 * scale,
            ih as f32 * scale,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &self.composite_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn upload_sprites(&mut self, frame: &Frame) {
        let instances: Vec<SpriteInstance> = frame
            .sprites
            .iter()
            .map(|item| {
                // Sine and cosine from the engine's committed tables, not the
                // platform's: what is drawn has to match what the simulation
                // decided, and libm does not agree with itself across systems.
                let (sin, cos) = item.rotation.sin_cos();
                SpriteInstance {
                    center: item.pos.to_f32_pair().into(),
                    size: item.size.to_f32_pair().into(),
                    rotation: [cos.to_f32(), sin.to_f32()],
                    uv_min: [item.uv[0], item.uv[1]],
                    uv_max: [item.uv[2], item.uv[3]],
                    color: to_linear(Color::rgba(
                        item.modulate[0],
                        item.modulate[1],
                        item.modulate[2],
                        item.modulate[3],
                    )),
                }
            })
            .collect();
        if instances.len() > self.sprite_capacity {
            self.sprite_capacity = instances.len().next_power_of_two();
            self.sprite_instances =
                instance_buffer::<SpriteInstance>(&self.device, self.sprite_capacity, "sprites");
        }
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.sprite_instances, 0, bytemuck::cast_slice(&instances));
        }
    }

    fn upload_lights(&mut self, frame: &Frame) {
        let instances: Vec<LightInstance> = frame
            .lights
            .iter()
            .map(|light| {
                let (sin, cos) = light.direction.sin_cos();
                LightInstance {
                    center: light.pos.to_f32_pair().into(),
                    radius: light.radius.to_f32(),
                    energy: light.energy.to_f32(),
                    color: to_linear(light.color),
                    direction: [cos.to_f32(), sin.to_f32()],
                    // -1 accepts every direction, which is how a radial light
                    // shares the cone pipeline instead of needing its own.
                    cone_cos: match light.cone {
                        Some(angle) => half_angle_cosine(angle),
                        None => -1.0,
                    },
                    _padding: 0.0,
                }
            })
            .collect();
        if instances.len() > self.light_capacity {
            self.light_capacity = instances.len().next_power_of_two();
            self.light_instances =
                instance_buffer::<LightInstance>(&self.device, self.light_capacity, "lights");
        }
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.light_instances, 0, bytemuck::cast_slice(&instances));
        }
    }
}

/// The cosine of a cone's half-width.
fn half_angle_cosine(angle: Angle) -> f32 {
    // I3-exempt: render boundary.
    Angle::from_bam(angle.to_bam() / 2).cos().to_f32()
}

/// sRGB bytes to the linear floats a shader works in.
///
/// The asymmetry caught by the M0 spike: a colour handed to the GPU as a float
/// is linear and gets encoded on the way into an sRGB target, while texture
/// bytes written directly are stored verbatim. Converting here keeps a tint and
/// a texture pixel of the same colour looking the same.
fn to_linear(color: Color) -> [f32; 4] {
    // I3-exempt: render boundary.
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(color.r),
        channel(color.g),
        channel(color.b),
        color.a as f32 / 255.0,
    ]
}

fn instance_buffer<T>(device: &wgpu::Device, capacity: usize, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (std::mem::size_of::<T>() * capacity.max(1)) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn internal_targets(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::Texture) {
    let make = |label: &str| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    (make("world"), make("light"))
}

fn upload_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &Atlas) -> wgpu::Texture {
    let (width, height) = (atlas.width.max(1), atlas.height.max(1));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut pixels = atlas.pixels.clone();
    pixels.resize((width * height * 4) as usize, 0);
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
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn sprite_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sprite layout"),
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
    })
}

fn camera_only_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("camera layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn composite_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("composite layout"),
        entries: &[
            texture(0),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn composite_bindings(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    world: &wgpu::Texture,
    light: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    settings: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let light_view = light.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite bindings"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&world_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&light_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: settings.as_entire_binding(),
            },
        ],
    })
}

fn blend_state(blend: Blend) -> wgpu::BlendState {
    match blend {
        Blend::Alpha => wgpu::BlendState::ALPHA_BLENDING,
        Blend::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        Blend::Multiply => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        },
    }
}

fn sprite_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    blend: Blend,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite pipeline layout"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprites"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SpriteInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2, 1 => Float32x2, 2 => Float32x2,
                    3 => Float32x2, 4 => Float32x2, 5 => Float32x4
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(blend_state(blend)),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn light_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("light pipeline layout"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lights"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LightInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2, 1 => Float32, 2 => Float32,
                    3 => Float32x4, 4 => Float32x2, 5 => Float32
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                // Lights add. Two torches on one wall are brighter than one,
                // and neither occludes the other.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::OVER,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn composite_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("composite pipeline layout"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("composite"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
