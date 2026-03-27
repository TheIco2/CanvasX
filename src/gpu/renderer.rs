// canvasx-runtime/src/gpu/renderer.rs
//
// The main renderer — takes a scene graph's paint output (list of UiInstance)
// and submits draw calls to the GPU. Also manages text rendering via glyphon.

use std::time::Instant;

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

    /// Per-texture bind groups for canvas / image textures.
    texture_bind_groups: std::collections::HashMap<u32, wgpu::BindGroup>,

    // Text renderer
    pub text_renderer: glyphon::TextRenderer,
    pub font_system: glyphon::FontSystem,
    pub swash_cache: glyphon::SwashCache,
    pub text_atlas: glyphon::TextAtlas,
    pub text_cache: glyphon::Cache,
    pub text_viewport: glyphon::Viewport,

    // Overlay text renderer (separate atlas to avoid eviction issues)
    overlay_text_renderer: glyphon::TextRenderer,
    overlay_text_atlas: glyphon::TextAtlas,
    overlay_text_viewport: glyphon::Viewport,

    // State
    frame_time: f32,
    scale_factor: f32,
    frame_count: u32,
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
        let mut text_atlas = glyphon::TextAtlas::with_color_mode(device, queue, &text_cache, ctx.surface_format, glyphon::ColorMode::Web);
        let text_renderer = glyphon::TextRenderer::new(
            &mut text_atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        // Overlay text (separate atlas so prepare() doesn't evict scene glyphs).
        let overlay_text_viewport = glyphon::Viewport::new(device, &text_cache);
        let mut overlay_text_atlas = glyphon::TextAtlas::with_color_mode(device, queue, &text_cache, ctx.surface_format, glyphon::ColorMode::Web);
        let overlay_text_renderer = glyphon::TextRenderer::new(
            &mut overlay_text_atlas,
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
            texture_bind_groups: std::collections::HashMap::new(),
            text_renderer,
            font_system,
            swash_cache,
            text_atlas,
            text_cache,
            text_viewport,
            overlay_text_renderer,
            overlay_text_atlas,
            overlay_text_viewport,
            frame_time: 0.0,
            scale_factor: 1.0,
            frame_count: 0,
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
    ///
    /// When `overlay_instances` and `overlay_text` are provided, scene text is drawn
    /// first, then overlay box instances (e.g. context menu background), then overlay
    /// text. This ensures overlay backgrounds occlude scene text.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        instances: &[UiInstance],
        text_areas: Vec<glyphon::TextArea<'_>>,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        self.render_layered(ctx, instances, text_areas, &[], Vec::new(), clear_color)
    }

    /// Layered render: scene instances + scene text, then overlay instances + overlay text.
    pub fn render_layered(
        &mut self,
        ctx: &GpuContext,
        instances: &[UiInstance],
        scene_text: Vec<glyphon::TextArea<'_>>,
        overlay_instances: &[UiInstance],
        overlay_text: Vec<glyphon::TextArea<'_>>,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        let acq_start = Instant::now();
        let output = ctx.current_texture()?;
        let acq_ms = acq_start.elapsed().as_secs_f64() * 1000.0;
        if acq_ms > 5.0 {
            log::debug!("[GPU] get_current_texture blocked for {:.2}ms", acq_ms);
        }
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Combine scene + overlay instances into one buffer; track the split.
        let scene_count = instances.len();
        let has_overlay = !overlay_instances.is_empty() || !overlay_text.is_empty();
        let has_overlay_text = !overlay_text.is_empty();
        let all_instances: Vec<UiInstance> = if has_overlay {
            let mut combined = Vec::with_capacity(instances.len() + overlay_instances.len());
            combined.extend_from_slice(instances);
            combined.extend_from_slice(overlay_instances);
            combined
        } else {
            Vec::new() // unused when no overlay
        };
        let upload_instances = if has_overlay { &all_instances[..] } else { instances };
        self.upload_instances(&ctx.device, &ctx.queue, upload_instances);

        // Update text viewport.
        let vp_w = (ctx.size.0 as f32 / self.scale_factor) as u32;
        let vp_h = (ctx.size.1 as f32 / self.scale_factor) as u32;
        let resolution = glyphon::Resolution {
            width: vp_w.max(1),
            height: vp_h.max(1),
        };
        self.text_viewport.update(&ctx.queue, resolution);

        // Prepare scene text.
        self.text_renderer
            .prepare(
                &ctx.device,
                &ctx.queue,
                &mut self.font_system,
                &mut self.text_atlas,
                &self.text_viewport,
                scene_text,
                &mut self.swash_cache,
            )
            .ok();

        // Prepare overlay text using a separate renderer + atlas (avoids evicting scene glyphs).
        if has_overlay_text {
            self.overlay_text_viewport.update(&ctx.queue, resolution);
            self.overlay_text_renderer
                .prepare(
                    &ctx.device,
                    &ctx.queue,
                    &mut self.font_system,
                    &mut self.overlay_text_atlas,
                    &self.overlay_text_viewport,
                    overlay_text,
                    &mut self.swash_cache,
                )
                .ok();
        }

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

            let total = upload_instances.len();
            if total > 0 {
                pass.set_pipeline(&self.pipelines.box_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_vertex_buffer(0, self.quad_vbo.slice(..));
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                pass.set_index_buffer(self.quad_ibo.slice(..), wgpu::IndexFormat::Uint16);

                // Draw scene instances (0..scene_count).
                let draw_end = if has_overlay { scene_count } else { total };
                if draw_end > 0 {
                    self.draw_instance_batches(&mut pass, upload_instances, 0, draw_end);
                }
            }

            // Draw scene text.
            self.text_renderer.render(&self.text_atlas, &self.text_viewport, &mut pass).ok();

            // Draw overlay instances on top of scene text, then overlay text.
            if has_overlay && scene_count < total {
                pass.set_pipeline(&self.pipelines.box_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_vertex_buffer(0, self.quad_vbo.slice(..));
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                pass.set_index_buffer(self.quad_ibo.slice(..), wgpu::IndexFormat::Uint16);
                self.draw_instance_batches(&mut pass, upload_instances, scene_count, total);
            }
        }

        // Second text pass for overlay text (if any) using the separate overlay renderer.
        if has_overlay_text {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay_text_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            self.overlay_text_renderer.render(&self.overlay_text_atlas, &self.overlay_text_viewport, &mut pass).ok();
        }

        ctx.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        // Trim text atlas every 64 frames instead of every frame
        self.frame_count = self.frame_count.wrapping_add(1);
        if self.frame_count & 0x3F == 0 {
            self.text_atlas.trim();
            self.overlay_text_atlas.trim();
        }

        Ok(())
    }

    /// Draw instance batches in [start..end) range, grouped by texture_index.
    fn draw_instance_batches<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        instances: &[UiInstance],
        start: usize,
        end: usize,
    ) {
        if start >= end {
            return;
        }
        let mut batch_start = start as u32;
        let mut current_tex_idx = instances[start].texture_index;

        for i in start..=end {
            let tex_idx = if i < end { instances[i].texture_index } else { i32::MIN };
            if tex_idx != current_tex_idx || i == end {
                let batch_end = i as u32;
                let bg = if current_tex_idx >= 0 {
                    self.texture_bind_groups.get(&(current_tex_idx as u32))
                        .unwrap_or(&self.default_texture_bg)
                } else {
                    &self.default_texture_bg
                };
                pass.set_bind_group(1, bg, &[]);
                pass.draw_indexed(0..6, 0, batch_start..batch_end);
                batch_start = batch_end;
                current_tex_idx = tex_idx;
            }
        }
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

    /// Upload canvas pixel data and create/update the texture bind group.
    /// `asset_index` is the texture slot (use high indices like 10000+ for canvases).
    pub fn upload_canvas_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_index: u32,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) {
        let recreated = self.texture_manager.update_texture(device, queue, asset_index, width, height, rgba_data);

        // Create or refresh bind group if texture was (re-)created.
        if recreated || !self.texture_bind_groups.contains_key(&asset_index) {
            let view = self.texture_manager.get_view(asset_index);
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("canvas_texture_bg"),
                layout: &self.pipelines.texture_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.texture_bind_groups.insert(asset_index, bg);
        }
    }
}

// wgpu::util::BufferInitDescriptor helper
use wgpu::util::DeviceExt;
