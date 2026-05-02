// prism-runtime/src/run.rs
//
// Minimal turn-key winit + wgpu event loop for an `AppHost`.
//
// This is what `prism_runtime::start()` calls under the hood — extracted
// here so consumers who want more control can call `run_app(host)` directly
// without the global `Prism` singleton.

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::gpu::context::GpuContext;
use crate::gpu::renderer::Renderer;
use crate::scene::app_host::{AppEvent, AppHost};
use crate::scene::input_handler::{
    KeyCode, Modifiers, MouseButton as CxMouseButton, RawInputEvent,
};

/// Window options for [`run_app`].
#[derive(Debug, Clone)]
pub struct WindowOptions {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self { title: "Prism".into(), width: 1024, height: 768 }
    }
}

/// Run an [`AppHost`] in a freshly-created window. Blocks until the window
/// is closed.
pub fn run_app(host: AppHost, opts: WindowOptions) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = HostedApp::new(host, opts);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct HostedApp {
    host: AppHost,
    opts: WindowOptions,
    window: Option<Arc<Window>>,
    gpu_ctx: Option<GpuContext>,
    renderer: Option<Renderer>,
    last_frame: Instant,
    cursor_pos: (f32, f32),
    current_modifiers: winit::keyboard::ModifiersState,
}

impl HostedApp {
    fn new(host: AppHost, opts: WindowOptions) -> Self {
        Self {
            host,
            opts,
            window: None,
            gpu_ctx: None,
            renderer: None,
            last_frame: Instant::now(),
            cursor_pos: (0.0, 0.0),
            current_modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    fn dispatch_input(&mut self, raw: RawInputEvent) {
        let (vw, vh) = self.viewport_size();
        self.host.handle_input(raw, vw, vh);
    }

    fn viewport_size(&self) -> (f32, f32) {
        let ctx = match self.gpu_ctx.as_ref() {
            Some(c) => c,
            None => return (self.opts.width as f32, self.opts.height as f32),
        };
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        (ctx.size.0 as f32 / scale, ctx.size.1 as f32 / scale)
    }

    fn render_frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        let (vw, vh, scale, events) = {
            let (ctx, renderer) = match (self.gpu_ctx.as_ref(), self.renderer.as_mut()) {
                (Some(c), Some(r)) => (c, r),
                _ => return,
            };
            let scale = self
                .window
                .as_ref()
                .map(|w| w.scale_factor() as f32)
                .unwrap_or(1.0);
            let vw = ctx.size.0 as f32 / scale;
            let vh = ctx.size.1 as f32 / scale;
            let events = self.host.tick(vw, vh, dt, &mut renderer.font_system);
            (vw, vh, scale, events)
        };

        for event in events {
            if matches!(event, AppEvent::CloseRequested) {
                event_loop.exit();
                return;
            }
        }

        let (ctx, renderer) = match (self.gpu_ctx.as_ref(), self.renderer.as_mut()) {
            (Some(c), Some(r)) => (c, r),
            _ => return,
        };

        let (scene_instances, devtools_instances, clear_color) =
            self.host.split_instances(vw, vh);
        let text_areas = if let Some(scene) = self.host.active_scene() {
            scene.text_areas()
        } else {
            Vec::new()
        };

        renderer.begin_frame(ctx, dt, scale);
        let _ = renderer.render_triple_layered(
            ctx,
            &scene_instances,
            text_areas,
            &devtools_instances,
            Vec::new(),
            &[],
            Vec::new(),
            clear_color,
        );
    }
}

impl ApplicationHandler for HostedApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title(&self.opts.title)
            .with_inner_size(PhysicalSize::new(self.opts.width, self.opts.height))
            .with_decorations(true);

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("[prism::run] failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let gpu_ctx = match pollster::block_on(GpuContext::new(window.clone())) {
            Ok(ctx) => ctx,
            Err(e) => {
                log::error!("[prism::run] GPU init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let renderer = match Renderer::new(&gpu_ctx) {
            Ok(r) => r,
            Err(e) => {
                log::error!("[prism::run] renderer init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        self.window = Some(window.clone());
        self.gpu_ctx = Some(gpu_ctx);
        self.renderer = Some(renderer);

        let (w, h) = self
            .gpu_ctx
            .as_ref()
            .map(|c| c.size)
            .unwrap_or((self.opts.width, self.opts.height));
        self.host.init_js_for_active_page(w, h);

        window.request_redraw();
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                if let Some(ref mut ctx) = self.gpu_ctx {
                    ctx.resize(new_size.width, new_size.height);
                    if let Some(scene) = self.host.active_scene_mut() {
                        scene.invalidate_layout();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame(event_loop);
                if let Some(ref w) = self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor() as f32)
                    .unwrap_or(1.0);
                self.cursor_pos = (position.x as f32 / scale, position.y as f32 / scale);
                self.dispatch_input(RawInputEvent::MouseMove {
                    x: self.cursor_pos.0,
                    y: self.cursor_pos.1,
                });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left => CxMouseButton::Left,
                    winit::event::MouseButton::Right => CxMouseButton::Right,
                    winit::event::MouseButton::Middle => CxMouseButton::Middle,
                    _ => return,
                };
                let raw = match state {
                    winit::event::ElementState::Pressed => RawInputEvent::MouseDown {
                        x: self.cursor_pos.0,
                        y: self.cursor_pos.1,
                        button: btn,
                    },
                    winit::event::ElementState::Released => RawInputEvent::MouseUp {
                        x: self.cursor_pos.0,
                        y: self.cursor_pos.1,
                        button: btn,
                    },
                };
                self.dispatch_input(raw);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    let mods = Modifiers {
                        ctrl: self.current_modifiers.control_key(),
                        shift: self.current_modifiers.shift_key(),
                        alt: self.current_modifiers.alt_key(),
                    };
                    let key = match &event.logical_key {
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                            KeyCode::Escape
                        }
                        _ => KeyCode::Other(0),
                    };
                    self.dispatch_input(RawInputEvent::KeyDown { key, modifiers: mods });
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers.state();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(ref w) = self.window {
            w.request_redraw();
        }
    }
}
