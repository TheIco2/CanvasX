// openrender-runtime/src/gpu/texture.rs
//
// Texture management — loading images into GPU textures,
// managing a texture atlas for small UI assets.

use anyhow::Result;
use std::collections::HashMap;

/// Manages GPU textures for the renderer.
pub struct TextureManager {
    /// Loaded textures by asset index.
    textures: HashMap<u32, GpuTexture>,
    /// Default 1x1 white texture (used when no texture is bound).
    pub default_texture: GpuTexture,
}

/// A loaded GPU texture.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

impl TextureManager {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // Create a 1x1 transparent default texture.
        let default_texture = Self::create_texture(device, queue, 1, 1, &[0, 0, 0, 0]);
        Self {
            textures: HashMap::new(),
            default_texture,
        }
    }

    /// Load an image from raw RGBA bytes into a GPU texture.
    pub fn load_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_index: u32,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> &GpuTexture {
        let tex = Self::create_texture(device, queue, width, height, rgba_data);
        self.textures.insert(asset_index, tex);
        self.textures.get(&asset_index).unwrap()
    }

    /// Load from a compressed image buffer (PNG/JPEG/WebP).
    pub fn load_image_from_bytes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_index: u32,
        data: &[u8],
    ) -> Result<&GpuTexture> {
        let img = image::load_from_memory(data)?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let tex = Self::create_texture(device, queue, w, h, &rgba);
        self.textures.insert(asset_index, tex);
        Ok(self.textures.get(&asset_index).unwrap())
    }

    /// Update an existing texture with new pixel data (or create if missing).
    /// Used for streaming canvas pixel data each frame.
    pub fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_index: u32,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> bool {
        // If existing texture has the same size, just write new data.
        if let Some(tex) = self.textures.get(&asset_index) {
            if tex.size == (width, height) {
                let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    rgba_data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * width),
                        rows_per_image: Some(height),
                    },
                    size,
                );
                return false; // no new texture created, bind group still valid
            }
        }

        // Size changed or texture missing — create new.
        let tex = Self::create_texture(device, queue, width, height, rgba_data);
        self.textures.insert(asset_index, tex);
        true // new texture created, bind group needs refresh
    }

    /// Get a loaded texture (or default).
    pub fn get(&self, asset_index: u32) -> &GpuTexture {
        self.textures.get(&asset_index).unwrap_or(&self.default_texture)
    }

    /// Get the texture view for binding.
    pub fn get_view(&self, asset_index: u32) -> &wgpu::TextureView {
        &self.get(asset_index).view
    }

    fn create_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> GpuTexture {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Use Rgba8Unorm (NOT Srgb) — all colors stay in sRGB space
            // throughout the pipeline to match browser compositing. No automatic
            // sRGB↔linear conversion should happen on read or write.
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        GpuTexture {
            texture,
            view,
            size: (width, height),
        }
    }

    /// Create a sampler for UI textures (linear filtering, clamp-to-edge).
    pub fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        })
    }
}
