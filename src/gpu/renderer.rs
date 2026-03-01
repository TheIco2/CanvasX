// canvasx-runtime/src/gpu/renderer.rs
//
// The main renderer — takes a scene graph's paint output (list of UiInstance)
// and submits draw calls to the GPU. Also manages text rendering via glyphon.

use anyhow::Result;
use crate::gpu::context::GpuContext;
use crate::gpu::pipeline::{UiPipelines, GlobalUniforms};
use crate::gpu::texture::TextureManager;
use crate::gpu::vertex::{UiInstance, QUAD_VERTICES, QUAD_INDICES};
use crate::cxrd::value::Color;

/// The main renderer for the CanvasX Runtime.
pub struct Renderer {
    pub pipelines: UiPipelines,
    pub texture_manager: TextureManager,
    pub sampler: wgpu::Sampler,

    // Buffers
    quad_vbo: wgpu::Buffer,
    quad_ibo: wgpu::Buffer,
    globals_ubo: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    default_texture_bg: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,

    // Text renderer
    pub text_renderer: glyphon::TextRenderer,
    pub font_system: glyphon::FontSystem,
    pub swash_cache: glyphon::SwashCache,
    pub text_atlas: glyphon::TextAtlas,
    pub text_cache: glyphon::Cache,
    pub text_viewport: glyphon::Viewport,

    // State
    frame_time: f32,
    scale_factor: f32,
}

impl Renderer {
    pub fn new(ctx: &GpuContext) -> Result<Self> {
        let device = &ctx.device;
        let queue = &ctx.queue;

        let pipelines = UiPipelines::new(device, ctx.surface_format);
        let texture_manager = TextureManager::new(device, queue);
        let sampler = TextureManager::create_sampler(device);

        // Quad vertex + index buffers (shared for all instanced draws).
        let quad_vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad_vbo"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad_ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad_ibo"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Global uniforms buffer.
        let globals = GlobalUniforms {
            viewport: [ctx.size.0 as f32, ctx.size.1 as f32, 1.0 / ctx.size.0 as f32, 1.0 / ctx.size.1 as f32],
            time: 0.0,
            scale: 1.0,
            _pad: [0.0; 2],
        };
        let globals_ubo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals_ubo"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bg"),
            layout: &pipelines.globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_ubo.as_entire_binding(),
            }],
        });

        // Default texture bind group (1x1 white).
        let default_texture_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("default_texture_bg"),
            layout: &pipelines.texture_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_manager.default_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        // Instance buffer (will grow as needed).
        let initial_capacity = 1024;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: (initial_capacity * std::mem::size_of::<UiInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Text rendering (glyphon) ---
        let font_system = glyphon::FontSystem::new();
        let swash_cache = glyphon::SwashCache::new();
        let text_cache = glyphon::Cache::new(device);
        let text_viewport = glyphon::Viewport::new(device, &text_cache);
        let mut text_atlas = glyphon::TextAtlas::new(device, queue, &text_cache, ctx.surface_format);
        let text_renderer = glyphon::TextRenderer::new(
            &mut text_atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        Ok(Self {
            pipelines,
            texture_manager,
            sampler,
            quad_vbo,
            quad_ibo,
            globals_ubo,
            globals_bg,
            default_texture_bg,
            instance_buffer,
            instance_capacity: initial_capacity,
            text_renderer,
            font_system,
            swash_cache,
            text_atlas,
            text_cache,
            text_viewport,
            frame_time: 0.0,
            scale_factor: 1.0,
        })
    }

    /// Begin a new frame — update timestamp and viewport uniforms.
    ///
    /// When `scale_factor > 1.0` the layout operates in virtual pixels
    /// (physical / scale) so the shader's viewport is also virtual.
    /// The GPU surface stays at physical resolution for crisp rendering.
    pub fn begin_frame(&mut self, ctx: &GpuContext, dt: f32, scale_factor: f32) {
        self.frame_time += dt;
        self.scale_factor = scale_factor;

        let vp_w = ctx.size.0 as f32 / scale_factor;
        let vp_h = ctx.size.1 as f32 / scale_factor;

        let globals = GlobalUniforms {
            viewport: [
                vp_w,
                vp_h,
                1.0 / vp_w,
                1.0 / vp_h,
            ],
            time: self.frame_time,
            scale: scale_factor,
            _pad: [0.0; 2],
        };
        ctx.queue.write_buffer(&self.globals_ubo, 0, bytemuck::bytes_of(&globals));
    }

    /// Render a frame: clear + draw all UI instances + text.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        instances: &[UiInstance],
        text_areas: Vec<glyphon::TextArea<'_>>,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = ctx.current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Upload instances
        self.upload_instances(&ctx.device, &ctx.queue, instances);

        // Update text viewport (use virtual dims matching our UI viewport).
        let vp_w = (ctx.size.0 as f32 / self.scale_factor) as u32;
        let vp_h = (ctx.size.1 as f32 / self.scale_factor) as u32;
        self.text_viewport.update(
            &ctx.queue,
            glyphon::Resolution {
                width: vp_w.max(1),
                height: vp_h.max(1),
            },
        );

        // Prepare text
        self.text_renderer
            .prepare(
                &ctx.device,
                &ctx.queue,
                &mut self.font_system,
                &mut self.text_atlas,
                &self.text_viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .ok();

        let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color.r as f64,
                            g: clear_color.g as f64,
                            b: clear_color.b as f64,
                            a: clear_color.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            if !instances.is_empty() {
                pass.set_pipeline(&self.pipelines.box_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(1, &self.default_texture_bg, &[]);
                pass.set_vertex_buffer(0, self.quad_vbo.slice(..));
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                pass.set_index_buffer(self.quad_ibo.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..instances.len() as u32);
            }

            // Draw text on top
            self.text_renderer.render(&self.text_atlas, &self.text_viewport, &mut pass).ok();
        }

        ctx.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.text_atlas.trim();

        Ok(())
    }

    fn upload_instances(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[UiInstance]) {
        let needed = instances.len();
        if needed == 0 {
            return;
        }

        // Grow buffer if needed.
        if needed > self.instance_capacity {
            self.instance_capacity = needed.next_power_of_two();
            self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instance_buffer"),
                size: (self.instance_capacity * std::mem::size_of::<UiInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    /// Load all assets from a CXRD document into GPU textures.
    pub fn load_assets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, assets: &crate::cxrd::asset::AssetBundle) {
        for (i, img) in assets.images.iter().enumerate() {
            if let Err(e) = self.texture_manager.load_image_from_bytes(device, queue, i as u32, &img.data) {
                log::warn!("Failed to load image asset '{}': {}", img.name, e);
            }
        }

        // Load bundled fonts into the font system.
        for font in &assets.fonts {
            self.font_system.db_mut().load_font_data(font.data.clone());
        }
    }
}

// wgpu::util::BufferInitDescriptor helper
use wgpu::util::DeviceExt;
