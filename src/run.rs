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
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Icon, Window, WindowAttributes, WindowId, WindowLevel};

use crate::gpu::context::GpuContext;
use crate::gpu::renderer::Renderer;
use crate::scene::app_host::{AppEvent, AppHost};
use crate::scene::input_handler::{
    CursorIcon, KeyCode, Modifiers, MouseButton as CxMouseButton, RawInputEvent,
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
    /// Frameless popup window for the custom HTML tray menu.
    /// Lazily created on the first right-click of the tray icon.
    /// Declared **before** `host` so its resources (its own AppHost,
    /// renderer and gpu context) are dropped first — critical because
    /// dropping out-of-creation-order V8/wgpu resources panics.
    tray_popup: Option<TrayMenuPopup>,
    host: AppHost,
    opts: WindowOptions,
    window: Option<Arc<Window>>,
    gpu_ctx: Option<GpuContext>,
    renderer: Option<Renderer>,
    last_frame: Instant,
    cursor_pos: (f32, f32),
    current_modifiers: winit::keyboard::ModifiersState,
    /// Cached cursor icon last applied to the OS window. We only call
    /// `set_cursor` when it changes, so the OS can manage non-client-area
    /// cursors (e.g. resize cursors at the window border) without us
    /// stomping them every frame.
    last_applied_cursor: Option<CursorIcon>,
}

/// Frameless popup that hosts the user-supplied tray menu HTML in its own
/// `AppHost` so it can render with the full PRISM CSS pipeline and dispatch
/// click events through the normal `data-action` machinery.
struct TrayMenuPopup {
    window: Arc<Window>,
    gpu: GpuContext,
    renderer: Renderer,
    host: AppHost,
    visible: bool,
    cursor_pos: (f32, f32),
    last_frame: Instant,
    /// Set when the popup needs to be hidden — handled in `about_to_wait`
    /// so we never call `set_visible(false)` while we're still inside the
    /// event the OS dispatched to this window. Hiding mid-event has been
    /// observed to recursively re-enter the focus handler on Windows.
    pending_dismiss: bool,
    /// Whether the window has been resized to fit its laid-out content.
    /// We do this once per show so the menu hugs its items rather than
    /// stretching to a fixed default.
    auto_fitted: bool,
}

impl HostedApp {
    fn new(host: AppHost, opts: WindowOptions) -> Self {
        Self {
            tray_popup: None,
            host,
            opts,
            window: None,
            gpu_ctx: None,
            renderer: None,
            last_frame: Instant::now(),
            cursor_pos: (0.0, 0.0),
            current_modifiers: winit::keyboard::ModifiersState::empty(),
            last_applied_cursor: None,
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
            let events = self.host.tick(vw, vh, dt, &mut renderer.font_system, scale);
            // Drain any host-action requests JS made this frame (e.g. the
            // long-press handler asking us to open the built-in PRISM
            // context menu).
            self.host.process_pending_js_requests(vw, vh);
            (vw, vh, scale, events)
        };

        for event in events {
            match event {
                AppEvent::CloseRequested => {
                    event_loop.exit();
                    return;
                }
                AppEvent::TrayShowWindow => {
                    if let Some(ref w) = self.window {
                        w.set_visible(true);
                        w.focus_window();
                    }
                }
                AppEvent::TrayToggleWindow => {
                    if let Some(ref w) = self.window {
                        let visible = self.host.window_visible;
                        w.set_visible(visible);
                        if visible {
                            w.focus_window();
                        }
                    }
                }
                AppEvent::ShowCustomTrayMenu { x, y } => {
                    self.show_tray_menu(event_loop, x, y);
                }
                AppEvent::PageReloaded(_) => {
                    let (w, h) = self
                        .gpu_ctx
                        .as_ref()
                        .map(|c| c.size)
                        .unwrap_or((self.opts.width, self.opts.height));
                    self.host.init_js_for_active_page(w, h);
                }
                _ => {}
            }
        }

        // Upload any new GPU assets (e.g. rasterized SVG textures) that were
        // registered during the tick above (page loads, innerHTML SVGs, etc.).
        if self.host.take_assets_dirty() {
            if let (Some(ctx), Some(renderer)) = (self.gpu_ctx.as_ref(), self.renderer.as_mut()) {
                if let Some(assets) = self.host.active_scene_assets() {
                    renderer.load_assets(&ctx.device, &ctx.queue, assets);
                }
            }
        }

        // Apply CSS cursor to the OS window only when it changes. Calling
        // `set_cursor` every frame on Windows continually overrides the
        // cursor the OS sets while hovering a non-client-area edge, which
        // is what produces the OS-rendered resize cursors at the window
        // border. By caching the last applied cursor we leave the OS in
        // charge of non-client cursors and only update when our own
        // computed cursor actually transitions.
        if let Some(ref w) = self.window {
            let cur = self.host.current_cursor();
            if self.last_applied_cursor != Some(cur) {
                let winit_cursor = match cur {
                    CursorIcon::Pointer    => winit::window::CursorIcon::Pointer,
                    CursorIcon::Text       => winit::window::CursorIcon::Text,
                    CursorIcon::Move       => winit::window::CursorIcon::Move,
                    CursorIcon::NotAllowed => winit::window::CursorIcon::NotAllowed,
                    CursorIcon::ResizeNS   => winit::window::CursorIcon::NsResize,
                    CursorIcon::ResizeEW   => winit::window::CursorIcon::EwResize,
                    CursorIcon::Default    => winit::window::CursorIcon::Default,
                };
                w.set_cursor(winit::window::Cursor::Icon(winit_cursor));
                self.last_applied_cursor = Some(cur);
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

        // Shape DevTools / context-menu text (logical positions; the renderer
        // scales positions+bounds by scale_factor at draw time, and we shape
        // glyphs at physical font metrics for crisp output on high-DPI).
        let devtools_text_entries = self.host.devtools_text_entries(vw, vh);
        let mut devtools_buffers: Vec<glyphon::Buffer> = Vec::with_capacity(devtools_text_entries.len());
        for entry in &devtools_text_entries {
            let font_size = entry.font_size * scale;
            let line_height = font_size * 1.3;
            let metrics = glyphon::Metrics::new(font_size, line_height);
            let mut buffer = glyphon::Buffer::new(&mut renderer.font_system, metrics);
            let weight = if entry.bold { glyphon::Weight(700) } else { glyphon::Weight(400) };
            let attrs = glyphon::Attrs::new()
                .family(glyphon::Family::SansSerif)
                .weight(weight);
            buffer.set_size(&mut renderer.font_system, Some(entry.width * scale), None);
            buffer.set_text(&mut renderer.font_system, &entry.text, &attrs, glyphon::Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut renderer.font_system, false);
            devtools_buffers.push(buffer);
        }
        let mut devtools_text_areas: Vec<glyphon::TextArea<'_>> = Vec::with_capacity(devtools_buffers.len());
        for (i, entry) in devtools_text_entries.iter().enumerate() {
            if let Some(buf) = devtools_buffers.get(i) {
                let c = entry.color;
                let bounds = if let Some(cl) = entry.clip {
                    glyphon::TextBounds {
                        left:   (cl[0] * scale) as i32,
                        top:    (cl[1] * scale) as i32,
                        right:  (cl[2] * scale) as i32,
                        bottom: (cl[3] * scale) as i32,
                    }
                } else {
                    glyphon::TextBounds {
                        left: 0,
                        top: 0,
                        right: vw as i32,
                        bottom: vh as i32,
                    }
                };
                devtools_text_areas.push(glyphon::TextArea {
                    buffer: buf,
                    left: entry.x,
                    top: entry.y,
                    scale: 1.0,
                    bounds,
                    default_color: glyphon::Color::rgba(
                        (c.r * 255.0) as u8,
                        (c.g * 255.0) as u8,
                        (c.b * 255.0) as u8,
                        (c.a * 255.0) as u8,
                    ),
                    custom_glyphs: &[],
                });
            }
        }

        renderer.begin_frame(ctx, dt, scale);

        // Context menu is rendered in a SEPARATE overlay layer so its rect
        // and text always paint on top of devtools panel text (which would
        // otherwise bleed through the menu in the same glyphon batch).
        // We also include "post-text" devtools chrome (e.g. Elements
        // breadcrumb cover rect + label) in this layer so the breadcrumb
        // is opaque over any tree row text drawn behind it.
        let mut overlay_instances = self.host.devtools_post_text_instances(vw, vh);
        overlay_instances.extend(self.host.context_menu_instances());

        let mut overlay_entries = self.host.devtools_post_text_entries(vw, vh);
        overlay_entries.extend(self.host.context_menu_text_entries());
        let mut overlay_buffers: Vec<glyphon::Buffer> = Vec::with_capacity(overlay_entries.len());
        for entry in &overlay_entries {
            let font_size = entry.font_size * scale;
            let line_height = font_size * 1.3;
            let metrics = glyphon::Metrics::new(font_size, line_height);
            let mut buffer = glyphon::Buffer::new(&mut renderer.font_system, metrics);
            let weight = if entry.bold { glyphon::Weight(700) } else { glyphon::Weight(400) };
            let attrs = glyphon::Attrs::new()
                .family(glyphon::Family::SansSerif)
                .weight(weight);
            buffer.set_size(&mut renderer.font_system, Some(entry.width * scale), None);
            buffer.set_text(&mut renderer.font_system, &entry.text, &attrs, glyphon::Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut renderer.font_system, false);
            overlay_buffers.push(buffer);
        }
        let mut overlay_text_areas: Vec<glyphon::TextArea<'_>> = Vec::with_capacity(overlay_buffers.len());
        for (i, entry) in overlay_entries.iter().enumerate() {
            if let Some(buf) = overlay_buffers.get(i) {
                let c = entry.color;
                let bounds = if let Some(cl) = entry.clip {
                    glyphon::TextBounds {
                        left:   (cl[0] * scale) as i32,
                        top:    (cl[1] * scale) as i32,
                        right:  (cl[2] * scale) as i32,
                        bottom: (cl[3] * scale) as i32,
                    }
                } else {
                    glyphon::TextBounds {
                        left: 0,
                        top: 0,
                        right: vw as i32,
                        bottom: vh as i32,
                    }
                };
                overlay_text_areas.push(glyphon::TextArea {
                    buffer: buf,
                    left: entry.x,
                    top: entry.y,
                    scale: 1.0,
                    bounds,
                    default_color: glyphon::Color::rgba(
                        (c.r * 255.0) as u8,
                        (c.g * 255.0) as u8,
                        (c.b * 255.0) as u8,
                        (c.a * 255.0) as u8,
                    ),
                    custom_glyphs: &[],
                });
            }
        }

        let _ = renderer.render_triple_layered(
            ctx,
            &scene_instances,
            text_areas,
            &devtools_instances,
            devtools_text_areas,
            &overlay_instances,
            overlay_text_areas,
            clear_color,
        );
    }

    // ---------------------------------------------------------------------
    // Custom HTML tray menu popup
    // ---------------------------------------------------------------------

    /// Show the custom tray menu popup at physical screen position `(x, y)`.
    /// Lazily creates the window, GPU context, renderer and a dedicated
    /// `AppHost` the first time it is called.
    fn show_tray_menu(&mut self, event_loop: &ActiveEventLoop, x: f64, y: f64) {
        let menu_path = match self.host.tray_menu_html_path() {
            Some(p) => p.to_string(),
            None => return,
        };

        // Lazy construction.
        if self.tray_popup.is_none() {
            let popup = match Self::build_tray_popup(event_loop, &menu_path) {
                Some(p) => p,
                None => return,
            };
            self.tray_popup = Some(popup);
        }

        if let Some(popup) = self.tray_popup.as_mut() {
            // ---- Measure pass --------------------------------------------
            // Run one offscreen tick at the popup's current (generously-
            // sized) viewport so we know the menu's actual content size,
            // THEN resize+position+show. Doing it in this order avoids the
            // user seeing the window flash at full size or end up flipped
            // upward as if it were the full size.
            let scale_f = popup.window.scale_factor() as f32;
            let init_size = popup.window.inner_size();
            let init_vw = init_size.width as f32 / scale_f;
            let init_vh = init_size.height as f32 / scale_f;
            let _ = popup.host.tick(
                init_vw,
                init_vh,
                0.0,
                &mut popup.renderer.font_system,
                scale_f,
            );
            let (measure_instances, _, _) =
                popup.host.split_instances(init_vw, init_vh);
            let mut max_w: f32 = 0.0;
            let mut max_h: f32 = 0.0;
            for inst in &measure_instances {
                let r = inst.rect;
                let w = r[2];
                let h = r[3];
                // Skip full-viewport html/body backgrounds.
                if w >= init_vw - 0.5 && h >= init_vh - 0.5 {
                    continue;
                }
                // Skip box-shadow / outline expansions (negative origin).
                if r[0] < 0.0 || r[1] < 0.0 {
                    continue;
                }
                max_w = max_w.max(r[0] + w);
                max_h = max_h.max(r[1] + h);
            }
            // Fall back to the initial size if we couldn't measure anything
            // useful (shouldn't happen, but be defensive).
            let fit_w = if max_w > 4.0 {
                (max_w * scale_f).ceil() as u32
            } else {
                init_size.width
            };
            let fit_h = if max_h > 4.0 {
                (max_h * scale_f).ceil() as u32
            } else {
                init_size.height
            };
            if fit_w != init_size.width || fit_h != init_size.height {
                let _ = popup
                    .window
                    .request_inner_size(PhysicalSize::new(fit_w, fit_h));
                popup.gpu.resize(fit_w, fit_h);
                if let Some(scene) = popup.host.active_scene_mut() {
                    scene.invalidate_layout();
                }
            }
            popup.auto_fitted = true;

            // Clamp the popup to the current monitor's work area so it
            // doesn't get cut off by the taskbar / screen edges. The tray
            // icon is almost always at the bottom of the screen, so by
            // default the menu will need to flip upward.
            let pw = fit_w as f64;
            let ph = fit_h as f64;

            let monitor = popup
                .window
                .current_monitor()
                .or_else(|| popup.window.available_monitors().next());
            let (mon_x, mon_y, mon_w, mon_h) = if let Some(m) = monitor {
                let pos = m.position();
                let sz = m.size();
                (
                    pos.x as f64,
                    pos.y as f64,
                    sz.width as f64,
                    sz.height as f64,
                )
            } else {
                (0.0, 0.0, 1920.0, 1080.0)
            };

            // Flip vertically when there isn't enough room below the cursor.
            let mut px = x;
            let mut py = y;
            if py + ph > mon_y + mon_h {
                py = (y - ph).max(mon_y);
            }
            // Flip horizontally when overflowing the right edge.
            if px + pw > mon_x + mon_w {
                px = (x - pw).max(mon_x);
            }
            // Final clamp inside the monitor bounds.
            px = px.clamp(mon_x, mon_x + mon_w - pw.min(mon_w));
            py = py.clamp(mon_y, mon_y + mon_h - ph.min(mon_h));

            popup
                .window
                .set_outer_position(PhysicalPosition::new(px, py));
            popup.window.set_visible(true);
            popup.window.focus_window();
            popup.visible = true;
            popup.window.request_redraw();
        }
    }

    /// Hide the popup without destroying it (so re-showing is instant).
    fn hide_tray_menu(&mut self) {
        if let Some(popup) = self.tray_popup.as_mut() {
            popup.window.set_visible(false);
            popup.visible = false;
            popup.pending_dismiss = false;
        }
    }

    /// Mark the popup for dismissal at the next safe point (in `about_to_wait`).
    /// Use this from inside event handlers to avoid hiding the window while
    /// we're still processing one of its events.
    fn schedule_tray_menu_dismiss(&mut self) {
        if let Some(popup) = self.tray_popup.as_mut() {
            if popup.visible {
                popup.pending_dismiss = true;
            }
        }
    }

    fn build_tray_popup(
        event_loop: &ActiveEventLoop,
        menu_path: &str,
    ) -> Option<TrayMenuPopup> {
        use winit::platform::windows::WindowAttributesExtWindows;

        let attrs = WindowAttributes::default()
            .with_title("Tray Menu")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_visible(false)
            .with_inner_size(PhysicalSize::new(240u32, 600u32))
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true);

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("[prism::tray-menu] failed to create popup window: {e}");
                return None;
            }
        };

        let gpu = match pollster::block_on(GpuContext::new(window.clone())) {
            Ok(c) => c,
            Err(e) => {
                log::error!("[prism::tray-menu] GPU init failed: {e}");
                return None;
            }
        };
        let renderer = match Renderer::new(&gpu) {
            Ok(r) => r,
            Err(e) => {
                log::error!("[prism::tray-menu] renderer init failed: {e}");
                return None;
            }
        };

        // Build a minimal AppHost containing only the tray-menu route.
        // We load the framework CSS (linked via <link> in tray-menu.html)
        // through the `theme_css` channel because `load_embedded_document_full`
        // doesn't follow <link> tags itself.
        use crate::scene::app_host::{PageSource, Route};
        let mut host = AppHost::new("tray-menu");
        // The popup is a tiny standalone window — it must NOT reserve room
        // for the OpenDesktop sidebar (which `AppHost::tick` subtracts from
        // viewport_width by default, producing a negative content width and
        // an empty layout in our 220px-wide popup).
        host.sidebar_visible = false;
        host.sidebar_width = 0.0;
        let mut combined_css = String::new();
        if let Some(framework) = crate::embed::read_page_str("assets/css/PrismFramework.css") {
            combined_css.push_str(framework);
            combined_css.push('\n');
        }
        if !combined_css.is_empty() {
            host.set_theme_css(Some(combined_css));
        }
        host.add_route(Route {
            id: "tray-menu".to_string(),
            label: "tray-menu".to_string(),
            icon: None,
            source: PageSource::Embedded(menu_path.to_string()),
            separator: false,
        });
        host.set_home_route("tray-menu");
        host.navigate_to("tray-menu");
        // IMPORTANT: do NOT call `init_js_for_active_page` for the popup.
        // Each AppHost that initialises JS creates its own V8 OwnedIsolate,
        // and v8::OwnedIsolate panics when two isolates exist on the same
        // thread ("PinnedRef ... do not belong to the same Isolate").
        // The tray menu has no scripts — `data-action` is wired directly
        // by the HTML compiler and dispatched by the input handler.
        let _ = gpu.size; // keep gpu live in this scope; layout uses popup.gpu later

        Some(TrayMenuPopup {
            window,
            gpu,
            renderer,
            host,
            visible: false,
            cursor_pos: (0.0, 0.0),
            last_frame: Instant::now(),
            pending_dismiss: false,
            auto_fitted: false,
        })
    }

    /// Translate a `data-action` command (delivered via `AppEvent::IpcCommand`
    /// from the popup host) into an action on the **main** window/host.
    /// Returns `true` if the popup should be dismissed afterwards.
    fn apply_tray_menu_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        cmd: &str,
    ) -> bool {
        let lc = cmd.to_ascii_lowercase();
        match lc.as_str() {
            "open" | "show" | "showwindow" => {
                if let Some(ref w) = self.window {
                    self.host.window_visible = true;
                    w.set_visible(true);
                    w.focus_window();
                }
            }
            "close" | "exit" | "quit" => {
                event_loop.exit();
            }
            "reload" => {
                self.host.reload_active_page();
            }
            "autostart" | "toggleautostart" | "toggle-autostart" | "run-at-startup" => {
                log::info!("[prism::tray-menu] toggle autostart");
            }
            "checkforupdate" | "check-for-update" | "update" => {
                log::info!("[prism::tray-menu] check for update");
            }
            "minimize" => {
                if let Some(ref w) = self.window {
                    w.set_minimized(true);
                }
            }
            other => {
                log::info!("[prism::tray-menu] custom action: {other}");
            }
        }
        true
    }

    /// Handle a `WindowEvent` that targets the tray-menu popup window.
    fn handle_popup_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.schedule_tray_menu_dismiss();
            }
            WindowEvent::Focused(false) => {
                // Click-outside-to-dismiss — deferred so we don't re-enter
                // the focus handler by hiding the window mid-event.
                self.schedule_tray_menu_dismiss();
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                if ke.state == winit::event::ElementState::Pressed
                    && matches!(
                        ke.logical_key,
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
                    )
                {
                    self.schedule_tray_menu_dismiss();
                }
            }
            WindowEvent::Resized(new_size) => {
                if let Some(popup) = self.tray_popup.as_mut() {
                    popup.gpu.resize(new_size.width, new_size.height);
                    if let Some(scene) = popup.host.active_scene_mut() {
                        scene.invalidate_layout();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(popup) = self.tray_popup.as_mut() {
                    let scale = popup.window.scale_factor() as f32;
                    popup.cursor_pos =
                        (position.x as f32 / scale, position.y as f32 / scale);
                    let (vw, vh) = popup_viewport(popup);
                    popup.host.handle_input(
                        RawInputEvent::MouseMove {
                            x: popup.cursor_pos.0,
                            y: popup.cursor_pos.1,
                        },
                        vw,
                        vh,
                    );
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left => CxMouseButton::Left,
                    winit::event::MouseButton::Right => CxMouseButton::Right,
                    winit::event::MouseButton::Middle => CxMouseButton::Middle,
                    _ => return,
                };
                if let Some(popup) = self.tray_popup.as_mut() {
                    let (cx, cy) = popup.cursor_pos;
                    let raw = match state {
                        winit::event::ElementState::Pressed => RawInputEvent::MouseDown {
                            x: cx,
                            y: cy,
                            button: btn,
                        },
                        winit::event::ElementState::Released => RawInputEvent::MouseUp {
                            x: cx,
                            y: cy,
                            button: btn,
                        },
                    };
                    let (vw, vh) = popup_viewport(popup);
                    popup.host.handle_input(raw, vw, vh);
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_popup_frame(event_loop);
                if let Some(popup) = self.tray_popup.as_ref() {
                    if popup.visible {
                        popup.window.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn render_popup_frame(&mut self, event_loop: &ActiveEventLoop) {
        let popup = match self.tray_popup.as_mut() {
            Some(p) if p.visible && !p.pending_dismiss => p,
            _ => return,
        };

        let now = Instant::now();
        let dt = now.duration_since(popup.last_frame).as_secs_f32();
        popup.last_frame = now;

        let scale = popup.window.scale_factor() as f32;
        let vw = popup.gpu.size.0 as f32 / scale;
        let vh = popup.gpu.size.1 as f32 / scale;
        let events =
            popup.host.tick(vw, vh, dt, &mut popup.renderer.font_system, scale);

        // Drain popup events: data-action="open" etc. arrive as IpcCommand
        // with ns="" and cmd=<action-string>.
        let mut should_dismiss = false;
        let mut should_exit = false;
        for event in events {
            match event {
                AppEvent::IpcCommand { ns, cmd, .. } if ns.is_empty() => {
                    if self.apply_tray_menu_command(event_loop, &cmd) {
                        should_dismiss = true;
                    }
                    if cmd.eq_ignore_ascii_case("close")
                        || cmd.eq_ignore_ascii_case("exit")
                        || cmd.eq_ignore_ascii_case("quit")
                    {
                        should_exit = true;
                    }
                }
                AppEvent::CloseRequested => {
                    should_dismiss = true;
                }
                _ => {}
            }
        }
        if should_exit {
            return;
        }
        if should_dismiss {
            self.hide_tray_menu();
            return;
        }

        let popup = match self.tray_popup.as_mut() {
            Some(p) if p.visible => p,
            _ => return,
        };

        // Upload any new GPU assets registered during this tick.
        if popup.host.take_assets_dirty() {
            if let Some(assets) = popup.host.active_scene_assets() {
                popup.renderer.load_assets(&popup.gpu.device, &popup.gpu.queue, assets);
            }
        }

        // Minimal render: scene instances + text only. No devtools overlay.
        let (scene_instances, _devtools_instances, clear_color) =
            popup.host.split_instances(vw, vh);

        let text_areas = if let Some(scene) = popup.host.active_scene() {
            scene.text_areas()
        } else {
            Vec::new()
        };

        popup.renderer.begin_frame(&popup.gpu, dt, scale);
        let _ = popup.renderer.render_triple_layered(
            &popup.gpu,
            &scene_instances,
            text_areas,
            &[],
            Vec::new(),
            &[],
            Vec::new(),
            clear_color,
        );
    }
}

fn popup_viewport(popup: &TrayMenuPopup) -> (f32, f32) {
    let scale = popup.window.scale_factor() as f32;
    (popup.gpu.size.0 as f32 / scale, popup.gpu.size.1 as f32 / scale)
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

        // Apply window title from <title>, fall back to opts.title.
        if let Some(title) = self.host.active_window_title() {
            window.set_title(&title);
        }

        // Apply icon from <link rel="icon"> if declared. Used for both the
        // window decoration / taskbar entry and the system tray.
        let icon_rgba = self.host.active_app_icon_rgba();
        if let Some((ref rgba, w, h)) = icon_rgba {
            match Icon::from_rgba(rgba.clone(), w, h) {
                Ok(icon) => {
                    window.set_window_icon(Some(icon.clone()));
                    #[cfg(target_os = "windows")]
                    {
                        use winit::platform::windows::WindowExtWindows;
                        window.set_taskbar_icon(Some(icon));
                    }
                }
                Err(e) => log::warn!("[prism::run] failed to build window icon: {e}"),
            }
        }

        // Initialise system tray (only fires when the TrayAccess capability
        // is declared by the embedding app).
        let tray_tooltip = self
            .host
            .active_window_title()
            .unwrap_or_else(|| self.opts.title.clone());
        // Auto-detect a custom HTML tray menu: if `tray-menu.html` exists in
        // the embedded `pages/` bundle, use it. Apps that want a different
        // path can call `AppHost::set_tray_menu_html_path` afterwards.
        let menu_html_path = if crate::embed::read_page_bytes("tray-menu.html").is_some() {
            Some("tray-menu.html".to_string())
        } else {
            None
        };
        if let Some((rgba, iw, ih)) = icon_rgba {
            let cfg = crate::tray::TrayConfig {
                enabled: true,
                tooltip: tray_tooltip,
                icon_rgba: Some((rgba, iw, ih)),
                menu_html_path: menu_html_path.clone(),
                ..crate::tray::TrayConfig::default()
            };
            self.host.init_tray_with_config(cfg);
        } else {
            let cfg = crate::tray::TrayConfig {
                enabled: true,
                tooltip: tray_tooltip,
                menu_html_path: menu_html_path.clone(),
                ..crate::tray::TrayConfig::default()
            };
            self.host.init_tray_with_config(cfg);
        }

        window.request_redraw();
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Route events for the tray-menu popup window separately from the
        // main app window. Both share this single event loop.
        if let Some(popup) = self.tray_popup.as_ref() {
            if popup.window.id() == window_id {
                self.handle_popup_event(event_loop, event);
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                // If a system tray is active, the X button hides the window
                // instead of exiting — the app keeps running in the tray and
                // the user closes it from the tray menu's "Close" item.
                if self.host.has_active_tray() {
                    if let Some(ref w) = self.window {
                        w.set_visible(false);
                        self.host.window_visible = false;
                    }
                } else {
                    event_loop.exit();
                }
            }
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
            WindowEvent::CursorEntered { .. } => {
                // Force-reapply our cursor on entry so we don't leave the OS
                // showing a stale cursor from outside the window.
                self.last_applied_cursor = None;
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
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 40.0, y * 40.0),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };
                self.dispatch_input(RawInputEvent::MouseWheel {
                    x: self.cursor_pos.0,
                    y: self.cursor_pos.1,
                    delta_x: dx,
                    delta_y: dy,
                });
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Process tray events even when the window is hidden — RedrawRequested
        // won't fire for invisible windows, so the render loop's tray handling
        // alone wouldn't catch Open/Exit clicks while minimised.
        for event in self.host.poll_tray() {
            match event {
                AppEvent::CloseRequested => {
                    event_loop.exit();
                    return;
                }
                AppEvent::TrayShowWindow => {
                    if let Some(ref w) = self.window {
                        w.set_visible(true);
                        w.focus_window();
                    }
                }
                AppEvent::TrayToggleWindow => {
                    if let Some(ref w) = self.window {
                        let visible = self.host.window_visible;
                        w.set_visible(visible);
                        if visible {
                            w.focus_window();
                        }
                    }
                }
                AppEvent::ShowCustomTrayMenu { x, y } => {
                    self.show_tray_menu(event_loop, x, y);
                }
                _ => {}
            }
        }

        // Drain a deferred tray-menu dismissal scheduled from inside an
        // event handler.
        if self
            .tray_popup
            .as_ref()
            .map(|p| p.pending_dismiss)
            .unwrap_or(false)
        {
            self.hide_tray_menu();
        }

        if let Some(ref w) = self.window {
            w.request_redraw();
        }
    }
}
