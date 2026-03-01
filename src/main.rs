// canvasx-runtime — main entry point
//
// This wires together every subsystem:
//   1. Parse CLI args (scene type, source path, target monitor)
//   2. Create winit window (or embed in WorkerW for wallpapers)
//   3. Initialise GPU context + renderer via wgpu
//   4. Compile HTML/CSS → CXRD (or load cached CXRD)
//   5. Start IPC client (background thread polling host application)
//   6. Enter render loop: layout → animate → paint → submit → present
//
// Binary name: canvasx-rt

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use canvasx_runtime::compiler::html::compile_html;
use canvasx_runtime::gpu::context::GpuContext;
use canvasx_runtime::gpu::renderer::Renderer;
use canvasx_runtime::ipc::client::IpcClient;
use canvasx_runtime::platform::monitor::enumerate_monitors;
use canvasx_runtime::scene::graph::SceneGraph;
use canvasx_runtime::scene::input_handler::{
    InputHandler, RawInputEvent, KeyCode, Modifiers, MouseButton as CxMouseButton,
};
use canvasx_runtime::cxrd::document::{SceneType, CxrdDocument};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

struct CliArgs {
    /// What kind of scene to render.
    scene_type: SceneType,
    /// Path to the HTML file (or .cxrd document) to render.
    source: PathBuf,
    /// Optional CSS override file.
    css_override: Option<PathBuf>,
    /// Which monitor to render on (0 = primary, 1, 2, …).
    monitor_index: usize,
    /// Target FPS (0 = VSync).
    target_fps: u32,
}

impl CliArgs {
    fn from_env() -> Self {
        let args: Vec<String> = std::env::args().collect();

        let mut scene_type = SceneType::ConfigPanel;
        let mut source = PathBuf::from("scene.html");
        let mut css_override: Option<PathBuf> = None;
        let mut monitor_index: usize = 0;
        let mut target_fps: u32 = 0;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--wallpaper" | "-w" => scene_type = SceneType::Wallpaper,
                "--statusbar" | "-s" => scene_type = SceneType::StatusBar,
                "--widget" => scene_type = SceneType::Widget,
                "--config" | "-c" => scene_type = SceneType::ConfigPanel,
                "--source" | "-f" => {
                    i += 1;
                    if i < args.len() {
                        source = PathBuf::from(&args[i]);
                    }
                }
                "--css" => {
                    i += 1;
                    if i < args.len() {
                        css_override = Some(PathBuf::from(&args[i]));
                    }
                }
                "--monitor" | "-m" => {
                    i += 1;
                    if i < args.len() {
                        monitor_index = args[i].parse().unwrap_or(0);
                    }
                }
                "--fps" => {
                    i += 1;
                    if i < args.len() {
                        target_fps = args[i].parse().unwrap_or(0);
                    }
                }
                other => {
                    // Positional: treat as source path.
                    if !other.starts_with('-') {
                        source = PathBuf::from(other);
                    }
                }
            }
            i += 1;
        }

        Self {
            scene_type,
            source,
            css_override,
            monitor_index,
            target_fps,
        }
    }
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

struct App {
    args: CliArgs,
    window: Option<Arc<Window>>,
    gpu_ctx: Option<GpuContext>,
    renderer: Option<Renderer>,
    scene: Option<SceneGraph>,
    input_handler: InputHandler,
    ipc_client: Option<IpcClient>,
    last_frame: Instant,
    frame_count: u64,
    fps_timer: Instant,
    /// Current modifier key state (tracked via ModifiersChanged).
    current_modifiers: winit::keyboard::ModifiersState,
}

impl App {
    fn new(args: CliArgs) -> Self {
        Self {
            args,
            window: None,
            gpu_ctx: None,
            renderer: None,
            scene: None,
            input_handler: InputHandler::new(),
            ipc_client: None,
            last_frame: Instant::now(),
            frame_count: 0,
            fps_timer: Instant::now(),
            current_modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    /// Load the scene document (compile from HTML/CSS or load cached CXRD).
    fn load_scene(&self) -> Result<CxrdDocument> {
        let source = &self.args.source;

        if source.extension().map_or(false, |e| e == "cxrd") {
            // Load pre-compiled CXRD.
            let data = std::fs::read(source)?;
            CxrdDocument::from_binary(&data).map_err(|e| anyhow::anyhow!(e))
        } else {
            // Compile from HTML + CSS.
            let html = std::fs::read_to_string(source)?;

            // Look for a sibling CSS file, or use the override.
            let css = if let Some(ref css_path) = self.args.css_override {
                std::fs::read_to_string(css_path)?
            } else {
                let css_path = source.with_extension("css");
                if css_path.exists() {
                    std::fs::read_to_string(css_path)?
                } else {
                    // Try a style.css next to the HTML.
                    let sibling = source
                        .parent()
                        .map(|p| p.join("style.css"))
                        .unwrap_or_default();
                    if sibling.exists() {
                        std::fs::read_to_string(sibling)?
                    } else {
                        String::new()
                    }
                }
            };

            let name = source
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("scene");

            let doc = compile_html(&html, &css, name, self.args.scene_type, source.parent())?;
            Ok(doc)
        }
    }

    /// Synchronise IPC data into the scene graph.
    fn sync_ipc_data(&mut self) {
        if let (Some(ref ipc), Some(ref mut scene)) = (&self.ipc_client, &mut self.scene) {
            if let Ok(data) = ipc.data.lock() {
                scene.update_data_batch(data.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// winit ApplicationHandler
// ---------------------------------------------------------------------------

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already initialised.
        }

        log::info!("Initialising CanvasX Runtime...");

        // Enumerate monitors.
        let monitors = enumerate_monitors();
        let mi = self
            .args
            .monitor_index
            .min(monitors.len().saturating_sub(1));

        let target_monitor = monitors.get(mi);
        log::info!(
            "Monitors: {} — rendering on #{}{}",
            monitors.len(),
            mi,
            if target_monitor.map_or(false, |m| m.primary) {
                " (primary)"
            } else {
                ""
            },
        );

        // Window attributes.
        let mut attrs = WindowAttributes::default().with_title("CanvasX Runtime");

        if let Some(mon) = target_monitor {
            attrs = attrs
                .with_inner_size(PhysicalSize::new(mon.width, mon.height))
                .with_position(winit::dpi::PhysicalPosition::new(mon.x, mon.y));
        }

        match self.args.scene_type {
            SceneType::Wallpaper => {
                // Borderless, full monitor.
                attrs = attrs
                    .with_decorations(false)
                    .with_resizable(false);
            }
            SceneType::StatusBar => {
                attrs = attrs
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_transparent(true);
                if let Some(mon) = target_monitor {
                    attrs = attrs.with_inner_size(PhysicalSize::new(mon.width, 48u32));
                }
            }
            _ => {
                // Config panel / widget — normal window.
                attrs = attrs.with_inner_size(PhysicalSize::new(1280u32, 800u32));
            }
        }

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

        // For wallpapers: embed in WorkerW layer.
        #[cfg(target_os = "windows")]
        if self.args.scene_type == SceneType::Wallpaper {
            use winit::raw_window_handle::HasWindowHandle;
            if let Ok(handle) = window.window_handle() {
                if let winit::raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
                    let hwnd =
                        windows::Win32::Foundation::HWND(h.hwnd.get() as *mut std::ffi::c_void);
                    if canvasx_runtime::platform::desktop::embed_in_desktop(hwnd) {
                        log::info!("Embedded render window in desktop WorkerW layer");
                    } else {
                        log::warn!("Failed to embed in WorkerW — rendering as overlay");
                    }
                }
            }
        }

        // --- GPU initialisation (blocking) ---
        let gpu_ctx = match pollster::block_on(GpuContext::new(window.clone())) {
            Ok(ctx) => ctx,
            Err(e) => {
                log::error!("GPU init failed: {}", e);
                event_loop.exit();
                return;
            }
        };

        // Renderer.
        let renderer = match Renderer::new(&gpu_ctx) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Renderer init failed: {}", e);
                event_loop.exit();
                return;
            }
        };

        // Load scene document.
        let doc = match self.load_scene() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to load scene: {}", e);
                // Create an empty document so we at least get a window.
                CxrdDocument::new("error", SceneType::ConfigPanel)
            }
        };

        let scene = SceneGraph::new(doc);

        // Start IPC client.
        let ipc_client = IpcClient::start();

        self.window = Some(window.clone());
        self.gpu_ctx = Some(gpu_ctx);
        self.renderer = Some(renderer);
        self.scene = Some(scene);
        self.ipc_client = Some(ipc_client);
        self.last_frame = Instant::now();
        self.fps_timer = Instant::now();

        // Request first frame.
        window.request_redraw();

        // Use Poll for lowest-latency rendering, or WaitUntil for capped FPS.
        if self.args.target_fps > 0 {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested — shutting down.");
                event_loop.exit();
            }

            WindowEvent::Resized(new_size) => {
                if let Some(ref mut ctx) = self.gpu_ctx {
                    ctx.resize(new_size.width, new_size.height);
                    if let Some(ref mut scene) = self.scene {
                        scene.invalidate_layout();
                    }
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                log::info!("Scale factor changed: {}", scale_factor);
                if let Some(ref mut scene) = self.scene {
                    scene.invalidate_layout();
                }
            }

            WindowEvent::RedrawRequested => {
                self.render_frame();
                // Request next frame immediately.
                if let Some(ref w) = self.window {
                    w.request_redraw();
                }
            }

            // --- Input events → InputHandler ---

            WindowEvent::CursorMoved { position, .. } => {
                self.dispatch_input(RawInputEvent::MouseMove {
                    x: position.x as f32,
                    y: position.y as f32,
                });
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left   => CxMouseButton::Left,
                    winit::event::MouseButton::Right  => CxMouseButton::Right,
                    winit::event::MouseButton::Middle => CxMouseButton::Middle,
                    _ => return,
                };
                let (x, y) = self.input_handler.mouse_pos;
                let raw = match state {
                    winit::event::ElementState::Pressed  => RawInputEvent::MouseDown { x, y, button: btn },
                    winit::event::ElementState::Released => RawInputEvent::MouseUp   { x, y, button: btn },
                };
                self.dispatch_input(raw);
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };
                let (x, y) = self.input_handler.mouse_pos;
                self.dispatch_input(RawInputEvent::MouseWheel {
                    x, y, delta_x: dx, delta_y: dy,
                });
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    let mods = Modifiers {
                        ctrl:  self.current_modifiers.control_key(),
                        shift: self.current_modifiers.shift_key(),
                        alt:   self.current_modifiers.alt_key(),
                    };
                    let key = winit_key_to_cx(&event.logical_key);
                    self.dispatch_input(RawInputEvent::KeyDown { key, modifiers: mods });

                    // Also forward text from character keys.
                    if let Some(text) = &event.text {
                        let s = text.to_string();
                        if !s.is_empty() && !mods.ctrl && !mods.alt {
                            let ch = s.chars().next().unwrap_or('\0');
                            if !ch.is_control() {
                                self.dispatch_input(RawInputEvent::TextInput { text: s });
                            }
                        }
                    }
                }
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers.state();
            }

            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.dispatch_input(RawInputEvent::TextInput { text });
            }

            _ => {}
        }
    }
}

impl App {
    /// Forward a raw input event to the InputHandler and process resulting UI events.
    fn dispatch_input(&mut self, raw: RawInputEvent) {
        let Some(ref mut scene) = self.scene else { return };
        let ui_events = self.input_handler.process_event(&mut scene.document, raw);

        for event in ui_events {
            match event {
                canvasx_runtime::scene::input_handler::UiEvent::NavigateRequest { scene_id } => {
                    log::info!("Navigate request: {}", scene_id);
                }
                canvasx_runtime::scene::input_handler::UiEvent::IpcCommand { ns, cmd, args } => {
                    log::info!("IPC command: {}.{} args={:?}", ns, cmd, args);
                }
                canvasx_runtime::scene::input_handler::UiEvent::OpenExternal { url } => {
                    log::info!("Open external: {}", url);
                    #[cfg(target_os = "windows")]
                    { let _ = std::process::Command::new("cmd").args(["/C", "start", &url]).spawn(); }
                }
                other => {
                    log::debug!("UI event: {:?}", other);
                }
            }
        }

        // Apply updated cursor icon.
        if let Some(ref w) = self.window {
            let winit_cursor = match self.input_handler.cursor {
                canvasx_runtime::scene::input_handler::CursorIcon::Default     => winit::window::CursorIcon::Default,
                canvasx_runtime::scene::input_handler::CursorIcon::Pointer     => winit::window::CursorIcon::Pointer,
                canvasx_runtime::scene::input_handler::CursorIcon::Text        => winit::window::CursorIcon::Text,
                canvasx_runtime::scene::input_handler::CursorIcon::Move        => winit::window::CursorIcon::Move,
                canvasx_runtime::scene::input_handler::CursorIcon::NotAllowed  => winit::window::CursorIcon::NotAllowed,
                canvasx_runtime::scene::input_handler::CursorIcon::ResizeNS    => winit::window::CursorIcon::NsResize,
                canvasx_runtime::scene::input_handler::CursorIcon::ResizeEW    => winit::window::CursorIcon::EwResize,
            };
            w.set_cursor(winit::window::Cursor::Icon(winit_cursor));
        }
    }

    fn render_frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        // FPS counter.
        self.frame_count += 1;
        if self.fps_timer.elapsed().as_secs_f32() >= 2.0 {
            let fps = self.frame_count as f64 / self.fps_timer.elapsed().as_secs_f64();
            log::debug!("FPS: {:.1}", fps);
            self.frame_count = 0;
            self.fps_timer = Instant::now();
        }

        // Sync IPC data.
        self.sync_ipc_data();

        let (ctx, renderer, scene) = match (
            self.gpu_ctx.as_ref(),
            self.renderer.as_mut(),
            self.scene.as_mut(),
        ) {
            (Some(c), Some(r), Some(s)) => (c, r, s),
            _ => return,
        };

        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);

        let (vw, vh) = (ctx.size.0 as f32 / scale, ctx.size.1 as f32 / scale);

        // Tick the scene graph: layout → animate → paint.
        let (instances, clear_color) = scene.tick(vw, vh, dt, &mut renderer.font_system);
        let instances = instances.to_vec();
        let text_areas = scene.text_areas();

        // Begin frame → render → present.
        renderer.begin_frame(ctx, dt, scale);
        match renderer.render(ctx, &instances, text_areas, clear_color) {
            Ok(()) => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                // Reconfigure surface on loss.
                if let Some(ref mut gpu) = self.gpu_ctx {
                    let (w, h) = gpu.size;
                    gpu.resize(w, h);
                }
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("GPU out of memory — exiting");
                std::process::exit(1);
            }
            Err(e) => {
                log::warn!("Surface error: {:?}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Winit key → CanvasX KeyCode translation
// ---------------------------------------------------------------------------

fn winit_key_to_cx(key: &winit::keyboard::Key) -> KeyCode {
    use winit::keyboard::{Key as WKey, NamedKey};
    match key {
        WKey::Named(NamedKey::Enter)     => KeyCode::Enter,
        WKey::Named(NamedKey::Tab)       => KeyCode::Tab,
        WKey::Named(NamedKey::Escape)    => KeyCode::Escape,
        WKey::Named(NamedKey::Backspace) => KeyCode::Backspace,
        WKey::Named(NamedKey::Delete)    => KeyCode::Delete,
        WKey::Named(NamedKey::ArrowLeft)  => KeyCode::Left,
        WKey::Named(NamedKey::ArrowRight) => KeyCode::Right,
        WKey::Named(NamedKey::ArrowUp)    => KeyCode::Up,
        WKey::Named(NamedKey::ArrowDown)  => KeyCode::Down,
        WKey::Named(NamedKey::Home)      => KeyCode::Home,
        WKey::Named(NamedKey::End)       => KeyCode::End,
        WKey::Named(NamedKey::PageUp)    => KeyCode::PageUp,
        WKey::Named(NamedKey::PageDown)  => KeyCode::PageDown,
        WKey::Named(NamedKey::Space)     => KeyCode::Space,
        WKey::Character(c) => {
            match c.as_str() {
                "a" | "A" => KeyCode::A,
                "c" | "C" => KeyCode::C,
                "v" | "V" => KeyCode::V,
                "x" | "X" => KeyCode::X,
                "z" | "Z" => KeyCode::Z,
                _ => KeyCode::Other(c.chars().next().unwrap_or('\0') as u32),
            }
        }
        _ => KeyCode::Other(0),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Initialise file-based logger (~/.Sentinel/logs/CanvasX.log).
    // Pass `true` for debug-level output, or `false` for warn-and-above only.
    canvasx_runtime::logging::init(cfg!(debug_assertions));

    let args = CliArgs::from_env();
    log::info!(
        "CanvasX Runtime v{} — scene: {:?}, source: {}",
        env!("CARGO_PKG_VERSION"),
        args.scene_type,
        args.source.display(),
    );

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = App::new(args);

    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("Event loop error: {}", e);
        std::process::exit(1);
    }
}
