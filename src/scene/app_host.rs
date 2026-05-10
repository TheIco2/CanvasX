// prism-runtime/src/scene/app_host.rs
//
// App Host — manages a OpenRender-powered application window with interactive
// multi-page navigation, sidebar, tabs, and document embedding.
//
// This replaces the wry/WebView2 config UI shell in OpenDesktop Core by providing
// native scene-graph equivalents of:
//   - Window chrome (title bar, close/min/max buttons)
//   - Sidebar navigation
//   - Tabbed document areas
//   - Embedded sub-documents (analogous to iframes)
//   - Custom protocol routing (e.g. opendesktop:// → local filesystem)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::capabilities::CapabilitySet;
use crate::env::Environment;
use crate::compiler::css::CssRule;
use crate::compiler::html::{compile_html, ScriptBlock};
use crate::compiler::editable::EditableContext;
use crate::prd::document::{PrdDocument, SceneType};
use crate::prd::node::NodeId;
use crate::prd::value::Color;
use crate::devtools::DevTools;
use crate::devtools::context_menu::ContextAction;
use crate::devtools::debug_server::DebugServer;
use crate::gpu::vertex::UiInstance;
use crate::instance::InstanceGuard;
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
    /// Pre-compiled PrdDocument.
    Document(Arc<PrdDocument>),
    /// HTML file path to compile on demand.
    HtmlFile(PathBuf),
    /// Embedded HTML page identified by its path within the binary's
    /// `pages/` directory (e.g. `"base.html"`, `"sub/page.html"`).
    /// Sibling CSS is also looked up in the embedded tree. JS is not
    /// loaded from disk for embedded pages — users cannot modify HTML/JS,
    /// only theme CSS overrides apply.
    Embedded(String),
    /// Inline HTML string.
    Inline(String),
    /// Custom protocol URI (e.g. opendesktop://wallpaper/options/options.html).
    ProtocolUri(String),
    /// External URL (opened in system browser, not embedded).
    External(String),
}

/// Protocol handler for custom URI schemes (e.g. opendesktop://).
pub trait ProtocolHandler: Send + Sync {
    /// Resolve a protocol URI to a local file path.
    fn resolve(&self, uri: &str) -> Option<PathBuf>;
}

/// A simple protocol handler that maps a scheme to a base directory.
/// e.g., `opendesktop://wallpaper/options/options.html`
///        → `~/ProjectOpen/OpenDesktop/Assets/wallpaper/options/options.html`
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
    /// Optional source directory used as a base for relative paths inside
    /// the loaded HTML scene (e.g. `<link href="theme.css">`).
    source_dir: Option<PathBuf>,
    /// For pages loaded from the embedded `pages/` bundle, the directory
    /// (relative to the bundle root) the page lives in. Used by
    /// `swap_page_content` to resolve sibling fragment files like
    /// `settings.html` for `<page-content>` swaps when there's no
    /// filesystem `source_dir`.
    embed_base: Option<String>,
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
    /// Data shared across all pages (e.g., from OpenDesktop IPC).
    shared_data: Arc<Mutex<HashMap<String, String>>>,
    /// Declared runtime capabilities for this application.
    pub capabilities: CapabilitySet,
    /// Built-in DevTools (OpenRender badge + developer panel + context menu).
    pub devtools: DevTools,
    /// JavaScript runtime (shared, reinitialised on page navigation).
    js_runtime: Option<JsRuntime>,
    /// System tray icon and menu (created from capabilities).
    system_tray: Option<SystemTray>,
    /// Path to the custom tray menu HTML file (when configured). Used by
    /// the runner to spawn a frameless popup window on right-click.
    tray_menu_html_path: Option<String>,
    /// Whether the window is currently visible (for tray hide/show).
    pub window_visible: bool,
    /// Pending context action from right-click menu.
    pending_context_action: Option<ContextAction>,
    /// Route id treated as the "Home" page for the right-click menu.
    home_route: Option<PageId>,
    /// Map from canvas CanvasId → GPU texture slot.
    canvas_texture_slots: HashMap<u32, u32>,
    /// Map from NodeId → CanvasId (mirrors JS runtime).
    node_canvas_map: HashMap<NodeId, u32>,
    /// Next available GPU texture slot for canvas textures.
    next_canvas_slot: u32,
    /// Debug web server for browser-based HTML/CSS inspection.
    debug_server: DebugServer,
    /// Optional custom title bar scene (compiled from title-bar.html).
    title_bar: Option<TitleBarInstance>,
    /// Whether a custom title bar was detected (signals to disable native decorations).
    pub has_custom_title_bar: bool,
    /// Single-instance guard (holds mutex + pipe listener). Only present when
    /// `SingleInstance` capability is declared and the lock was acquired.
    instance_guard: Option<InstanceGuard>,
    /// Set when new image assets need uploading to the GPU (e.g. dynamic SVGs).
    assets_dirty: bool,
    /// Active theme CSS — injected into every page that gets compiled.
    /// `None` means no themed overrides.
    theme_css: Option<String>,
    /// Surface kind. Drives capability defaults and `prism.env`.
    pub environment: Environment,
    /// Extra CSS injected into every loaded page (after `theme_css`).
    /// Populated by [`AppHost::inject_css`]; survives navigation.
    injected_css: Vec<String>,
    /// Extra JavaScript executed once per page load, after the page's own
    /// scripts. Populated by [`AppHost::inject_js`].
    injected_js: Vec<String>,
}

/// A compiled custom title bar scene.
struct TitleBarInstance {
    scene: SceneGraph,
    input_handler: InputHandler,
    /// Height of the title bar in pixels.
    height: f32,
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
    /// Tray: always show the window.
    TrayShowWindow,
    /// Tray: toggle window visibility.
    TrayToggleWindow,
    /// Tray: user toggled "Run at startup".
    TrayToggleAutostart,
    /// Tray: user clicked "Check for update".
    TrayCheckForUpdate,
    /// Tray: user right-clicked the tray icon and a custom HTML menu is
    /// configured — the host runner should pop up its tray menu window at
    /// the physical screen position `(x, y)`.
    ShowCustomTrayMenu { x: f64, y: f64 },
    /// Tray: custom action fired.
    TrayAction(String),
    /// Content was swapped inside a `<page-content>` container.
    ContentSwap { page: PageId, content_id: String },
    /// A content fragment was swapped inside `<page-content>`.
    /// Emitted after the swap completes so consuming apps can populate data.
    ContentSwapped { content_id: String },
    /// The active page was reloaded (HTML/CSS recompiled, JS runtime dropped).
    /// Consumer should call `init_js_for_active_page()`.
    PageReloaded(PageId),
    /// Update the window title.
    SetTitle(String),
    /// Window drag requested (from custom title bar).
    WindowDragRequested,
}

impl AppHost {
    /// Create a new application host.
    pub fn new(title: impl Into<String>) -> Self {
        Self::with_environment(title, Environment::Application)
    }

    /// Create a new application host targeting a specific [`Environment`].
    /// The environment seeds capability defaults, controls tray availability,
    /// and is exposed to scripts as `prism.env`.
    pub fn with_environment(title: impl Into<String>, env: Environment) -> Self {
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
            capabilities: env.default_capabilities(),
            devtools: DevTools::new(),
            js_runtime: None,
            system_tray: None,
            tray_menu_html_path: None,
            window_visible: true,
            pending_context_action: None,
            home_route: None,
            canvas_texture_slots: HashMap::new(),
            node_canvas_map: HashMap::new(),
            next_canvas_slot: 10000,
            debug_server: DebugServer::new(),
            title_bar: None,
            has_custom_title_bar: false,
            instance_guard: None,
            assets_dirty: false,
            theme_css: None,
            environment: env,
            injected_css: Vec::new(),
            injected_js: Vec::new(),
        }
    }

    /// Append a CSS string to be injected into every page (after `theme_css`).
    /// Persists across navigation. Useful for hosts that want to push a
    /// site-wide stylesheet without owning the page HTML (e.g. WCP StatusBar
    /// pushing widget chrome rules into PRISM Widget scenes).
    pub fn inject_css(&mut self, css: impl Into<String>) {
        self.injected_css.push(css.into());
        self.assets_dirty = true;
    }

    /// Append a JavaScript snippet executed after each page's own scripts.
    /// Persists across navigation. Use to bridge host-specific globals (e.g.
    /// `window.WCP = ...`) into pages without modifying their source.
    pub fn inject_js(&mut self, js: impl Into<String>) {
        self.injected_js.push(js.into());
        self.assets_dirty = true;
    }

    /// Read-only access to the accumulated CSS injections.
    pub fn injected_css(&self) -> &[String] { &self.injected_css }

    /// Read-only access to the accumulated JS injections.
    pub fn injected_js(&self) -> &[String] { &self.injected_js }

    /// Clear all CSS/JS injections.
    pub fn clear_injections(&mut self) {
        self.injected_css.clear();
        self.injected_js.clear();
        self.assets_dirty = true;
    }

    /// The active surface kind for this host.
    pub fn environment(&self) -> Environment { self.environment }

    /// Register a navigation route.
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }

    /// Add multiple routes.
    pub fn add_routes(&mut self, routes: impl IntoIterator<Item = Route>) {
        self.routes.extend(routes);
    }

    /// Register a custom protocol handler (e.g., for opendesktop://).
    pub fn add_protocol_handler(&mut self, handler: Box<dyn ProtocolHandler>) {
        self.protocol_handlers.push(handler);
    }

    /// Set the declared runtime capabilities.
    pub fn set_capabilities(&mut self, caps: CapabilitySet) {
        self.devtools.has_network = caps.has_network();
        self.capabilities = caps;
    }

    /// Set the active theme CSS that gets prepended to every page's
    /// stylesheet. Pass `None` to disable themed overrides.
    pub fn set_theme_css(&mut self, css: Option<String>) {
        self.theme_css = css;
    }

    /// Currently active theme CSS, if any.
    pub fn theme_css(&self) -> Option<&str> {
        self.theme_css.as_deref()
    }

    /// Load a custom title bar from a `title-bar.html` file.
    /// If the file exists and compiles successfully, the title bar is stored
    /// and `has_custom_title_bar` is set to `true` — the consuming app should
    /// disable native window decorations.
    pub fn load_title_bar(&mut self, base_dir: &Path) {
        let title_bar_path = base_dir.join("title-bar.html");
        if !title_bar_path.exists() {
            return;
        }

        log::info!("AppHost: loading custom title bar from {}", title_bar_path.display());

        match load_html_document_full(&title_bar_path, "title-bar") {
            Ok((doc, _scripts, _rules)) => {
                // Default title bar height: use the root node's height if set, else 32px.
                let height = match doc.nodes.first() {
                    Some(root) => match root.style.height {
                        crate::prd::value::Dimension::Px(h) => h,
                        _ => 32.0,
                    },
                    None => 32.0,
                };

                self.title_bar = Some(TitleBarInstance {
                    scene: SceneGraph::new(doc),
                    input_handler: InputHandler::new(),
                    height,
                });
                self.has_custom_title_bar = true;
            }
            Err(e) => {
                log::error!("AppHost: failed to load title-bar.html: {}", e);
            }
        }
    }

    /// Get the title bar height (0.0 if no custom title bar).
    pub fn title_bar_height(&self) -> f32 {
        self.title_bar.as_ref().map_or(0.0, |tb| tb.height)
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

    /// Set which route id is treated as the "Home" page (used by the
    /// right-click menu's Home action).
    pub fn set_home_route(&mut self, page_id: &str) {
        self.home_route = Some(page_id.to_string());
    }

    /// Replace the right-click context menu using a developer-supplied
    /// configuration. `extra_items` are appended after the (filtered)
    /// built-in entries; `hide_defaults` removes built-ins by name
    /// (`inspect`, `devtools`, `popout-devtools`, `debug-server`, `home`,
    /// `back`, `forward`, `reload`, `exit`).
    pub fn configure_context_menu(
        &mut self,
        extra_items: &[crate::config::ContextMenuItemConfig],
        hide_defaults: &[String],
    ) {
        use crate::devtools::context_menu::{ContextAction, ContextMenu, ContextMenuEntry};

        let mut entries: Vec<ContextMenuEntry> = Vec::new();
        for it in extra_items {
            if it.separator || it.label.trim() == "-" {
                entries.push(ContextMenuEntry::Separator);
                continue;
            }
            let action = match it.action.as_deref().unwrap_or("").trim() {
                "" => continue,
                "home"            => ContextAction::Home,
                "back"            => ContextAction::Back,
                "forward"         => ContextAction::Forward,
                "reload"          => ContextAction::Reload,
                "devtools"        => ContextAction::ToggleDevTools,
                "popout-devtools" => ContextAction::PopoutDevTools,
                "debug-server"    => ContextAction::DebugServer,
                "inspect"         => ContextAction::InspectElement,
                "exit"            => ContextAction::Exit,
                other if other.starts_with("navigate:") => {
                    ContextAction::NavigateTo(other["navigate:".len()..].to_string())
                }
                other if other.starts_with("js:") => {
                    ContextAction::Eval(other["js:".len()..].to_string())
                }
                other => {
                    log::warn!("[Prism] unknown context-menu action: '{other}'");
                    continue;
                }
            };
            entries.push(ContextMenuEntry::Item {
                label: it.label.clone(),
                shortcut: it.shortcut.clone(),
                action,
                enabled: true,
            });
        }
        self.devtools.context_menu = ContextMenu::with_config(entries, hide_defaults);
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
        self.assets_dirty = true; // re-upload GPU textures for the new active page
        self.pending_events.push(AppEvent::NavigateTo(page_id.to_string()));

        log::info!("AppHost: navigated to '{}'", page_id);
    }

    /// Request a content swap inside the active page's `<page-content>` container.
    ///
    /// This loads a fragment HTML file (e.g. `device_edit.html`) and replaces
    /// the children of the `<page-content>` node — analogous to what happens
    /// when a sidebar `data-navigate` click targets a non-route ID.
    ///
    /// Returns `false` if there is no active page or it lacks a `<page-content>`.
    pub fn request_content_swap(&mut self, content_id: &str) -> bool {
        let page_id = match self.active_page.clone() {
            Some(id) => id,
            None => return false,
        };
        if let Some(page) = self.pages.get(&page_id) {
            if page.scene.document.find_page_content_node().is_some() {
                self.pending_events.push(AppEvent::ContentSwap {
                    page: page_id,
                    content_id: content_id.to_string(),
                });
                return true;
            }
        }
        false
    }

    /// Navigate back.
    pub fn navigate_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            if let Some(current) = self.active_page.take() {
                self.forward_stack.push(current);
            }
            self.active_page = Some(prev);
            self.assets_dirty = true;
        }
    }

    /// Navigate forward.
    pub fn navigate_forward(&mut self) {
        if let Some(next) = self.forward_stack.pop() {
            if let Some(current) = self.active_page.take() {
                self.history.push(current);
            }
            self.active_page = Some(next);
            self.assets_dirty = true;
        }
    }

    /// Get the active page ID.
    pub fn active_page(&self) -> Option<&str> {
        self.active_page.as_deref()
    }

    /// Check and clear the assets_dirty flag.  When true, the caller should
    /// call `active_scene_assets()` and upload the bundle to the GPU.
    pub fn take_assets_dirty(&mut self) -> bool {
        let dirty = self.assets_dirty;
        self.assets_dirty = false;
        dirty
    }

    /// Get the asset bundle from the active page's scene document.
    pub fn active_scene_assets(&self) -> Option<&crate::prd::asset::AssetBundle> {
        self.active_page.as_ref()
            .and_then(|pid| self.pages.get(pid))
            .map(|page| &page.scene.document.assets)
    }

    /// Get icon declarations from the active page's document.
    pub fn icon_declarations(&self) -> &[crate::prd::document::IconDecl] {
        if let Some(ref page_id) = self.active_page {
            if let Some(page) = self.pages.get(page_id) {
                return &page.scene.document.icons;
            }
        }
        &[]
    }

    /// Get the active page's `<title>` value, if any.
    pub fn active_window_title(&self) -> Option<String> {
        let pid = self.active_page.as_ref()?;
        let page = self.pages.get(pid)?;
        page.scene.document.title.clone()
    }

    /// Decode the first usable icon declared by the active page (matching
    /// `target` "", "window", or "app") into raw RGBA8 bytes.
    /// Returns `(rgba, width, height)` or `None` if no icon is declared,
    /// the file cannot be read, or the format is unsupported.
    pub fn active_app_icon_rgba(&self) -> Option<(Vec<u8>, u32, u32)> {
        let icons = self.icon_declarations();
        if icons.is_empty() {
            return None;
        }
        // Prefer window/app/empty-target icons over system-only icons.
        let preferred = icons
            .iter()
            .find(|i| {
                let t = i.target.as_str();
                t.is_empty() || t == "window" || t == "app"
            })
            .or_else(|| icons.first())?;

        // Try embedded bundle first (paths like `pages/icons/icon.ico`),
        // fall back to filesystem.
        let bytes: Vec<u8> = if let Some(b) = crate::embed::read_page_bytes(&preferred.path) {
            b.to_vec()
        } else if let Ok(b) = std::fs::read(&preferred.path) {
            b
        } else {
            log::warn!("active_app_icon_rgba: could not read icon at '{}'", preferred.path);
            return None;
        };

        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                Some((rgba.into_raw(), w, h))
            }
            Err(e) => {
                log::warn!("active_app_icon_rgba: decode failed for '{}': {}", preferred.path, e);
                None
            }
        }
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
            // Update DevTools tab-bar hover so the tab strip can show a
            // hover background on the tab under the cursor.
            self.devtools.hovered_tab = self.devtools
                .hit_test_tab(*x, *y, viewport_width, viewport_height);

            // Drive any in-progress DevTools drags (sidebar splitter,
            // scrollbar thumbs).
            if self.devtools.elements_dragging_sidebar {
                self.devtools.drag_elements_splitter(*x, viewport_width);
                return;
            }
            let doc_opt = self.active_scene().map(|s| s.document.clone());
            if let Some(doc) = doc_opt.as_ref() {
                if self.devtools.drag_elements_scrollbar(
                    *x, *y, viewport_width, viewport_height, doc,
                ) {
                    return;
                }
            }
        }

        // Mouse-up: end any DevTools drag (splitter or scrollbar).
        if let RawInputEvent::MouseUp { button: CxMouseButton::Left, .. } = &event {
            self.devtools.end_elements_drag();
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
                if self.devtools.hit_test_badge(x, y, viewport_width, viewport_height) {
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
                    // Forward to Elements panel (expand/collapse, search box,
                    // splitter, sidebar tabs, force-state chips, highlight).
                    if let Some(page) = self
                        .active_page
                        .as_ref()
                        .and_then(|id| self.pages.get(id))
                    {
                        // Clone the document so we can borrow `self.devtools`
                        // mutably without aliasing.
                        let doc = page.scene.document.clone();
                        self.devtools.handle_elements_click_ex(
                            x,
                            y,
                            viewport_width,
                            viewport_height,
                            &doc,
                        );
                    } else {
                        let doc = PrdDocument::new("empty", SceneType::ConfigPanel);
                        self.devtools.handle_elements_click_ex(
                            x,
                            y,
                            viewport_width,
                            viewport_height,
                            &doc,
                        );
                    }
                    return;
                }

                // Click landed outside the DevTools panel — clear any
                // selected element so the highlight overlay disappears.
                if self.devtools.selected_node.is_some() {
                    self.devtools.selected_node = None;
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
        if let RawInputEvent::MouseWheel { delta_x, delta_y, .. } = &event {
            let (_, y) = if let Some(page) = self.active_page.as_ref()
                .and_then(|id| self.pages.get(id))
            {
                page.input_handler.mouse_pos
            } else {
                (0.0, 0.0)
            };
            if self.devtools.hit_test_panel(0.0, y, viewport_height) {
                let doc_opt = self.active_scene().map(|s| s.document.clone());
                self.devtools.handle_scroll_xy(
                    *delta_x, *delta_y,
                    doc_opt.as_ref(),
                    viewport_width, viewport_height,
                );
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

        // Route events in the title bar area to the title bar input handler.
        if self.title_bar.is_some() {
            let in_title_bar = match &event {
                RawInputEvent::MouseMove { y, .. }
                | RawInputEvent::MouseDown { y, .. }
                | RawInputEvent::MouseUp { y, .. } => *y < self.title_bar.as_ref().unwrap().height,
                RawInputEvent::MouseWheel { y, .. } => *y < self.title_bar.as_ref().unwrap().height,
                _ => false,
            };

            if in_title_bar {
                let tb = self.title_bar.as_mut().unwrap();
                let ui_events = tb.input_handler.process_event(&mut tb.scene.document, event);
                for ui_event in ui_events {
                    match ui_event {
                        UiEvent::Action(ref action) => {
                            use crate::prd::node::EventAction;
                            match action {
                                EventAction::WindowClose => {
                                    self.pending_events.push(AppEvent::CloseRequested);
                                }
                                EventAction::WindowMinimize => {
                                    self.pending_events.push(AppEvent::MinimizeRequested);
                                }
                                EventAction::WindowMaximize => {
                                    self.pending_events.push(AppEvent::MaximizeToggleRequested);
                                }
                                EventAction::WindowDrag => {
                                    self.pending_events.push(AppEvent::WindowDragRequested);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                return;
            }
        }

        // Route event to active page's input handler.
        let is_mouse_move = matches!(event, RawInputEvent::MouseMove { .. });
        let is_mouse_down = matches!(event, RawInputEvent::MouseDown { .. });
        let is_mouse_up = matches!(event, RawInputEvent::MouseUp { .. });
        if let Some(ref page_id) = self.active_page.clone() {
            if let Some(page) = self.pages.get_mut(page_id) {
                let prev_hovered = page.input_handler.hovered;
                let ui_events = page.input_handler.process_event(&mut page.scene.document, event);
                let mut click_node_ids: Vec<u32> = Vec::new();

                for ui_event in ui_events {
                    match ui_event {
                        UiEvent::NavigateRequest { scene_id } => {
                            // Prefer fragment swap when the active page is a shell
                            // with a <page-content> container — sidebar/nav clicks
                            // should swap the inner content rather than reload the
                            // whole window. Fall back to a full route navigation
                            // when the active page has no shell.
                            if page.scene.document.find_page_content_node().is_some() {
                                self.pending_events.push(AppEvent::ContentSwap {
                                    page: page_id.clone(),
                                    content_id: scene_id,
                                });
                            } else if self.routes.iter().any(|r| r.id == scene_id) {
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
                        UiEvent::Action(ref action) => {
                            use crate::prd::node::EventAction;
                            match action {
                                EventAction::WindowClose => {
                                    self.pending_events.push(AppEvent::CloseRequested);
                                }
                                EventAction::WindowMinimize => {
                                    self.pending_events.push(AppEvent::MinimizeRequested);
                                }
                                EventAction::WindowMaximize => {
                                    self.pending_events.push(AppEvent::MaximizeToggleRequested);
                                }
                                EventAction::WindowDrag => {
                                    self.pending_events.push(AppEvent::WindowDragRequested);
                                }
                                _ => {}
                            }
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

                // If hover target changed and any involved node has hover_style,
                // repaint so the hover overrides are applied visually.
                if is_mouse_move && page.input_handler.hovered != prev_hovered {
                    page.scene.invalidate_paint();
                }

                // If mouse was pressed or released, repaint for :active / :focus styles.
                if is_mouse_down || is_mouse_up {
                    page.scene.invalidate_paint();
                }

                // If scrolling occurred, re-layout to apply the new scroll offset.
                if page.input_handler.scroll_dirty {
                    page.input_handler.scroll_dirty = false;
                    page.scene.invalidate_layout();
                }

                // If a class was toggled, re-apply CSS rules so styles reflect the new class.
                if page.input_handler.class_dirty {
                    page.input_handler.class_dirty = false;
                    crate::compiler::html::reapply_all_styles(
                        &mut page.scene.document,
                        &page.css_rules,
                    );
                    page.scene.invalidate_layout();
                    // Sync toggled classes back to JS runtime so merge_js_document
                    // doesn't overwrite them on the next tick.
                    if let Some(ref js_rt) = self.js_runtime {
                        js_rt.sync_document(&page.scene.document);
                    }
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
        scale_factor: f32,
    ) -> Vec<AppEvent> {
        // Handle pending context menu actions.
        if let Some(action) = self.pending_context_action.take() {
            match action {
                ContextAction::ToggleDevTools => {
                    self.devtools.toggle();
                }
                ContextAction::PopoutDevTools => {
                    self.popout_devtools();
                }
                ContextAction::DebugServer => {
                    self.toggle_debug_server();
                }
                ContextAction::Reload => {
                    self.reload_active_page();
                }
                ContextAction::Home => {
                    if let Some(home) = self.home_route.clone() {
                        self.navigate_to(&home);
                    }
                }
                ContextAction::Back => {
                    self.navigate_back();
                }
                ContextAction::Forward => {
                    self.navigate_forward();
                }
                ContextAction::NavigateTo(id) => {
                    self.navigate_to(&id);
                }
                ContextAction::Eval(code) => {
                    if let Some(rt) = self.js_runtime.as_mut() {
                        rt.execute(&code, "<context-menu>");
                    }
                }
                ContextAction::Exit => {
                    self.pending_events.push(AppEvent::CloseRequested);
                }
                ContextAction::InspectElement => {
                    // Handled in main.rs where we have cursor position context
                }
            }
        }

        // Tray events are handled exclusively by `poll_tray()` (called from
        // `about_to_wait`) to avoid a double-poll race when the window is hidden.

        // Smooth-scroll DevTools toward its scroll targets.
        self.devtools.tick_scroll(dt);

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

            // If JS modified the DOM, merge changes into the scene document
            // without clobbering runtime state (hovered, layout, etc.).
            if js_rt.take_layout_dirty() {
                if let Some(ref page_id) = self.active_page {
                    if let Some(page) = self.pages.get_mut(page_id) {
                        let js_doc = js_rt.document();
                        page.scene.merge_js_document(&js_doc);
                        drop(js_doc);
                        // Apply CSS rules to any newly created/changed nodes.
                        crate::compiler::html::reapply_all_styles(
                            &mut page.scene.document,
                            &page.css_rules,
                        );
                    }
                }
            }

            // If JS created new image assets (e.g. SVG via innerHTML),
            // mark them for GPU upload.
            if js_rt.take_assets_dirty() {
                self.assets_dirty = true;
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

                let _ = page.scene.tick(content_width, viewport_height, dt, font_system, scale_factor);
            }
        }

        // Tick custom title bar if present.
        if let Some(ref mut tb) = self.title_bar {
            let _ = tb.scene.tick(viewport_width, tb.height, dt, font_system, scale_factor);
        }

        // Drain pending events.
        let mut events = std::mem::take(&mut self.pending_events);

        // Handle content swaps internally (page-content fragment loading).
        let mut i = 0;
        while i < events.len() {
            if let AppEvent::ContentSwap { ref page, ref content_id } = events[i] {
                let page_id = page.clone();
                let target = content_id.clone();
                self.swap_page_content(&page_id, &target);
                // Replace the internal ContentSwap with a public ContentSwapped
                // so consuming apps can react (e.g. populate data).
                events[i] = AppEvent::ContentSwapped { content_id: target };
            } else {
                i += 1;
            }
        }

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

    /// Current cursor icon for the active page (driven by CSS `cursor` and
    /// hover state). Consumers should apply this to the window each frame.
    pub fn current_cursor(&self) -> crate::scene::input_handler::CursorIcon {
        self.active_page
            .as_ref()
            .and_then(|id| self.pages.get(id))
            .map(|p| p.input_handler.cursor)
            .unwrap_or(crate::scene::input_handler::CursorIcon::Default)
    }

    /// Get the title bar scene (for rendering text areas).
    pub fn title_bar_scene(&self) -> Option<&SceneGraph> {
        self.title_bar.as_ref().map(|tb| &tb.scene)
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

    /// Initialise system tray with a custom configuration.
    pub fn init_tray_with_config(&mut self, config: TrayConfig) {
        if !self.capabilities.has_tray() {
            return;
        }
        let tooltip = config.tooltip.clone();
        self.tray_menu_html_path = config.menu_html_path.clone();
        self.system_tray = Some(SystemTray::new(&config));
        log::info!("System tray created for '{}'", tooltip);
    }

    /// Path to the configured custom tray menu HTML file, if any.
    pub fn tray_menu_html_path(&self) -> Option<&str> {
        self.tray_menu_html_path.as_deref()
    }

    /// Set or clear the custom tray menu HTML path at runtime.
    pub fn set_tray_menu_html_path(&mut self, path: Option<String>) {
        self.tray_menu_html_path = path;
    }

    /// Update the tray menu items dynamically (preserves built-in Reload/Exit).
    pub fn update_tray_menu(&mut self, items: &[crate::tray::TrayMenuEntry]) {
        if let Some(ref mut tray) = self.system_tray {
            tray.update_menu(items);
        }
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

        // Helper closure to execute a single script block.
        let execute_script = |js_rt: &mut JsRuntime, script: &ScriptBlock, source_dir: &Option<PathBuf>| {
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
        };

        // Execute immediate (non-deferred) scripts first.
        for script in &scripts {
            if !script.deferred {
                execute_script(&mut js_rt, script, &source_dir);
            }
        }

        // Fire DOMContentLoaded.
        js_rt.execute(
            r#"(function(){
                if(typeof __or_globalListeners==='object' && __or_globalListeners['DOMContentLoaded']){
                    var fns=__or_globalListeners['DOMContentLoaded'].slice();
                    for(var i=0;i<fns.length;i++){try{fns[i]({type:'DOMContentLoaded'});}catch(e){console.error('DOMContentLoaded handler error:',e);}}
                }
            })();"#,
            "<DOMContentLoaded>",
        );

        // Execute deferred scripts after DOM is ready.
        for script in &scripts {
            if script.deferred {
                execute_script(&mut js_rt, script, &source_dir);
            }
        }

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
        // Emit PageReloaded so the consumer re-initializes the JS runtime.
        self.pending_events.push(AppEvent::PageReloaded(page_id));
    }

    /// Toggle the debug web server on/off and open the browser.
    fn toggle_debug_server(&mut self) {
        if self.debug_server.is_running() {
            self.debug_server.stop();
            log::info!("Debug server stopped.");
        } else {
            // Find the active page's HTML source path.
            if let Some(path) = self.active_html_path() {
                self.debug_server.update_content(&path);
            }
            let port = self.debug_server.start();
            if port > 0 {
                let url = format!("http://127.0.0.1:{}", port);
                log::info!("Debug server started at {}", url);
                // Open in default browser.
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("cmd").args(["/C", "start", &url]).spawn(); }
                #[cfg(target_os = "macos")]
                { let _ = std::process::Command::new("open").arg(&url).spawn(); }
                #[cfg(target_os = "linux")]
                { let _ = std::process::Command::new("xdg-open").arg(&url).spawn(); }
            }
        }
    }

    /// Pop the DevTools out into the system browser via the debug web server.
    /// Closes the in-window panel and ensures the server is running.
    fn popout_devtools(&mut self) {
        // Hide the in-window panel — devtools is now "external".
        self.devtools.open = false;
        if self.debug_server.is_running() {
            let port = self.debug_server.port();
            if port > 0 {
                open_in_browser(&format!("http://127.0.0.1:{}", port));
            }
            return;
        }
        if let Some(path) = self.active_html_path() {
            self.debug_server.update_content(&path);
        }
        let port = self.debug_server.start();
        if port > 0 {
            let url = format!("http://127.0.0.1:{}", port);
            log::info!("DevTools popped out at {}", url);
            open_in_browser(&url);
        }
    }

    /// Get the HTML file path for the currently active page.
    fn active_html_path(&self) -> Option<PathBuf> {
        let page_id = self.active_page.as_ref()?;
        let route = self.routes.iter().find(|r| &r.id == page_id)?;
        match &route.source {
            PageSource::HtmlFile(path) => Some(path.clone()),
            _ => None,
        }
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
            PrdDocument::new("empty", SceneType::ConfigPanel)
        };

        // Append DevTools overlay instances.
        let devtools_instances = self.devtools.paint(&doc_for_devtools, viewport_width, viewport_height);
        let highlight_instances = self.devtools.scene_highlight_instances(&doc_for_devtools);
        let mut combined = patched;

        // Prepend title bar instances (rendered on top, at y=0).
        if let Some(ref tb) = self.title_bar {
            combined.extend(tb.scene.cached_instances.iter().cloned());
        }

        combined.extend(highlight_instances);
        combined.extend(devtools_instances);

        self.devtools.draw_calls = combined.len() as u32;

        (combined, clear_color)
    }

    /// Return scene instances (with title bar) and DevTools instances separately,
    /// along with the clear color. This enables the consumer to render them in
    /// separate layers for correct text z-ordering.
    pub fn split_instances(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
    ) -> (Vec<UiInstance>, Vec<UiInstance>, Color) {
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

        self.devtools.instance_count = patched.len() as u32;

        let doc_for_devtools = if let Some(scene) = self.active_scene() {
            scene.document.clone()
        } else {
            PrdDocument::new("empty", SceneType::ConfigPanel)
        };

        let devtools_instances = self.devtools.paint(&doc_for_devtools, viewport_width, viewport_height);
        let highlight_instances = self.devtools.scene_highlight_instances(&doc_for_devtools);

        let mut scene = patched;
        if let Some(ref tb) = self.title_bar {
            scene.extend(tb.scene.cached_instances.iter().cloned());
        }
        scene.extend(highlight_instances);

        self.devtools.draw_calls = (scene.len() + devtools_instances.len()) as u32;

        (scene, devtools_instances, clear_color)
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
                &PrdDocument::new("empty", SceneType::ConfigPanel),
                viewport_width,
                viewport_height,
            );
        };
        self.devtools.text_entries(doc, viewport_width, viewport_height)
    }

    /// Returns the context menu overlay rect `(x, y, w, h)` if open, else `None`.
    /// Consumers can use this to clip scene text behind the context menu.
    pub fn context_menu_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.devtools.context_menu.overlay_rect()
    }

    /// GPU instances for the context menu overlay (rendered in a separate layer).
    pub fn context_menu_instances(&self) -> Vec<UiInstance> {
        self.devtools.context_menu_instances()
    }

    /// Text entries for the context menu overlay (rendered in a separate layer).
    pub fn context_menu_text_entries(&self) -> Vec<crate::devtools::DevToolsTextEntry> {
        self.devtools.context_menu_text_entries()
    }

    /// GPU instances that must paint after the DevTools text pass (e.g.
    /// the Elements breadcrumb cover rect). Caller should append these
    /// to the overlay rect layer (paints after devtools text, before
    /// context menu).
    pub fn devtools_post_text_instances(
        &self,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<UiInstance> {
        self.devtools.post_text_instances(viewport_width, viewport_height)
    }

    /// Text entries paired with [`Self::devtools_post_text_instances`] so
    /// the breadcrumb label re-paints on top of the cover rect.
    pub fn devtools_post_text_entries(
        &self,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<crate::devtools::DevToolsTextEntry> {
        let doc = if let Some(scene) = self.active_scene() {
            scene.document.clone()
        } else {
            PrdDocument::new("empty", SceneType::ConfigPanel)
        };
        self.devtools.post_text_entries(&doc, viewport_width, viewport_height)
    }

    /// Fire a PRISM toast from Rust. Returns immediately; the toast is
    /// rendered by the JS runtime via the embedded toast shim. Variant
    /// must be one of: `"info"`, `"success"`, `"warning"`, `"danger"`.
    /// `timeout_ms` is clamped to `[1000, 10000]` (or `0` for persistent).
    pub fn show_toast(&mut self, title: &str, message: &str, variant: &str, timeout_ms: Option<u32>) {
        // Minimal JSON string escape — sufficient for embedding into a JS
        // string literal. Handles backslash, quote, and control chars.
        fn esc(s: &str) -> String {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('"');
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '"'  => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }
        let persistent = matches!(timeout_ms, Some(0));
        let timeout = timeout_ms.unwrap_or(4000);
        let script = format!(
            "try {{ if (typeof toast !== 'undefined') {{ \
                toast.show({{ title: {title}, message: {message}, variant: {variant}, \
                    persistent: {persistent}, timeoutMs: {timeout} }}); \
            }} }} catch (e) {{ if (typeof console !== 'undefined') console.error('show_toast failed:', e); }}",
            title = esc(title),
            message = esc(message),
            variant = esc(variant),
            persistent = persistent,
            timeout = timeout,
        );
        if let Some(ref mut js) = self.js_runtime {
            js.execute(&script, "<show_toast>");
        }
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

    /// Execute a JS snippet in the page's V8 runtime.
    pub fn execute_js(&mut self, source: &str) {
        if let Some(ref mut js_rt) = self.js_runtime {
            js_rt.execute(source, "<ipc>");
        }
    }

    /// Whether this host has system tray active (for close behavior).
    pub fn has_active_tray(&self) -> bool {
        self.system_tray.as_ref().map_or(false, |t| t.is_active())
    }

    /// Attach a single-instance guard obtained from
    /// `instance::acquire_single_instance()`.
    ///
    /// The guard is polled automatically by `poll_tray()` — any focus requests
    /// from secondary launches are emitted as `AppEvent::TrayShowWindow`.
    pub fn set_instance_guard(&mut self, guard: InstanceGuard) {
        self.instance_guard = Some(guard);
    }

    /// Whether a single-instance guard is active.
    pub fn has_instance_guard(&self) -> bool {
        self.instance_guard.is_some()
    }

    /// Poll tray events without doing a full tick.
    ///
    /// Use in `about_to_wait()` so tray events (especially Exit) are processed
    /// even when the window is hidden and `RedrawRequested` never fires.
    ///
    /// Also polls the single-instance guard for focus requests from secondary
    /// launches, emitting them as `AppEvent::TrayShowWindow`.
    pub fn poll_tray(&mut self) -> Vec<AppEvent> {
        let mut events = Vec::new();

        // Poll single-instance focus requests.
        if let Some(ref guard) = self.instance_guard {
            for _ in guard.poll_focus_requests() {
                self.window_visible = true;
                events.push(AppEvent::TrayShowWindow);
            }
        }

        if let Some(ref tray) = self.system_tray {
            for event in tray.poll_events() {
                match event {
                    TrayEvent::ShowWindow => {
                        self.window_visible = true;
                        events.push(AppEvent::TrayShowWindow);
                    }
                    TrayEvent::ToggleWindow => {
                        self.window_visible = !self.window_visible;
                        events.push(AppEvent::TrayToggleWindow);
                    }
                    TrayEvent::Exit => {
                        events.push(AppEvent::CloseRequested);
                    }
                    TrayEvent::Reload => {
                        self.reload_active_page();
                    }
                    TrayEvent::ToggleAutostart => {
                        events.push(AppEvent::TrayToggleAutostart);
                    }
                    TrayEvent::CheckForUpdate => {
                        events.push(AppEvent::TrayCheckForUpdate);
                    }
                    TrayEvent::ShowCustomMenuAt { x, y } => {
                        // Only forward when a custom HTML menu is actually
                        // configured \u2014 otherwise the OS already showed the
                        // native menu and there's nothing for us to pop up.
                        if self.tray_menu_html_path.is_some() {
                            events.push(AppEvent::ShowCustomTrayMenu { x, y });
                        }
                    }
                    TrayEvent::CustomAction(id) => {
                        events.push(AppEvent::TrayAction(id));
                    }
                }
            }
        }
        events
    }

    // --- Internal ---

    /// Swap the content of a `<page-content>` container with a new HTML fragment.
    ///
    /// Loads `{source_dir}/{target_id}.html`, compiles its nodes, and replaces
    /// the children of the PageContent node in-place. Respects `<meta name="redirect">`
    /// in the fragment to prevent redirect loops.
    fn swap_page_content(&mut self, page_id: &str, target_id: &str) {
        let page = match self.pages.get_mut(page_id) {
            Some(p) => p,
            None => return,
        };

        let pc_node_id = match page.scene.document.find_page_content_node() {
            Some(id) => id,
            None => return,
        };

        // Don't reload if already showing this content.
        let already_active = page.scene.document.get_node(pc_node_id)
            .and_then(|n| n.attributes.get("data-active-content"))
            .map(|s| s.as_str()) == Some(target_id);
        if already_active {
            return;
        }

        // Resolve the fragment from either a filesystem source dir (when
        // the host page came from disk) or the embedded `pages/` bundle.
        let (frag_doc, _frag_scripts, _) = if let Some(source_dir) = page.source_dir.as_ref() {
            let fragment_path = source_dir.join(format!("{}.html", target_id));
            match load_html_document_full(&fragment_path, target_id) {
                Ok(result) => result,
                Err(e) => {
                    log::error!("Failed to load content fragment '{}': {}", target_id, e);
                    return;
                }
            }
        } else if let Some(embed_base) = page.embed_base.clone() {
            let rel = if embed_base.is_empty() {
                format!("{}.html", target_id)
            } else {
                format!("{}/{}.html", embed_base.trim_end_matches('/'), target_id)
            };
            match load_embedded_document_full(&rel, target_id, self.theme_css.as_deref()) {
                Ok(result) => result,
                Err(e) => {
                    log::error!("Failed to load embedded content fragment '{}': {}", target_id, e);
                    return;
                }
            }
        } else {
            log::warn!("swap_page_content: page '{}' has neither source_dir nor embed_base", page_id);
            return;
        };

        // Check for redirect in the fragment.
        if let Some(ref redirect) = frag_doc.redirect {
            if redirect != target_id {
                let redirect = redirect.clone();
                let page_id = page_id.to_string();
                // Re-borrow after drop to avoid double mutable borrow.
                self.swap_page_content(&page_id, &redirect);
                return;
            }
        }

        // Re-acquire mutable reference (needed after potential recursive call above).
        let page = self.pages.get_mut(page_id).unwrap();

        // Free old children of page-content node.
        page.scene.document.free_subtree(pc_node_id);

        // Transplant new children from fragment document.
        page.scene.document.transplant_children_from(&frag_doc, pc_node_id);

        // Re-apply the page's CSS rules to the transplanted nodes so they pick
        // up the page-level stylesheet (fragments compile with no external CSS).
        crate::compiler::html::reapply_all_styles(
            &mut page.scene.document,
            &page.css_rules,
        );

        // Reset scroll position for the new content.
        if let Some(node) = page.scene.document.get_node_mut(pc_node_id) {
            node.layout.scroll_y = 0.0;
        }

        // Update the active content attribute.
        if let Some(node) = page.scene.document.get_node_mut(pc_node_id) {
            node.attributes.insert("data-active-content".to_string(), target_id.to_string());
        }

        page.dirty = true;
        page.scene.invalidate_layout();

        // Sync the updated document into the JS runtime so that JS-driven DOM
        // syncs (layout_dirty) don't overwrite the swapped content with a stale copy.
        if let Some(ref js_rt) = self.js_runtime {
            js_rt.sync_document(&page.scene.document);
        }

        // Notify JS that the page content changed so page-specific init can run.
        let swap_code = format!(
            "if(typeof window.__veil_on_content_swap==='function')window.__veil_on_content_swap('{}');",
            target_id.replace('\'', "\\'")
        );
        if let Some(ref mut js_rt) = self.js_runtime {
            js_rt.eval_script(&swap_code);
        }

        // Fragment may have brought new image assets (e.g. rasterized SVGs);
        // mark dirty so the render loop uploads them to the GPU.
        self.assets_dirty = true;

        log::info!("AppHost: swapped page-content to '{}'", target_id);
    }

    fn load_page(&mut self, route: &Route) {
        let mut embed_base: Option<String> = None;
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
                        (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                    }
                }
            }

            PageSource::Embedded(rel) => {
                // Record the embedded directory (relative to bundle root) so
                // page-content fragment swaps can resolve sibling files.
                let parent = std::path::Path::new(rel)
                    .parent()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                embed_base = Some(parent);
                match load_embedded_document_full(rel, &route.id, self.theme_css.as_deref()) {
                    Ok((d, s, r)) => (d, s, r, None),
                    Err(e) => {
                        log::error!("AppHost: failed to load embedded '{}': {}", rel, e);
                        (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                    }
                }
            }

            PageSource::Inline(html) => {
                let css = extract_inline_styles(html);
                match compile_html(html, &css, &route.id, SceneType::ConfigPanel, None) {
                    Ok((d, s, r)) => (d, s, r, None),
                    Err(e) => {
                        log::error!("AppHost: failed to compile inline '{}': {}", route.id, e);
                        (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
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
                            (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                        }
                    }
                } else {
                    log::error!("AppHost: unresolvable URI '{}'", uri);
                    (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
                }
            }

            PageSource::External(url) => {
                self.pending_events.push(AppEvent::OpenExternal(url.clone()));
                (PrdDocument::new(&route.id, SceneType::ConfigPanel), Vec::new(), Vec::new(), None)
            }
        };

        let has_assets = !doc.assets.images.is_empty();
        let scene = SceneGraph::new(doc.clone());
        let input_handler = InputHandler::new();

        // Push title update if the document has a <title>.
        if let Some(ref t) = doc.title {
            self.title = t.clone();
            self.pending_events.push(AppEvent::SetTitle(t.clone()));
        }

        // Mark assets dirty so the render loop uploads textures to GPU.
        if has_assets {
            self.assets_dirty = true;
        }

        self.pages.insert(route.id.clone(), PageInstance {
            scene,
            input_handler,
            editables: None,
            dirty: true,
            scripts,
            css_rules,
            source_dir,
            embed_base,
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
) -> Result<(PrdDocument, Vec<ScriptBlock>, Vec<CssRule>), String> {
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

/// Load an HTML document from the embedded `pages/` directory. Sibling CSS
/// (same stem, `.css` extension, or a sibling `style.css`) is concatenated,
/// followed by the optional `theme_css` (themes always win over page CSS).
fn load_embedded_document_full(
    rel_path: &str,
    name: &str,
    theme_css: Option<&str>,
) -> Result<(PrdDocument, Vec<ScriptBlock>, Vec<CssRule>), String> {
    use crate::embed;

    let html = embed::read_page_str(rel_path)
        .ok_or_else(|| format!("embedded HTML not found: {rel_path}"))?;

    // Look up sibling CSS (same stem, .css) or style.css in the same dir.
    let mut css = String::new();
    if let Some(stem_css) = strip_ext_then_add(rel_path, "css") {
        if let Some(s) = embed::read_page_str(&stem_css) {
            css.push_str(s);
            css.push('\n');
        }
    }
    if let Some(parent) = parent_dir(rel_path) {
        let style = format!("{parent}/style.css");
        let style = style.trim_start_matches('/').to_string();
        if let Some(s) = embed::read_page_str(&style) {
            css.push_str(s);
            css.push('\n');
        }
    }
    if let Some(theme) = theme_css {
        css.push_str(theme);
    }

    // Preprocess <include> and <page-content> tags against the embedded
    // bundle BEFORE handing off to `compile_html` (which would otherwise try
    // to read includes from the filesystem and fail).
    let embed_base = parent_dir(rel_path).unwrap_or("");
    let expanded = crate::compiler::html::preprocess_includes_embedded(html, embed_base, 0);
    let expanded = crate::compiler::html::preprocess_page_content_embedded(&expanded, embed_base);

    compile_html(&expanded, &css, name, SceneType::ConfigPanel, None)
        .map_err(|e| e.to_string())
}

fn strip_ext_then_add(path: &str, new_ext: &str) -> Option<String> {
    let dot = path.rfind('.')?;
    Some(format!("{}.{}", &path[..dot], new_ext))
}

fn parent_dir(path: &str) -> Option<&str> {
    let slash = path.rfind('/')?;
    Some(&path[..slash])
}

/// Open a URL in the system's default browser.
fn open_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
}

/// Builder for constructing an AppHost with a common OpenDesktop-style layout
/// (sidebar + content area, with routes for each addon's options page).
pub struct OpenDesktopAppBuilder {
    host: AppHost,
    opendesktop_base: Option<PathBuf>,
}

impl OpenDesktopAppBuilder {
    /// Create a new OpenDesktop app builder.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            host: AppHost::new(title),
            opendesktop_base: None,
        }
    }

    /// Set the OpenDesktop base directory (e.g., `~/ProjectOpen/OpenDesktop`).
    pub fn opendesktop_base(mut self, path: impl Into<PathBuf>) -> Self {
        let base: PathBuf = path.into();
        // Register opendesktop:// protocol handler.
        self.host.add_protocol_handler(Box::new(FileSystemProtocol {
            scheme: "opendesktop".into(),
            base_dir: base.clone(),
        }));
        self.opendesktop_base = Some(base);
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

    /// Set the surface environment (`Application`/`Desktop`/`Overlay`/`Widget`).
    /// Replaces the capability set with the env's defaults; call
    /// [`AppHostBuilder::capabilities`] *after* this to override.
    pub fn environment(mut self, env: Environment) -> Self {
        self.host.environment = env;
        self.host.capabilities = env.default_capabilities();
        self
    }

    /// Push a CSS string to be injected into every page after compilation.
    pub fn inject_css(mut self, css: impl Into<String>) -> Self {
        self.host.inject_css(css);
        self
    }

    /// Push a JS snippet to run after every page's scripts.
    pub fn inject_js(mut self, js: impl Into<String>) -> Self {
        self.host.inject_js(js);
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

    /// Auto-discover addon options pages from the OpenDesktop assets directory.
    pub fn discover_addon_options(mut self) -> Self {
        if let Some(ref base) = self.opendesktop_base {
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

