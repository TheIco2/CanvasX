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

use crate::cxrd::document::{CxrdDocument, SceneType};
use crate::compiler::html::compile_html;
use crate::compiler::editable::EditableContext;
use crate::scene::graph::SceneGraph;
use crate::scene::input_handler::{InputHandler, RawInputEvent, UiEvent};

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
    editables: Option<EditableContext>,
    /// Whether the page needs re-layout.
    dirty: bool,
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
    pub fn handle_input(&mut self, event: RawInputEvent) {
        // First, check if the event hit the sidebar / chrome area.
        // Then check if the event hit the active page.
        if let Some(ref page_id) = self.active_page.clone() {
            if let Some(page) = self.pages.get_mut(page_id) {
                let ui_events = page.input_handler.process_event(&mut page.scene.document, event);
                for ui_event in ui_events {
                    match ui_event {
                        UiEvent::NavigateRequest { scene_id } => {
                            // Check if this is a route ID or a URL.
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
                        _ => {
                            // Other events (Click, etc.) handled internally.
                        }
                    }
                }
            }
        }
    }

    /// Tick the host — update active page, process animations, return pending events.
    pub fn tick(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        dt: f32,
        font_system: &mut glyphon::FontSystem,
    ) -> Vec<AppEvent> {
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

        // Process navigation events deferred during tick.
        for event in &events {
            if let AppEvent::NavigateTo(ref page_id) = event {
                if self.active_page.as_deref() != Some(page_id) {
                    // This will be handled next frame — store intent.
                    self.active_page = Some(page_id.clone());
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

    // --- Internal ---

    fn load_page(&mut self, route: &Route) {
        let doc = match &route.source {
            PageSource::Document(d) => (**d).clone(),

            PageSource::HtmlFile(path) => {
                match load_html_document(path, &route.id) {
                    Ok(d) => d,
                    Err(e) => {
                        log::error!("AppHost: failed to load '{}': {}", route.id, e);
                        CxrdDocument::new(&route.id, SceneType::ConfigPanel)
                    }
                }
            }

            PageSource::Inline(html) => {
                match compile_html(html, "", &route.id, SceneType::ConfigPanel, None) {
                    Ok((d, _, _)) => d,
                    Err(e) => {
                        log::error!("AppHost: failed to compile inline '{}': {}", route.id, e);
                        CxrdDocument::new(&route.id, SceneType::ConfigPanel)
                    }
                }
            }

            PageSource::ProtocolUri(uri) => {
                if let Some(path) = self.resolve_protocol(uri) {
                    match load_html_document(&path, &route.id) {
                        Ok(d) => d,
                        Err(e) => {
                            log::error!("AppHost: protocol load failed for '{}': {}", uri, e);
                            CxrdDocument::new(&route.id, SceneType::ConfigPanel)
                        }
                    }
                } else {
                    log::error!("AppHost: unresolvable URI '{}'", uri);
                    CxrdDocument::new(&route.id, SceneType::ConfigPanel)
                }
            }

            PageSource::External(url) => {
                // External URLs are opened in the system browser, not loaded.
                self.pending_events.push(AppEvent::OpenExternal(url.clone()));
                CxrdDocument::new(&route.id, SceneType::ConfigPanel)
            }
        };

        let scene = SceneGraph::new(doc);
        let input_handler = InputHandler::new();

        self.pages.insert(route.id.clone(), PageInstance {
            scene,
            input_handler,
            editables: None,
            dirty: true,
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

fn load_html_document(path: &Path, name: &str) -> Result<CxrdDocument, String> {
    let html = std::fs::read_to_string(path)
        .map_err(|e| format!("Read error: {}", e))?;

    // Look for sibling CSS.
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
        .map(|(doc, _, _)| doc)
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
