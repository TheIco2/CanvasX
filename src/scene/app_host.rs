// canvasx-runtime/src/scene/app_host.rs
//
// App Host — manages a CanvasX-powered application window with interactive
// multi-page navigation, sidebar, tabs, and document embedding.
//
// This replaces the wry/WebView2 config UI shell in Sentinel Core by providing
// native scene-graph equivalents of:
//   - Window chrome (title bar, close/min/max buttons)
//   - Sidebar navigation
//   - Tabbed document areas
//   - Embedded sub-documents (analogous to iframes)
//   - Custom protocol routing (e.g. sentinel:// → local filesystem)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::capabilities::CapabilitySet;
use crate::compiler::css::CssRule;
use crate::compiler::html::{compile_html, ScriptBlock};
use crate::compiler::editable::EditableContext;
use crate::cxrd::document::{CxrdDocument, SceneType};
use crate::cxrd::node::NodeId;
use crate::cxrd::value::Color;
use crate::devtools::DevTools;
use crate::devtools::context_menu::ContextAction;
use crate::gpu::vertex::UiInstance;
use crate::scene::graph::SceneGraph;
use crate::scene::input_handler::{InputHandler, RawInputEvent, UiEvent, MouseButton as CxMouseButton};
use crate::scripting::JsRuntime;
use crate::tray::{SystemTray, TrayConfig, TrayEvent};

/// Unique page identifier.
pub type PageId = String;

/// A registered navigation route.
#[derive(Clone, Debug)]
pub struct Route {
    /// Unique route identifier (e.g. "general", "addons", "about").
    pub id: PageId,
    /// Display label for sidebar / tab.
    pub label: String,
    /// Optional icon (path to SVG or image asset).
    pub icon: Option<String>,
    /// Source for this page.
    pub source: PageSource,
    /// Whether this route represents a separator in the sidebar.
    pub separator: bool,
}

/// Where a page's content comes from.
#[derive(Clone, Debug)]
pub enum PageSource {
    /// Pre-compiled CxrdDocument.
    Document(Arc<CxrdDocument>),
    /// HTML file path to compile on demand.
    HtmlFile(PathBuf),
    /// Inline HTML string.
    Inline(String),
    /// Custom protocol URI (e.g. sentinel://wallpaper/options/options.html).
    ProtocolUri(String),
    /// External URL (opened in system browser, not embedded).
    External(String),
}

/// Protocol handler for custom URI schemes (e.g. sentinel://).
pub trait ProtocolHandler: Send + Sync {
    /// Resolve a protocol URI to a local file path.
    fn resolve(&self, uri: &str) -> Option<PathBuf>;
}

/// A simple protocol handler that maps a scheme to a base directory.
/// e.g., `sentinel://wallpaper/options/options.html`
///        → `~/.Sentinel/Assets/wallpaper/options/options.html`
pub struct FileSystemProtocol {
    pub scheme: String,
    pub base_dir: PathBuf,
}

impl ProtocolHandler for FileSystemProtocol {
    fn resolve(&self, uri: &str) -> Option<PathBuf> {
        let prefix = format!("{}://", self.scheme);
        if !uri.starts_with(&prefix) {
            return None;
        }
        let rel = &uri[prefix.len()..];
        let path = self.base_dir.join(rel);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
}

/// Page instance — a loaded, renderable page.
struct PageInstance {
    /// The SceneGraph for this page.
    scene: SceneGraph,
    /// Input handler for this page.
    input_handler: InputHandler,
    /// Optional editables.
    #[allow(dead_code)]
    editables: Option<EditableContext>,
    /// Whether the page needs re-layout.
    dirty: bool,
    /// Scripts collected from HTML compilation (for JS init).
    scripts: Vec<ScriptBlock>,
    /// CSS rules from compilation (for JS runtime).
    css_rules: Vec<CssRule>,
    /// Source directory for resolving external script paths.
    source_dir: Option<PathBuf>,
}

/// The main application host — orchestrates multi-page navigation,
/// sidebar, and document embedding.
pub struct AppHost {
    /// Registered routes (in display order).
    routes: Vec<Route>,
    /// Loaded page instances, keyed by page ID.
    pages: HashMap<PageId, PageInstance>,
    /// Currently visible page.
    active_page: Option<PageId>,
    /// Navigation history.
    history: Vec<PageId>,
    /// Forward stack (pages navigated away from via back()).
    forward_stack: Vec<PageId>,
    /// Protocol handlers for custom URI schemes.
    protocol_handlers: Vec<Box<dyn ProtocolHandler>>,
    /// App-level input handler (for sidebar, title bar, etc.).
    #[allow(dead_code)]
    chrome_input_handler: InputHandler,
    /// Sidebar width in pixels.
    pub sidebar_width: f32,
    /// Whether the sidebar is visible.
    pub sidebar_visible: bool,
    /// Title of the application window.
    pub title: String,
    /// Pending UI events from user interaction.
    pending_events: Vec<AppEvent>,
    /// Data shared across all pages (e.g., from Sentinel IPC).
    shared_data: Arc<Mutex<HashMap<String, String>>>,
    /// Declared runtime capabilities for this application.
    pub capabilities: CapabilitySet,
    /// Built-in DevTools (CanvasX badge + developer panel + context menu).
    pub devtools: DevTools,
    /// JavaScript runtime (shared, reinitialised on page navigation).
    js_runtime: Option<JsRuntime>,
    /// System tray icon and menu (created from capabilities).
    system_tray: Option<SystemTray>,
    /// Whether the window is currently visible (for tray hide/show).
    pub window_visible: bool,
    /// Pending context action from right-click menu.
    pending_context_action: Option<ContextAction>,
    /// Map from canvas CanvasId → GPU texture slot.
    canvas_texture_slots: HashMap<u32, u32>,
    /// Map from NodeId → CanvasId (mirrors JS runtime).
    node_canvas_map: HashMap<NodeId, u32>,
    /// Next available GPU texture slot for canvas textures.
    next_canvas_slot: u32,
}

/// High-level application events.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// User navigated to a page.
    NavigateTo(PageId),
    /// User clicked a link.
    LinkClicked { url: String, page: PageId },
    /// User changed an editable value.
    EditableChanged { variable: String, value: String, page: PageId },
    /// An IPC command was triggered from an interactive element.
    IpcCommand { ns: String, cmd: String, args: Option<serde_json::Value> },
    /// User requested to close the window.
    CloseRequested,
    /// User requested window minimize.
    MinimizeRequested,
    /// User requested window maximize toggle.
    MaximizeToggleRequested,
    /// Open external URL in system browser.
    OpenExternal(String),
    /// Tray: toggle window visibility.
    TrayToggleWindow,
    /// Tray: custom action fired.
    TrayAction(String),
}

impl AppHost {
    /// Create a new application host.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            routes: Vec::new(),
            pages: HashMap::new(),
            active_page: None,
            history: Vec::new(),
            forward_stack: Vec::new(),
            protocol_handlers: Vec::new(),
            chrome_input_handler: InputHandler::new(),
            sidebar_width: 240.0,
            sidebar_visible: true,
            title: title.into(),
            pending_events: Vec::new(),
            shared_data: Arc::new(Mutex::new(HashMap::new())),
            capabilities: CapabilitySet::new(),
            devtools: DevTools::new(),
            js_runtime: None,
            system_tray: None,
            window_visible: true,
            pending_context_action: None,
            canvas_texture_slots: HashMap::new(),
            node_canvas_map: HashMap::new(),
            next_canvas_slot: 10000,
        }
    }

    /// Register a navigation route.
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }

    /// Add multiple routes.
    pub fn add_routes(&mut self, routes: impl IntoIterator<Item = Route>) {
        self.routes.extend(routes);
    }

    /// Register a custom protocol handler (e.g., for sentinel://).
    pub fn add_protocol_handler(&mut self, handler: Box<dyn ProtocolHandler>) {
        self.protocol_handlers.push(handler);
    }

    /// Set the declared runtime capabilities.
    pub fn set_capabilities(&mut self, caps: CapabilitySet) {
        self.devtools.has_network = caps.has_network();
        self.capabilities = caps;
    }

    /// Get a reference to the shared data store.
    pub fn shared_data(&self) -> Arc<Mutex<HashMap<String, String>>> {
        self.shared_data.clone()
    }

    /// Update shared data (called by the IPC bridge each frame).
    pub fn push_data(&self, data: HashMap<String, String>) {
        if let Ok(mut store) = self.shared_data.lock() {
            *store = data;
        }
    }

    /// Navigate to a page by ID.
    pub fn navigate_to(&mut self, page_id: &str) {
        // Save current page to history.
        if let Some(ref current) = self.active_page {
            self.history.push(current.clone());
        }
        self.forward_stack.clear();

        // Ensure page is loaded.
        if !self.pages.contains_key(page_id) {
            if let Some(route) = self.routes.iter().find(|r| r.id == page_id).cloned() {
                self.load_page(&route);
            }
        }

        // Tear down old JS runtime so the consumer can call init_js_for_active_page.
        self.js_runtime = None;

        self.active_page = Some(page_id.to_string());
        self.pending_events.push(AppEvent::NavigateTo(page_id.to_string()));

        log::info!("AppHost: navigated to '{}'", page_id);
    }

    /// Navigate back.
    pub fn navigate_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            if let Some(current) = self.active_page.take() {
                self.forward_stack.push(current);
            }
            self.active_page = Some(prev);
        }
    }

    /// Navigate forward.
    pub fn navigate_forward(&mut self) {
        if let Some(next) = self.forward_stack.pop() {
            if let Some(current) = self.active_page.take() {
                self.history.push(current);
            }
            self.active_page = Some(next);
        }
    }

    /// Get the active page ID.
    pub fn active_page(&self) -> Option<&str> {
        self.active_page.as_deref()
    }

    /// Get the list of registered routes.
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// Process a raw input event.
    /// Handles DevTools badge/context menu interception before routing to the page.
    pub fn handle_input(&mut self, event: RawInputEvent, viewport_width: f32, viewport_height: f32) {
        // Track mouse position for context menu hover.
        if let RawInputEvent::MouseMove { x, y } = &event {
            self.devtools.context_menu.update_hover(*x, *y);
        }

        // Intercept left-clicks for DevTools badge, context menu, and panel.
        if let RawInputEvent::MouseDown { button: CxMouseButton::Left, .. }
            | RawInputEvent::MouseUp { button: CxMouseButton::Left, .. } = &event
        {
            if let RawInputEvent::MouseDown { .. } = &event {
                // Get mouse position from the active page's input handler.
                let (x, y) = if let Some(page) = self.active_page.as_ref()
                    .and_then(|id| self.pages.get(id))
                {
                    page.input_handler.mouse_pos
                } else {
                    (0.0, 0.0)
                };

                // Context menu: if open, handle click (action or dismiss).
                if self.devtools.context_menu.open {
                    if let Some(action) = self.devtools.context_menu.click(x, y) {
                        self.pending_context_action = Some(action);
                    }
                    return;
                }

                // Badge click → toggle DevTools.
                if self.devtools.hit_test_badge(x, y, viewport_height) {
                    self.devtools.toggle();
                    return;
                }

                // Tab click → switch tab.
                if let Some(tab) = self.devtools.hit_test_tab(x, y, viewport_width, viewport_height) {
                    self.devtools.active_tab = tab;
                    return;
                }

                // Block clicks inside the DevTools panel from reaching the page.
                if self.devtools.hit_test_panel(x, y, viewport_height) {
                    return;
                }
            }
        }

        // Intercept right-clicks → show context menu.
        if let RawInputEvent::MouseDown { button: CxMouseButton::Right, .. } = &event {
            let (x, y) = if let Some(page) = self.active_page.as_ref()
                .and_then(|id| self.pages.get(id))
            {
                page.input_handler.mouse_pos
            } else {
                (0.0, 0.0)
            };
            self.devtools.context_menu.show(x, y, viewport_width, viewport_height);
            return;
        }

        // Intercept scroll for DevTools panel.
        if let RawInputEvent::MouseWheel { delta_y, .. } = &event {
            let (_, y) = if let Some(page) = self.active_page.as_ref()
                .and_then(|id| self.pages.get(id))
            {
                page.input_handler.mouse_pos
            } else {
                (0.0, 0.0)
            };
            if self.devtools.hit_test_panel(0.0, y, viewport_height) {
                self.devtools.handle_scroll(*delta_y);
                return;
            }
        }

        // Intercept Escape → dismiss context menu.
        if let RawInputEvent::KeyDown { key: crate::scene::input_handler::KeyCode::Escape, .. } = &event {
            if self.devtools.context_menu.open {
                self.devtools.context_menu.hide();
                return;
            }
        }

        // Route event to active page's input handler.
        if let Some(ref page_id) = self.active_page.clone() {
            if let Some(page) = self.pages.get_mut(page_id) {
                let ui_events = page.input_handler.process_event(&mut page.scene.document, event);
                let mut click_node_ids: Vec<u32> = Vec::new();

                for ui_event in ui_events {
                    match ui_event {
                        UiEvent::NavigateRequest { scene_id } => {
                            if self.routes.iter().any(|r| r.id == scene_id) {
                                self.pending_events.push(AppEvent::NavigateTo(scene_id));
                            } else {
                                self.pending_events.push(AppEvent::LinkClicked {
                                    url: scene_id,
                                    page: page_id.clone(),
                                });
                            }
                        }
                        UiEvent::OpenExternal { url } => {
                            self.pending_events.push(AppEvent::OpenExternal(url));
                        }
                        UiEvent::IpcCommand { ns, cmd, args } => {
                            self.pending_events.push(AppEvent::IpcCommand { ns, cmd, args });
                        }
                        UiEvent::ValueChanged { node_id, value, .. } => {
                            self.pending_events.push(AppEvent::EditableChanged {
                                variable: node_id.to_string(),
                                value,
                                page: page_id.clone(),
                            });
                        }
                        UiEvent::Click { node_id } => {
                            click_node_ids.push(node_id);
                        }
                        _ => {}
                    }
                }

                // Forward click events to JS runtime.
                if !click_node_ids.is_empty() {
                    if let Some(ref mut js_rt) = self.js_runtime {
                        for nid in &click_node_ids {
                            js_rt.dispatch_dom_event(*nid, "click");
                        }
                    }
                    page.scene.invalidate_layout();
                }
            }
        }
    }

    /// Tick the host — update active page, JS runtime, tray events, return pending events.
    pub fn tick(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        dt: f32,
        font_system: &mut glyphon::FontSystem,
    ) -> Vec<AppEvent> {
        // Handle pending context menu actions.
        if let Some(action) = self.pending_context_action.take() {
            match action {
                ContextAction::ToggleDevTools => {
                    self.devtools.toggle();
                }
                ContextAction::Reload => {
                    self.reload_active_page();
                }
                ContextAction::Exit => {
                    self.pending_events.push(AppEvent::CloseRequested);
                }
            }
        }

        // Poll system tray events.
        if let Some(ref tray) = self.system_tray {
            for event in tray.poll_events() {
                match event {
                    TrayEvent::ShowWindow | TrayEvent::ToggleWindow => {
                        self.window_visible = !self.window_visible;
                        self.pending_events.push(AppEvent::TrayToggleWindow);
                    }
                    TrayEvent::Exit => {
                        self.pending_events.push(AppEvent::CloseRequested);
                    }
                    TrayEvent::Reload => {
                        self.reload_active_page();
                    }
                    TrayEvent::CustomAction(id) => {
                        self.pending_events.push(AppEvent::TrayAction(id));
                    }
                }
            }
        }

        // Tick JS runtime (requestAnimationFrame, timers, etc.).
        if let Some(ref mut js_rt) = self.js_runtime {
            js_rt.gc_gradients();
            let _js_dirty = js_rt.tick(dt);

            // Drain JS console messages into DevTools.
            for (level, msg) in js_rt.drain_console() {
                let log_level = match level {
                    0 => crate::devtools::console::LogLevel::Log,
                    2 => crate::devtools::console::LogLevel::Warn,
                    3 => crate::devtools::console::LogLevel::Error,
                    _ => crate::devtools::console::LogLevel::Info,
                };
                self.devtools.console.log(log_level, msg);
            }

            // If JS modified the DOM, sync document back to scene.
            if js_rt.take_layout_dirty() {
                if let Some(ref page_id) = self.active_page {
                    if let Some(page) = self.pages.get_mut(page_id) {
                        let new_doc = js_rt.document();
                        page.scene.load_document(new_doc.clone());
                        drop(new_doc);
                    }
                }
            }
        }

        // Sync shared data into active page.
        if let Some(ref page_id) = self.active_page {
            if let Some(page) = self.pages.get_mut(page_id) {
                if let Ok(data) = self.shared_data.lock() {
                    page.scene.update_data_batch(data.clone());
                }

                // Calculate content area (subtract sidebar).
                let content_x = if self.sidebar_visible { self.sidebar_width } else { 0.0 };
                let content_width = viewport_width - content_x;

                if page.dirty {
                    page.scene.invalidate_layout();
                    page.dirty = false;
                }

                let _ = page.scene.tick(content_width, viewport_height, dt, font_system);
            }
        }

        // Drain pending events.
        let events = std::mem::take(&mut self.pending_events);

        // Navigate to pending pages (from handle_input click events).
        for event in &events {
            if let AppEvent::NavigateTo(ref page_id) = event {
                if self.active_page.as_deref() != Some(page_id) {
                    // Will be handled by the consuming app calling navigate_to.
                }
            }
        }

        events
    }

    /// Drain app events without ticking.
    pub fn drain_events(&mut self) -> Vec<AppEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Get the active page's scene (for rendering).
    pub fn active_scene(&self) -> Option<&SceneGraph> {
        self.active_page
            .as_ref()
            .and_then(|id| self.pages.get(id))
            .map(|p| &p.scene)
    }

    /// Get the active page's scene mutably.
    pub fn active_scene_mut(&mut self) -> Option<&mut SceneGraph> {
        let page_id = self.active_page.clone()?;
        self.pages.get_mut(&page_id).map(|p| &mut p.scene)
    }

    /// Initialise the system tray icon (call once after window creation).
    /// Only creates the tray if TrayAccess capability is declared.
    pub fn init_tray(&mut self, tooltip: &str) {
        if !self.capabilities.has_tray() {
            return;
        }
        let tray_config = TrayConfig {
            enabled: true,
            tooltip: tooltip.to_string(),
            ..TrayConfig::default()
        };
        self.system_tray = Some(SystemTray::new(&tray_config));
        log::info!("System tray created for '{}'", tooltip);
    }

    /// Initialise the JS runtime for the current active page.
    /// Call this once after window creation and after each navigation.
    pub fn init_js_for_active_page(&mut self, viewport_width: u32, viewport_height: u32) {
        let page_id = match self.active_page.clone() {
            Some(id) => id,
            None => return,
        };
        let page = match self.pages.get(&page_id) {
            Some(p) => p,
            None => return,
        };

        let doc = page.scene.document.clone();
        let css_variables: HashMap<String, String> = doc.variables.iter().cloned().collect();
        let css_rules = page.css_rules.clone();
        let scripts = page.scripts.clone();
        let source_dir = page.source_dir.clone();

        let mut js_rt = JsRuntime::new(doc, css_rules, css_variables);
        js_rt.init_canvases(viewport_width, viewport_height);

        // Execute page scripts.
        for script in &scripts {
            if let Some(ref src) = script.src {
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

        // Fire DOMContentLoaded.
        js_rt.execute(
            r#"(function(){
                if(typeof __cx_globalListeners==='object' && __cx_globalListeners['DOMContentLoaded']){
                    var fns=__cx_globalListeners['DOMContentLoaded'].slice();
                    for(var i=0;i<fns.length;i++){try{fns[i]({type:'DOMContentLoaded'});}catch(e){console.error('DOMContentLoaded handler error:',e);}}
                }
            })();"#,
            "<DOMContentLoaded>",
        );

        js_rt.cache_raf_tick_fn();
        self.canvas_texture_slots.clear();
        self.node_canvas_map.clear();
        self.next_canvas_slot = 10000;
        self.js_runtime = Some(js_rt);
    }

    /// Store GPU adapter info for DevTools GPU tab.
    pub fn set_gpu_info(&mut self, info: String) {
        self.devtools.gpu_info = info;
    }

    /// Reload the currently active page (re-compile HTML/CSS + re-init JS).
    pub fn reload_active_page(&mut self) {
        let page_id = match self.active_page.clone() {
            Some(id) => id,
            None => return,
        };
        let route = match self.routes.iter().find(|r| r.id == page_id).cloned() {
            Some(r) => r,
            None => return,
        };
        log::info!("Reloading page '{}'", page_id);
        self.pages.remove(&page_id);
        self.load_page(&route);
        self.js_runtime = None;
        self.devtools.console.entries.clear();
    }

    /// Get combined paint output: scene instances + DevTools overlay.
    /// Returns (instances, clear_color).
    pub fn combined_instances(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
    ) -> (Vec<UiInstance>, Color) {
        let (scene_instances, clear_color): (Vec<UiInstance>, Color) = if let Some(scene) = self.active_scene() {
            let instances = scene.cached_instances.clone();
            let bg = scene.document.background;
            (instances, bg)
        } else {
            (Vec::new(), Color::BLACK)
        };

        // Patch canvas instances with GPU texture slot.
        let patched: Vec<UiInstance> = scene_instances.iter().map(|inst| {
            if inst.texture_index <= -2
                && (inst.flags & UiInstance::FLAG_HAS_TEXTURE) != 0
            {
                let node_id = (-inst.texture_index - 2) as u32;
                if let Some(&slot) = self.node_canvas_map.get(&node_id) {
                    let mut p = *inst;
                    p.texture_index = slot as i32;
                    return p;
                }
            }
            *inst
        }).collect();

        // Update DevTools stats.
        self.devtools.instance_count = patched.len() as u32;

        // Get the document for DevTools paint.
        let doc_for_devtools = if let Some(scene) = self.active_scene() {
            scene.document.clone()
        } else {
            CxrdDocument::new("empty", SceneType::ConfigPanel)
        };

        // Append DevTools overlay instances.
        let devtools_instances = self.devtools.paint(&doc_for_devtools, viewport_width, viewport_height);
        let mut combined = patched;
        combined.extend(devtools_instances);

        self.devtools.draw_calls = combined.len() as u32;

        (combined, clear_color)
    }

    /// Get DevTools text entries for rendering alongside scene text.
    pub fn devtools_text_entries(
        &self,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<crate::devtools::DevToolsTextEntry> {
        let doc = if let Some(scene) = self.active_scene() {
            &scene.document
        } else {
            return self.devtools.text_entries(
                &CxrdDocument::new("empty", SceneType::ConfigPanel),
                viewport_width,
                viewport_height,
            );
        };
        self.devtools.text_entries(doc, viewport_width, viewport_height)
    }

    /// Get dirty canvas textures from JS runtime for GPU upload.
    pub fn dirty_canvases(&self) -> Vec<(u32, Option<NodeId>, u32, u32, Vec<u8>)> {
        if let Some(ref js_rt) = self.js_runtime {
            js_rt.dirty_canvases()
        } else {
            Vec::new()
        }
    }

    /// Mark canvases clean and update node→slot mapping.
    pub fn commit_canvas_uploads(&mut self) {
        if let Some(ref mut js_rt) = self.js_runtime {
            js_rt.clear_dirty_flags();
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
    }

    /// Get or assign a GPU texture slot for a canvas.
    pub fn canvas_slot(&mut self, canvas_id: u32) -> u32 {
        *self.canvas_texture_slots.entry(canvas_id).or_insert_with(|| {
            let s = self.next_canvas_slot;
            self.next_canvas_slot += 1;
            s
        })
    }

    /// Whether this host has system tray active (for close behavior).
    pub fn has_active_tray(&self) -> bool {
        self.system_tray.as_ref().map_or(false, |t| t.is_active())
    }

    // --- Internal ---

    fn load_page(&mut self, route: &Route) {
        let (doc, scripts, css_rules, source_dir) = match &route.source {
            PageSource::Document(d) => ((**d).clone(), Vec::new(), Vec::new(), None),

            PageSource::HtmlFile(path) => {
                match load_html_document_full(path, &route.id) {
                    Ok((d, s, r)) => {
                        let dir = path.parent().map(|p| p.to_path_buf());
                        (d, s, r, dir)
                    }
                    Err(e) => {
                        log::error!("AppHost: failed to load '{}': {}", route.id, e);
                        (CxrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                    }
                }
            }

            PageSource::Inline(html) => {
                let css = extract_inline_styles(html);
                match compile_html(html, &css, &route.id, SceneType::ConfigPanel, None) {
                    Ok((d, s, r)) => (d, s, r, None),
                    Err(e) => {
                        log::error!("AppHost: failed to compile inline '{}': {}", route.id, e);
                        (CxrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                    }
                }
            }

            PageSource::ProtocolUri(uri) => {
                if let Some(path) = self.resolve_protocol(uri) {
                    match load_html_document_full(&path, &route.id) {
                        Ok((d, s, r)) => {
                            let dir = path.parent().map(|p| p.to_path_buf());
                            (d, s, r, dir)
                        }
                        Err(e) => {
                            log::error!("AppHost: protocol load failed for '{}': {}", uri, e);
                            (CxrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                        }
                    }
                } else {
                    log::error!("AppHost: unresolvable URI '{}'", uri);
                    (CxrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                }
            }

            PageSource::External(url) => {
                self.pending_events.push(AppEvent::OpenExternal(url.clone()));
                (CxrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
            }
        };

        let scene = SceneGraph::new(doc);
        let input_handler = InputHandler::new();

        self.pages.insert(route.id.clone(), PageInstance {
            scene,
            input_handler,
            editables: None,
            dirty: true,
            scripts,
            css_rules,
            source_dir,
        });
    }

    fn resolve_protocol(&self, uri: &str) -> Option<PathBuf> {
        for handler in &self.protocol_handlers {
            if let Some(path) = handler.resolve(uri) {
                return Some(path);
            }
        }
        None
    }
}

// --- Helpers ---

/// Extract the content of all `<style>…</style>` blocks from an HTML string.
fn extract_inline_styles(html: &str) -> String {
    let mut css = String::new();
    let lower = html.to_lowercase();
    let mut search_start = 0;
    while let Some(open) = lower[search_start..].find("<style") {
        let abs_open = search_start + open;
        // Find the end of the opening tag '>'
        if let Some(gt) = lower[abs_open..].find('>') {
            let content_start = abs_open + gt + 1;
            if let Some(close) = lower[content_start..].find("</style>") {
                let content_end = content_start + close;
                css.push_str(&html[content_start..content_end]);
                css.push('\n');
                search_start = content_end + "</style>".len();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    css
}

fn load_html_document_full(
    path: &Path,
    name: &str,
) -> Result<(CxrdDocument, Vec<ScriptBlock>, Vec<CssRule>), String> {
    let html = std::fs::read_to_string(path)
        .map_err(|e| format!("Read error: {}", e))?;

    let css_path = path.with_extension("css");
    let css = if css_path.exists() {
        std::fs::read_to_string(&css_path).unwrap_or_default()
    } else {
        let sibling = path.parent().map(|p| p.join("style.css")).unwrap_or_default();
        if sibling.exists() {
            std::fs::read_to_string(sibling).unwrap_or_default()
        } else {
            String::new()
        }
    };

    compile_html(&html, &css, name, SceneType::ConfigPanel, path.parent())
        .map_err(|e| e.to_string())
}

/// Builder for constructing an AppHost with a common Sentinel-style layout
/// (sidebar + content area, with routes for each addon's options page).
pub struct SentinelAppBuilder {
    host: AppHost,
    sentinel_base: Option<PathBuf>,
}

impl SentinelAppBuilder {
    /// Create a new Sentinel app builder.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            host: AppHost::new(title),
            sentinel_base: None,
        }
    }

    /// Set the Sentinel base directory (e.g., `~/.Sentinel`).
    pub fn sentinel_base(mut self, path: impl Into<PathBuf>) -> Self {
        let base: PathBuf = path.into();
        // Register sentinel:// protocol handler.
        self.host.add_protocol_handler(Box::new(FileSystemProtocol {
            scheme: "sentinel".into(),
            base_dir: base.clone(),
        }));
        self.sentinel_base = Some(base);
        self
    }

    /// Set sidebar width.
    pub fn sidebar_width(mut self, width: f32) -> Self {
        self.host.sidebar_width = width;
        self
    }

    /// Declare runtime capabilities for this application.
    pub fn capabilities(mut self, caps: CapabilitySet) -> Self {
        self.host.capabilities = caps;
        self
    }

    /// Add a built-in page (inline HTML or file).
    pub fn add_page(mut self, id: &str, label: &str, source: PageSource) -> Self {
        self.host.add_route(Route {
            id: id.into(),
            label: label.into(),
            icon: None,
            source,
            separator: false,
        });
        self
    }

    /// Add a separator in the sidebar.
    pub fn add_separator(mut self) -> Self {
        self.host.add_route(Route {
            id: format!("__sep_{}", self.host.routes.len()),
            label: String::new(),
            icon: None,
            source: PageSource::Inline(String::new()),
            separator: true,
        });
        self
    }

    /// Auto-discover addon options pages from the Sentinel assets directory.
    pub fn discover_addon_options(mut self) -> Self {
        if let Some(ref base) = self.sentinel_base {
            let addons_dir = base.join("Assets");
            if addons_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&addons_dir) {
                    for entry in entries.flatten() {
                        let addon_dir = entry.path();
                        if !addon_dir.is_dir() {
                            continue;
                        }
                        // Look for options/options.html in each addon.
                        let options_html = addon_dir.join("options").join("options.html");
                        if options_html.exists() {
                            let addon_name = addon_dir
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("unknown")
                                .to_string();

                            let label = format_addon_label(&addon_name);

                            self.host.add_route(Route {
                                id: format!("addon_{}", addon_name),
                                label,
                                icon: None,
                                source: PageSource::HtmlFile(options_html),
                                separator: false,
                            });
                        }
                    }
                }
            }
        }
        self
    }

    /// Build the AppHost, optionally navigating to the first page.
    pub fn build(mut self) -> AppHost {
        // Navigate to first non-separator route.
        if let Some(first) = self.host.routes.iter().find(|r| !r.separator).cloned() {
            self.host.navigate_to(&first.id);
        }
        self.host
    }
}

fn format_addon_label(raw_name: &str) -> String {
    raw_name
        .replace('-', " ")
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + &chars.collect::<String>(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
