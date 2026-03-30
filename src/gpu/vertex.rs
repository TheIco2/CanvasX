// openrender-runtime/src/gpu/vertex.rs
//
// Vertex and per-instance data structures for the GPU renderer.
// We use instanced rendering: one quad per UI element with per-instance
// data describing the element's rect, colors, border, etc.  The fragment
// shader evaluates SDF rounded-rectangles for pixel-perfect anti-aliasing.

use bytemuck::{Pod, Zeroable};

/// Per-vertex data for a unit quad (only 4 vertices, reused for all elements).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct QuadVertex {
    /// Position of this quad corner (0,0)–(1,1).
    pub position: [f32; 2],
    /// UV coordinate.
    pub uv: [f32; 2],
}

/// The 4 vertices of a unit quad.
pub const QUAD_VERTICES: [QuadVertex; 4] = [
    QuadVertex { position: [0.0, 0.0], uv: [0.0, 0.0] }, // top-left
    QuadVertex { position: [1.0, 0.0], uv: [1.0, 0.0] }, // top-right
    QuadVertex { position: [0.0, 1.0], uv: [0.0, 1.0] }, // bottom-left
    QuadVertex { position: [1.0, 1.0], uv: [1.0, 1.0] }, // bottom-right
];

/// Index buffer for the quad (two triangles).
pub const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// Per-instance data for a single UI element.
/// The fragment shader uses this to evaluate the SDF shape.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct UiInstance {
    /// Element rect in pixels: [x, y, width, height].
    pub rect: [f32; 4],

    /// Background color (RGBA, premultiplied alpha).
    pub bg_color: [f32; 4],

    /// Border color (RGBA).
    pub border_color: [f32; 4],

    /// Border widths: [top, right, bottom, left].
    pub border_width: [f32; 4],

    /// Corner radii: [top-left, top-right, bottom-right, bottom-left].
    pub border_radius: [f32; 4],

    /// Clip rect: [x, y, width, height] — for overflow:hidden masking.
    pub clip_rect: [f32; 4],

    /// Texture layer index (-1 = no texture; >=0 = index into texture array).
    pub texture_index: i32,

    /// Opacity (0.0–1.0).
    pub opacity: f32,

    /// Extra flags packed as bits:
    ///   bit 0: has_background
    ///   bit 1: has_border
    ///   bit 2: has_texture
    ///   bit 3: has_clip
    pub flags: u32,

    /// Padding for alignment.
    pub _pad: u32,
}

impl UiInstance {
    pub const FLAG_HAS_BACKGROUND: u32 = 1 << 0;
    pub const FLAG_HAS_BORDER: u32     = 1 << 1;
    pub const FLAG_HAS_TEXTURE: u32    = 1 << 2;
    pub const FLAG_HAS_CLIP: u32       = 1 << 3;
}

impl QuadVertex {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                // uv
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
            ],
        }
    }
}

impl UiInstance {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UiInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // rect
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 2,
                },
                // bg_color
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 3,
                },
                // border_color
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 4,
                },
                // border_width
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 5,
                },
                // border_radius
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 6,
                },
                // clip_rect
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 7,
                },
                // texture_index (as sint)
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Sint32,
                    offset: 96,
                    shader_location: 8,
                },
                // opacity
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 100,
                    shader_location: 9,
                },
                // flags
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 104,
                    shader_location: 10,
                },
            ],
        }
    }
}
