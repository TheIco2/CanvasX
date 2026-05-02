// prism-runtime/src/gpu/context.rs
//
// GPU device context — initialises wgpu with Vulkan or DX12 backend,
// manages surface, device, queue, and swap chain configuration.

use anyhow::Result;
use std::sync::Arc;

/// The GPU context holds the core wgpu state.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface_format: wgpu::TextureFormat,
    pub size: (u32, u32),
    pub backend: wgpu::Backend,
}

impl GpuContext {
    /// Initialise the GPU context for a given window.
    ///
    /// Backend priority: Vulkan → DX12 → DX11 (fallback).
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Self> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Prefer Vulkan + DX12; allow DX11 as fallback.
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await?;

        let backend = adapter.get_info().backend;
        log::info!(
            "GPU adapter: {} ({:?})",
            adapter.get_info().name,
            backend
        );

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("prism-runtime"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                },
            )
            .await?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // Pick surface format.
        // Use a NON-sRGB format so the GPU does not apply automatic linear→sRGB
        // conversion on write. CSS/browser compositing blends in sRGB space, and
        // our shader keeps colors in sRGB throughout, so a non-sRGB surface gives
        // the most accurate browser-matching output.
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let present_mode = wgpu::PresentMode::Fifo;
        // if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
        //     wgpu::PresentMode::Mailbox
        // } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
        //     wgpu::PresentMode::Immediate
        // } else {
        //     wgpu::PresentMode::Fifo
        // };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            desired_maximum_frame_latency: 3,
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            surface_format,
            size: (width, height),
            backend,
        })
    }

    /// Resize the surface (call on window resize).
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.size = (new_width, new_height);
        self.surface_config.width = new_width;
        self.surface_config.height = new_height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Create a GPU context from a raw Win32 HWND (no winit window required).
    ///
    /// This is used by host applications that create their own Win32 windows
    /// (e.g., the wallpaper addon's WorkerW child windows).
    #[cfg(target_os = "windows")]
    pub async fn from_raw_hwnd(hwnd: isize, width: u32, height: u32) -> Result<Self> {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};

        let width = width.max(1);
        let height = height.max(1);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            ..Default::default()
        });

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                raw_window_handle: RawWindowHandle::Win32(
                    Win32WindowHandle::new(
                        std::num::NonZeroIsize::new(hwnd)
                            .ok_or_else(|| anyhow::anyhow!("Null HWND"))?
                    )
                ),
            })?
        };

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await?;

        let backend = adapter.get_info().backend;
        log::info!(
            "GPU adapter (raw hwnd): {} ({:?})",
            adapter.get_info().name,
            backend
        );

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("prism-runtime-raw"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                },
            )
            .await?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        // Prefer sRGB format for correct color space handling (see comment in new() method).
        let surface_format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        // Use Fifo for deterministic VSync pacing across monitors.
        // Mailbox/Immediate can produce inconsistent uncapped behaviour per output.
        let present_mode = wgpu::PresentMode::Fifo;

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            desired_maximum_frame_latency: 3,
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            surface_format,
            size: (width, height),
            backend,
        })
    }

    /// Switch present mode (e.g. Fifo ↔ Mailbox for VSync toggle).
    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        self.surface_config.present_mode = mode;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Get the current surface texture for rendering.
    pub fn current_texture(&self) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        self.surface.get_current_texture()
    }
}

