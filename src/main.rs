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
use canvasx_runtime::compiler::css::CssRule;
use canvasx_runtime::compiler::html::ScriptBlock;
use canvasx_runtime::gpu::context::GpuContext;
use canvasx_runtime::gpu::renderer::Renderer;
use canvasx_runtime::ipc::client::IpcClient;
use canvasx_runtime::platform::monitor::enumerate_monitors;
use canvasx_runtime::scene::graph::SceneGraph;
use canvasx_runtime::scene::input_handler::{
    InputHandler, RawInputEvent, KeyCode, Modifiers, MouseButton as CxMouseButton,
};
use canvasx_runtime::cxrd::document::{SceneType, CxrdDocument};
use canvasx_runtime::cxrd::node::NodeId;
use canvasx_runtime::scripting::JsRuntime;
use canvasx_runtime::gpu::vertex::UiInstance;

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
    js_runtime: Option<JsRuntime>,
    /// Scripts collected during HTML compilation (executed once on init).
    pending_scripts: Vec<ScriptBlock>,
    /// CSS rules from compilation (passed to JS runtime).
    compiled_css_rules: Vec<CssRule>,
    /// Map from canvas CanvasId → GPU texture asset index (high range).
    canvas_texture_slots: std::collections::HashMap<u32, u32>,
    /// Map from NodeId → CanvasId (mirrors JS runtime's node_canvas_map).
    node_canvas_map: std::collections::HashMap<NodeId, u32>,
    /// Next available GPU texture slot for canvas textures.
    next_canvas_slot: u32,
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
            js_runtime: None,
            pending_scripts: Vec::new(),
            compiled_css_rules: Vec::new(),
            canvas_texture_slots: std::collections::HashMap::new(),
            node_canvas_map: std::collections::HashMap::new(),
            next_canvas_slot: 10000, // high range to avoid colliding with asset textures
            last_frame: Instant::now(),
            frame_count: 0,
            fps_timer: Instant::now(),
            current_modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    /// Load the scene document (compile from HTML/CSS or load cached CXRD).
    /// Returns (document, scripts, css_rules).
    fn load_scene(&self) -> Result<(CxrdDocument, Vec<ScriptBlock>, Vec<CssRule>)> {
        let source = &self.args.source;

        if source.extension().map_or(false, |e| e == "cxrd") {
            // Load pre-compiled CXRD.
            let data = std::fs::read(source)?;
            let doc = CxrdDocument::from_binary(&data).map_err(|e| anyhow::anyhow!(e))?;
            Ok((doc, Vec::new(), Vec::new()))
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

            let (doc, scripts, css_rules) = compile_html(&html, &css, name, self.args.scene_type, source.parent())?;
            Ok((doc, scripts, css_rules))
        }
    }

    /// Synchronise IPC data into the scene graph.
    /// Uses try_lock to avoid blocking the render thread.
    fn sync_ipc_data(&mut self) {
        if let (Some(ref ipc), Some(ref mut scene)) = (&self.ipc_client, &mut self.scene) {
            if let Ok(mut data) = ipc.data.try_lock() {
                if !data.is_empty() {
                    // Swap out the data to avoid clone
                    let taken = std::mem::take(&mut *data);
                    scene.update_data_batch(taken);
                }
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
        let (doc, scripts, css_rules) = match self.load_scene() {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to load scene: {}", e);
                // Create an empty document so we at least get a window.
                (CxrdDocument::new("error", SceneType::ConfigPanel), Vec::new(), Vec::new())
            }
        };

        let scene = SceneGraph::new(doc.clone());

        // Start IPC client.
        let ipc_client = IpcClient::start();

        // Store CSS rules and scripts for JS runtime initialization.
        self.pending_scripts = scripts;
        self.compiled_css_rules = css_rules;

        self.window = Some(window.clone());
        self.gpu_ctx = Some(gpu_ctx);
        self.renderer = Some(renderer);
        self.scene = Some(scene);
        self.ipc_client = Some(ipc_client);
        self.last_frame = Instant::now();
        self.fps_timer = Instant::now();

        // Initialize JS runtime with the compiled document.
        let css_variables: std::collections::HashMap<String, String> = doc.variables.iter().cloned().collect();
        let css_rules_for_js = self.compiled_css_rules.clone();
        let mut js_rt = JsRuntime::new(doc, css_rules_for_js, css_variables);

        // Initialize canvas elements.
        let vw = window.inner_size().width;
        let vh = window.inner_size().height;
        js_rt.init_canvases(vw, vh);

        // Execute collected scripts.
        let source_dir = self.args.source.parent().map(|p| p.to_path_buf());
        let scripts = std::mem::take(&mut self.pending_scripts);
        for script in &scripts {
            if let Some(ref src) = script.src {
                // External script — resolve relative to source directory.
                let script_path = if let Some(ref dir) = source_dir {
                    dir.join(src)
                } else {
                    PathBuf::from(src)
                };
                log::info!("Loading script: {}", script_path.display());
                js_rt.execute_file(&script_path);
            } else if !script.content.is_empty() {
                log::info!("Executing inline script ({} bytes)", script.content.len());
                js_rt.execute(&script.content, "<inline>");
            }
        }

        js_rt.cache_raf_tick_fn();
        self.js_runtime = Some(js_rt);

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

        // FPS counter (lightweight — only check every 128 frames).
        self.frame_count += 1;
        if self.frame_count & 0x7F == 0 {
            let elapsed = self.fps_timer.elapsed().as_secs_f64();
            if elapsed >= 2.0 {
                let fps = self.frame_count as f64 / elapsed;
                log::debug!("FPS: {:.1}", fps);
                self.frame_count = 0;
                self.fps_timer = Instant::now();
            }
        }

        // Sync IPC data.
        self.sync_ipc_data();

        // Tick JS runtime (requestAnimationFrame, timers, etc.).
        if let Some(ref mut js_rt) = self.js_runtime {
            // Prevent unbounded per-frame gradient allocations from JS canvas code.
            js_rt.gc_gradients();
            let _js_dirty = js_rt.tick(dt);

            // If JS modified the DOM, sync document back to scene.
            if js_rt.take_layout_dirty() {
                if let Some(ref mut scene) = self.scene {
                    // Borrow document directly instead of clone to avoid allocation.
                    let new_doc = js_rt.document();
                    scene.load_document(new_doc.clone());
                    drop(new_doc);
                }
            }
        }

        let (ctx, renderer, scene) = match (
            self.gpu_ctx.as_ref(),
            self.renderer.as_mut(),
            self.scene.as_mut(),
        ) {
            (Some(c), Some(r), Some(s)) => (c, r, s),
            _ => return,
        };

        // Upload dirty canvas textures to GPU.
        if let Some(ref mut js_rt) = self.js_runtime {
            let dirty = js_rt.dirty_canvases();
            for (canvas_id, _node_id, w, h, pixels) in dirty {
                // Get or assign a GPU texture slot for this canvas.
                let slot = *self.canvas_texture_slots.entry(canvas_id).or_insert_with(|| {
                    let s = self.next_canvas_slot;
                    self.next_canvas_slot += 1;
                    s
                });
                renderer.upload_canvas_texture(&ctx.device, &ctx.queue, slot, w, h, &pixels);
            }

            // Mark uploaded canvases clean so we only re-upload on real changes.
            js_rt.clear_dirty_flags();

            // Update node→canvas→slot mapping only when canvas set changed.
            let state = js_rt.state.borrow();
            if state.node_canvas_map.len() != self.node_canvas_map.len() {
                self.node_canvas_map.clear();
                for (&node_id, &canvas_id) in &state.node_canvas_map {
                    if let Some(&slot) = self.canvas_texture_slots.get(&canvas_id) {
                        self.node_canvas_map.insert(node_id, slot);
                    }
                }
            }
        }

        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);

        let (vw, vh) = (ctx.size.0 as f32 / scale, ctx.size.1 as f32 / scale);

        // Tick the scene graph: layout → animate → paint.
        // Split into two phases to avoid borrow conflicts:
        // 1. Run tick (mutates scene, updates cached_instances and gradient_textures)
        scene.tick(vw, vh, dt, &mut renderer.font_system);

        // 2. Upload gradient textures (before borrowing cached_instances)
        if scene.take_gradient_textures_dirty() {
            for grad in &scene.cached_gradient_textures {
                renderer.upload_canvas_texture(&ctx.device, &ctx.queue, grad.slot, grad.width, grad.height, &grad.rgba);
            }
        }

        // 3. Now borrow instances and clear color immutably
        let instances = scene.cached_instances.as_slice();
        let clear_color = scene.document.background;

        // Patch canvas instances with their actual GPU texture slot.
        // Build patched list only if canvas textures exist (avoid copy when not needed).
        let has_canvas_patches = instances.iter().any(|inst| {
            inst.texture_index <= -2
                && (inst.flags & canvasx_runtime::gpu::vertex::UiInstance::FLAG_HAS_TEXTURE) != 0
        });

        let patched_instances;
        let final_instances: &[UiInstance] = if has_canvas_patches {
            patched_instances = instances.iter().map(|inst| {
                if inst.texture_index <= -2
                    && (inst.flags & canvasx_runtime::gpu::vertex::UiInstance::FLAG_HAS_TEXTURE) != 0
                {
                    let node_id = (-inst.texture_index - 2) as u32;
                    if let Some(&slot) = self.node_canvas_map.get(&node_id) {
                        let mut patched = *inst;
                        patched.texture_index = slot as i32;
                        return patched;
                    }
                }
                *inst
            }).collect::<Vec<_>>();
            &patched_instances
        } else {
            instances
        };

        let text_areas = scene.text_areas();

        // Begin frame → render → present.
        renderer.begin_frame(ctx, dt, scale);
        match renderer.render(ctx, final_instances, text_areas, clear_color) {
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
