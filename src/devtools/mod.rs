// prism-runtime/src/devtools/mod.rs
//
// Built-in developer tools for OpenRender Runtime.
// Provides an Elements panel (DOM tree view with collapsible nodes and
// computed-styles sidebar), Console (logs/errors with filtering),
// GPU info (with FPS graph), and Network panel.
// Activated by clicking the "PRISM" badge or pressing F12.

pub mod theme;
pub mod widgets;
pub mod overlay;
pub mod console;
pub mod elements;
pub mod context_menu;
pub mod debug_server;
pub mod palette;

use std::collections::HashSet;
use crate::gpu::vertex::UiInstance;
use crate::prd::document::PrdDocument;
use crate::prd::node::NodeId;
use crate::prd::value::Color;

/// Which DevTools tab is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsTab {
    Elements,
    Console,
    Sources,
    Network,
    Performance,
    Storage,
    Gpu,
}

/// Special keys consumed by the Elements search box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementsKey { Backspace, Escape }

/// Host follow-up actions requested by the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFollowup { None, Reload }

/// Badge anchor position on the viewport edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgePosition {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// The DevTools panel state.
pub struct DevTools {
    /// Whether the DevTools panel is open (visible).
    pub open: bool,
    /// The currently selected tab.
    pub active_tab: DevToolsTab,
    /// Console log entries.
    pub console: console::ConsoleLog,
    /// Whether the app declares network capability (controls Network tab visibility).
    pub has_network: bool,
    /// GPU info string (adapter name, backend, etc.).
    pub gpu_info: String,
    /// Current FPS value.
    pub fps: f32,
    /// Draw call count from last frame.
    pub draw_calls: u32,
    /// Instance count from last frame.
    pub instance_count: u32,
    /// Scroll offset for the elements panel.
    pub elements_scroll: f32,
    /// Scroll offset for the console panel.
    pub console_scroll: f32,
    /// Selected node ID in elements panel (for highlighting + styles sidebar).
    pub selected_node: Option<u32>,
    /// Hovered line index in elements panel.
    pub hovered_element_line: Option<u32>,
    /// Set of expanded node IDs in the elements tree.
    pub expanded_nodes: HashSet<NodeId>,
    /// Right-click context menu state.
    pub context_menu: context_menu::ContextMenu,
    /// Badge anchor position (default: BottomRight).
    pub badge_position: BadgePosition,
    /// Badge rotation in degrees (0, 90, 180, 270). Controls text flow direction.
    pub badge_rotation: u16,
    /// Panel height (resizable by dragging the top edge).
    pub panel_height: f32,
    /// Whether the user is currently dragging the panel resize handle.
    pub resizing: bool,
    /// FPS ring buffer for sparkline graph.
    pub fps_history: Vec<f32>,
    /// Frame time ring buffer (ms per frame).
    pub frame_time_history: Vec<f32>,
    /// Vertex count from last frame.
    pub vertex_count: u32,
    /// Texture count.
    pub texture_count: u32,
    /// Whether to draw a highlight outline in the scene around the selected
    /// or hovered DOM node (Elements panel). Toggleable via the checkbox in
    /// the Elements tab header.
    pub highlight_enabled: bool,
    /// Persistent state for the new Elements panel (search, sidebar, chips).
    pub elements_search: String,
    pub elements_search_focused: bool,
    pub elements_sidebar_tab: elements::SidebarTab,
    pub elements_force_hover: bool,
    pub elements_force_active: bool,
    pub elements_force_focus: bool,
    pub elements_sidebar_width: f32,
    pub elements_dragging_sidebar: bool,
    /// Command palette (Ctrl+Shift+P) state.
    pub palette: palette::PaletteState,
}

impl DevTools {
    pub fn new() -> Self {
        Self {
            open: false,
            active_tab: DevToolsTab::Elements,
            console: console::ConsoleLog::new(),
            has_network: false,
            gpu_info: String::new(),
            fps: 0.0,
            draw_calls: 0,
            instance_count: 0,
            elements_scroll: 0.0,
            console_scroll: 0.0,
            selected_node: None,
            hovered_element_line: None,
            expanded_nodes: HashSet::new(),
            context_menu: context_menu::ContextMenu::new(),
            badge_position: BadgePosition::BottomRight,
            badge_rotation: 0,
            panel_height: overlay::PANEL_HEIGHT,
            resizing: false,
            fps_history: Vec::with_capacity(120),
            frame_time_history: Vec::with_capacity(120),
            vertex_count: 0,
            texture_count: 0,
            highlight_enabled: true,
            elements_search: String::new(),
            elements_search_focused: false,
            elements_sidebar_tab: elements::SidebarTab::Computed,
            elements_force_hover: false,
            elements_force_active: false,
            elements_force_focus: false,
            elements_sidebar_width: theme::SIDEBAR_W,
            elements_dragging_sidebar: false,
            palette: palette::PaletteState::new(),
        }
    }

    /// Toggle the DevTools panel open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Set the badge position and rotation.
    pub fn set_badge(&mut self, position: BadgePosition, rotation: u16) {
        self.badge_position = position;
        self.badge_rotation = rotation % 360;
    }

    /// Record a frame's FPS and frame-time for the GPU tab sparkline.
    pub fn record_frame_stats(&mut self, fps: f32, frame_dt_secs: f32) {
        self.fps = fps;
        if self.fps_history.len() >= 120 {
            self.fps_history.remove(0);
        }
        self.fps_history.push(fps);

        let frame_ms = frame_dt_secs * 1000.0;
        if self.frame_time_history.len() >= 120 {
            self.frame_time_history.remove(0);
        }
        self.frame_time_history.push(frame_ms);
    }

    /// Auto-scroll console to bottom if new entries were added.
    pub fn auto_scroll_console(&mut self) {
        if self.console.has_new_entries && self.active_tab == DevToolsTab::Console {
            let total = self.console.total_content_height() + console::CONSOLE_FILTER_BAR_HEIGHT;
            let content_h = self.panel_height - overlay::TAB_BAR_HEIGHT;
            if total > content_h {
                self.console_scroll = total - content_h;
            }
            self.console.has_new_entries = false;
        }
    }

    /// Compute the badge bounding rect (x, y, w, h) based on position and rotation.
    fn badge_rect(&self, viewport_width: f32, viewport_height: f32) -> (f32, f32, f32, f32) {
        let m = overlay::BADGE_MARGIN;
        let is_vertical = self.badge_rotation == 90 || self.badge_rotation == 270;
        let (w, h) = if is_vertical {
            (overlay::BADGE_HEIGHT, overlay::BADGE_WIDTH)
        } else {
            (overlay::BADGE_WIDTH, overlay::BADGE_HEIGHT)
        };

        let x = match self.badge_position {
            BadgePosition::TopLeft | BadgePosition::CenterLeft | BadgePosition::BottomLeft => m,
            BadgePosition::TopCenter | BadgePosition::Center | BadgePosition::BottomCenter => {
                (viewport_width - w) / 2.0
            }
            BadgePosition::TopRight | BadgePosition::CenterRight | BadgePosition::BottomRight => {
                viewport_width - w - m
            }
        };
        let y = match self.badge_position {
            BadgePosition::TopLeft | BadgePosition::TopCenter | BadgePosition::TopRight => m,
            BadgePosition::CenterLeft | BadgePosition::Center | BadgePosition::CenterRight => {
                (viewport_height - h) / 2.0
            }
            BadgePosition::BottomLeft | BadgePosition::BottomCenter | BadgePosition::BottomRight => {
                viewport_height - h - m
            }
        };
        (x, y, w, h)
    }

    /// Check if a click at (x, y) hits the OpenRender watermark badge.
    pub fn hit_test_badge(&self, x: f32, y: f32, viewport_width: f32, viewport_height: f32) -> bool {
        let (bx, by, bw, bh) = self.badge_rect(viewport_width, viewport_height);
        x >= bx && x <= bx + bw && y >= by && y <= by + bh
    }

    /// Check if a click at (x, y) hits one of the tab buttons.
    pub fn hit_test_tab(&self, x: f32, y: f32, _viewport_width: f32, viewport_height: f32) -> Option<DevToolsTab> {
        if !self.open {
            return None;
        }
        let panel_y = viewport_height - self.panel_height;
        let tab_y = panel_y;
        let tab_h = overlay::TAB_BAR_HEIGHT;

        if y < tab_y || y > tab_y + tab_h {
            return None;
        }

        for (tab, tx, tw, _label) in self.tab_layout() {
            if x >= tx && x <= tx + tw {
                return Some(tab);
            }
        }
        None
    }

    /// Compute the per-tab label, x position, and dynamic width based on
    /// label length. Single source of truth for tab geometry; used by paint,
    /// text rendering, and hit-test.
    pub fn tab_layout(&self) -> Vec<(DevToolsTab, f32, f32, String)> {
        let tabs = self.visible_tabs();
        let mut out = Vec::with_capacity(tabs.len());
        let mut x = 0.0_f32;
        for tab in tabs {
            let label = match tab {
                DevToolsTab::Elements    => "Elements".to_string(),
                DevToolsTab::Console     => if self.console.error_count > 0 {
                    format!("Console ({})", self.console.error_count)
                } else { "Console".to_string() },
                DevToolsTab::Sources     => "Sources".to_string(),
                DevToolsTab::Network     => "Network".to_string(),
                DevToolsTab::Performance => "Performance".to_string(),
                DevToolsTab::Storage     => "Storage".to_string(),
                DevToolsTab::Gpu         => "GPU".to_string(),
            };
            // 12px font → ~7px per char average; 14px padding each side.
            let w = (label.chars().count() as f32 * 7.5 + 28.0).max(60.0);
            out.push((tab, x, w, label));
            x += w;
        }
        out
    }

    /// Build a snapshot of the persistent Elements state for the current frame.
    /// The new pipeline (paint_rects_with_state / text_entries_with_state /
    /// hit_test) all read from this.
    pub fn elements_state(&self) -> elements::ElementsState {
        elements::ElementsState {
            selected: self.selected_node,
            expanded: self.expanded_nodes.clone(),
            hovered_line: self.hovered_element_line,
            scroll: self.elements_scroll,
            search_query: self.elements_search.clone(),
            search_focused: self.elements_search_focused,
            sidebar_tab: self.elements_sidebar_tab,
            force_hover: self.elements_force_hover,
            force_active: self.elements_force_active,
            force_focus: self.elements_force_focus,
            sidebar_width: self.elements_sidebar_width,
            dragging_sidebar: self.elements_dragging_sidebar,
        }
    }

    /// Compute the content rect (x,y,w,h) of the Elements panel for the
    /// current viewport. Used by both paint and hit-test.
    pub fn elements_content_rect(&self, viewport_width: f32, viewport_height: f32) -> (f32, f32, f32, f32) {
        let panel_y = viewport_height - self.panel_height;
        let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
        let content_h = self.panel_height - overlay::TAB_BAR_HEIGHT;
        (0.0, content_y, viewport_width, content_h)
    }

    /// Returns the list of visible tabs (Network only if has_network).
    pub fn visible_tabs(&self) -> Vec<DevToolsTab> {
        let mut tabs = vec![
            DevToolsTab::Elements,
            DevToolsTab::Console,
            DevToolsTab::Sources,
        ];
        if self.has_network {
            tabs.push(DevToolsTab::Network);
        }
        tabs.push(DevToolsTab::Performance);
        tabs.push(DevToolsTab::Storage);
        tabs.push(DevToolsTab::Gpu);
        tabs
    }

    /// Handle scroll in the DevTools panel.
    pub fn handle_scroll(&mut self, delta_y: f32) {
        match self.active_tab {
            DevToolsTab::Elements => {
                self.elements_scroll = (self.elements_scroll - delta_y).max(0.0);
            }
            DevToolsTab::Console => {
                self.console_scroll = (self.console_scroll - delta_y).max(0.0);
            }
            _ => {}
        }
    }

    /// Check if a point is inside the DevTools panel area.
    pub fn hit_test_panel(&self, _x: f32, y: f32, viewport_height: f32) -> bool {
        if !self.open {
            return false;
        }
        y >= viewport_height - self.panel_height
    }

    /// Check if a point is on the resize handle (top 4px of panel).
    pub fn hit_test_resize_handle(&self, _x: f32, y: f32, viewport_height: f32) -> bool {
        if !self.open {
            return false;
        }
        let panel_top = viewport_height - self.panel_height;
        y >= panel_top - 3.0 && y <= panel_top + 3.0
    }

    /// Handle a click inside the Elements panel content area.
    /// Returns true if the click was consumed.
    pub fn handle_elements_click(&mut self, _x: f32, y: f32, viewport_height: f32, doc: &PrdDocument) -> bool {
        self.handle_elements_click_ex(_x, y, 99999.0, viewport_height, doc)
    }

    /// Same as `handle_elements_click` but receives the viewport width so the
    /// "highlight" checkbox in the Elements tab header (right-aligned) can be
    /// hit-tested. New code should prefer this variant.
    pub fn handle_elements_click_ex(&mut self, x: f32, y: f32, viewport_width: f32, viewport_height: f32, doc: &PrdDocument) -> bool {
        if !self.open || self.active_tab != DevToolsTab::Elements {
            return false;
        }
        let (cx, cy, cw, ch) = self.elements_content_rect(viewport_width, viewport_height);
        if y < cy || y > cy + ch {
            return false;
        }

        // Highlight checkbox hit (top-right corner of Elements content area).
        if hit_highlight_checkbox(x, y, viewport_width, cy) {
            self.highlight_enabled = !self.highlight_enabled;
            return true;
        }

        let state = self.elements_state();
        let geom = elements::Geometry::compute(&state, cx, cy, cw, ch);
        match elements::hit_test(&state, doc, &geom, x, y) {
            elements::Hit::Search => {
                self.elements_search_focused = true;
                return true;
            }
            elements::Hit::Splitter => {
                self.elements_dragging_sidebar = true;
                return true;
            }
            elements::Hit::SidebarTab(tab) => {
                self.elements_sidebar_tab = tab;
                return true;
            }
            elements::Hit::StateChip(chip) => {
                match chip {
                    elements::StateChip::Hover => self.elements_force_hover = !self.elements_force_hover,
                    elements::StateChip::Active => self.elements_force_active = !self.elements_force_active,
                    elements::StateChip::Focus => self.elements_force_focus = !self.elements_force_focus,
                }
                self.apply_force_state(doc);
                return true;
            }
            elements::Hit::TreeCaret(idx) => {
                let rows = elements::build_rows(doc, &self.expanded_nodes, &self.elements_search);
                if let Some(row) = rows.get(idx) {
                    if self.expanded_nodes.contains(&row.node_id) {
                        self.expanded_nodes.remove(&row.node_id);
                    } else {
                        self.expanded_nodes.insert(row.node_id);
                    }
                }
                self.elements_search_focused = false;
                return true;
            }
            elements::Hit::TreeRow(idx) => {
                let rows = elements::build_rows(doc, &self.expanded_nodes, &self.elements_search);
                if let Some(row) = rows.get(idx) {
                    self.selected_node = Some(row.node_id);
                    // Also toggle expansion on a row click when the row has
                    // children — the 12px caret target is too small to hit
                    // reliably, and this matches typical tree-view UX.
                    if let elements::TreeRowKind::Open { has_children: true, .. } = row.kind {
                        if self.expanded_nodes.contains(&row.node_id) {
                            self.expanded_nodes.remove(&row.node_id);
                        } else {
                            self.expanded_nodes.insert(row.node_id);
                        }
                    }
                }
                self.elements_search_focused = false;
                return true;
            }
            elements::Hit::Breadcrumb(i) => {
                let path = elements::ancestor_path(doc, self.selected_node);
                if let Some(&id) = path.get(i as usize) {
                    self.selected_node = Some(id);
                }
                return true;
            }
            elements::Hit::None => {
                // Click outside any control inside the panel — unfocus search.
                self.elements_search_focused = false;
            }
        }
        false
    }

    /// Drop the persistent splitter-drag state (call on mouse-up).
    pub fn end_elements_drag(&mut self) { self.elements_dragging_sidebar = false; }

    /// Move the sidebar splitter while dragging. `x` is mouse-x in viewport space.
    pub fn drag_elements_splitter(&mut self, x: f32, viewport_width: f32) {
        if !self.elements_dragging_sidebar { return; }
        let new_w = (viewport_width - x).clamp(160.0, viewport_width * 0.7);
        self.elements_sidebar_width = new_w;
    }

    /// Receive a typed character for the Elements search box. Returns true if
    /// the character was consumed. Caller should also forward backspace/escape
    /// via [`Self::handle_elements_key_special`].
    pub fn handle_elements_key_char(&mut self, c: char) -> bool {
        if !self.open || self.active_tab != DevToolsTab::Elements || !self.elements_search_focused {
            return false;
        }
        if c.is_control() { return false; }
        self.elements_search.push(c);
        self.elements_scroll = 0.0;
        true
    }

    /// Handle backspace/escape for the search box. Returns true if consumed.
    pub fn handle_elements_key_special(&mut self, key: ElementsKey) -> bool {
        if !self.open || self.active_tab != DevToolsTab::Elements || !self.elements_search_focused {
            return false;
        }
        match key {
            ElementsKey::Backspace => { self.elements_search.pop(); self.elements_scroll = 0.0; true }
            ElementsKey::Escape => {
                self.elements_search.clear();
                self.elements_search_focused = false;
                true
            }
        }
    }

    /// Apply the force-state chip toggles to the currently selected node so
    /// hover/active/focus styles render even without real mouse input.
    fn apply_force_state(&self, doc: &PrdDocument) {
        // Force-state mutation runs through DevTools::sync_force_state at frame
        // start; here we only flip the flags. Renderers that don't observe the
        // flags will simply not show the simulated state.
        let _ = doc;
    }

    /// Toggle the command palette (Ctrl+Shift+P).
    pub fn toggle_palette(&mut self) { self.palette.toggle(); }

    /// Dispatch a [`palette::PaletteAction`]. Returns `true` if a host-level
    /// follow-up (reload) is requested by the action; the host should call
    /// its reload routine in that case.
    pub fn invoke_palette_action(&mut self, action: palette::PaletteAction) -> PaletteFollowup {
        use palette::PaletteAction as A;
        match action {
            A::SwitchTab(t) => { self.open = true; self.active_tab = t; PaletteFollowup::None }
            A::ToggleHighlight => { self.highlight_enabled = !self.highlight_enabled; PaletteFollowup::None }
            A::Reload => PaletteFollowup::Reload,
            A::ClearConsole => { self.console.entries.clear(); self.console.error_count = 0; PaletteFollowup::None }
            A::FocusElementsSearch => {
                self.open = true;
                self.active_tab = DevToolsTab::Elements;
                self.elements_search_focused = true;
                PaletteFollowup::None
            }
            A::CloseDevTools => { self.open = false; PaletteFollowup::None }
        }
    }

    /// Handle mouse move inside the Elements panel for hover highlighting.
    pub fn update_elements_hover(&mut self, y: f32, viewport_height: f32) {
        if !self.open || self.active_tab != DevToolsTab::Elements {
            self.hovered_element_line = None;
            return;
        }
        let panel_y = viewport_height - self.panel_height;
        let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
        let content_h = self.panel_height - overlay::TAB_BAR_HEIGHT;

        if y < content_y || y > content_y + content_h {
            self.hovered_element_line = None;
            return;
        }

        // Use the same row math as the new tree (ROW_H = 18, +SP_1 top inset,
        // +SEARCH_H so the tree starts below the search box).
        let tree_top = content_y + 26.0 + 4.0; // SEARCH_H + SP_1
        if y < tree_top {
            self.hovered_element_line = None;
            return;
        }
        let relative_y = (y - tree_top) + self.elements_scroll;
        if relative_y < 0.0 {
            self.hovered_element_line = None;
            return;
        }
        self.hovered_element_line = Some((relative_y / 18.0) as u32);
    }

    /// Handle a click on the console filter bar.
    pub fn handle_console_filter_click(&mut self, y: f32, viewport_height: f32) -> bool {
        if !self.open || self.active_tab != DevToolsTab::Console {
            return false;
        }
        let panel_y = viewport_height - self.panel_height;
        let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
        let filter_bottom = content_y + console::CONSOLE_FILTER_BAR_HEIGHT;

        if y >= content_y && y <= filter_bottom {
            self.console.cycle_filter();
            return true;
        }
        false
    }

    /// Generate GPU instances for the DevTools overlay (badge + panel).
    pub fn paint(
        &self,
        doc: &PrdDocument,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<UiInstance> {
        let mut instances = Vec::new();

        overlay::paint_badge(&mut instances, viewport_width, viewport_height);

        if self.open {
            overlay::paint_panel(
                &mut instances,
                self,
                doc,
                viewport_width,
                viewport_height,
            );
        }

        // Context menu always paints on top (when open) — independent of the
        // DevTools panel so right-click menus work even when DevTools is closed.
        instances.extend(self.context_menu.paint());

        // Command palette overlays everything else.
        palette::paint(&mut instances, &self.palette, viewport_width, viewport_height);

        instances
    }

    /// GPU instances for the context menu overlay (rendered on top of scene text).
    pub fn context_menu_instances(&self) -> Vec<UiInstance> {
        self.context_menu.paint()
    }

    /// GPU instances that must paint **after** the DevTools text pass to
    /// cover bleed-through (e.g. the Elements breadcrumb bar should hide
    /// any tree-view row text drawn behind it). Returns an empty vec when
    /// the panel is closed or no overlay rect is needed.
    pub fn post_text_instances(&self, viewport_width: f32, viewport_height: f32) -> Vec<UiInstance> {
        let mut rects = Vec::new();
        let mut _texts = Vec::new();
        if !self.open {
            return rects;
        }
        if self.active_tab == DevToolsTab::Elements {
            let (cx, cy, cw, ch) = self.elements_content_rect(viewport_width, viewport_height);
            let state = self.elements_state();
            let geom = elements::Geometry::compute(&state, cx, cy, cw, ch);
            // Repaint divider hairline above the breadcrumb so the cover
            // rect blends with the rest of the panel chrome.
            rects.push(crate::devtools::widgets::hline(
                geom.content_x, geom.breadcrumb_y, geom.content_w, theme::LINE_SOFT,
            ));
            // Empty doc; only the rect from paint_breadcrumb_into matters.
            let empty = crate::prd::document::PrdDocument::new(
                "empty",
                crate::prd::document::SceneType::ConfigPanel,
            );
            elements::paint_breadcrumb_into(&mut rects, &mut _texts, &state, &empty, &geom);
        }
        rects
    }

    /// Text entries that must paint after the DevTools text pass. Pairs
    /// with [`Self::post_text_instances`] so chrome like the breadcrumb
    /// bar can re-emit its label on top of the cover rect.
    pub fn post_text_entries(
        &self,
        doc: &PrdDocument,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<DevToolsTextEntry> {
        let mut rects = Vec::new();
        let mut texts = Vec::new();
        if !self.open {
            return texts;
        }
        if self.active_tab == DevToolsTab::Elements {
            let (cx, cy, cw, ch) = self.elements_content_rect(viewport_width, viewport_height);
            let state = self.elements_state();
            let geom = elements::Geometry::compute(&state, cx, cy, cw, ch);
            elements::paint_breadcrumb_into(&mut rects, &mut texts, &state, doc, &geom);
        }
        texts
    }

    /// GPU instances for the in-scene **hover** highlight (the box-model
    /// overlay for the *selected* node is painted from `overlay::paint_panel`).
    /// Returns an empty vec when `highlight_enabled == false` or DevTools is
    /// closed, or no node is hovered in the Elements panel.
    pub fn scene_highlight_instances(&self, doc: &PrdDocument) -> Vec<UiInstance> {
        if !self.open || !self.highlight_enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(hover_line) = self.hovered_element_line {
            if let Some(nid) = elements::node_id_at_line(doc, hover_line as usize, &self.expanded_nodes) {
                if Some(nid) != self.selected_node {
                    if let Some(node) = doc.get_node(nid) {
                        out.extend(highlight_rect(
                            &node.layout.rect,
                            Color::TRANSPARENT,
                            Color::new(1.0, 0.55, 0.20, 1.0),
                            1.0,
                        ));
                    }
                }
            }
        }
        out
    }

    /// Generate text areas for the DevTools overlay.
    pub fn text_entries(
        &self,
        doc: &PrdDocument,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<DevToolsTextEntry> {
        let mut entries = Vec::new();

        // Badge text "PRISM"
        let (bx, by, bw, bh) = self.badge_rect(viewport_width, viewport_height);
        let is_vertical = self.badge_rotation == 90 || self.badge_rotation == 270;
        let badge_color = Color::new(0.55, 0.71, 0.97, 0.65);

        if is_vertical {
            let label = if self.badge_rotation == 270 {
                "PRISM".to_string()
            } else {
                "PRISM".chars().rev().collect::<String>()
            };
            let char_text = label.chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let font_size = 12.0;
            let line_h = font_size * 1.3;
            let text_h = line_h * (label.chars().count() as f32);
            let inset_x = (bw - font_size) / 2.0;
            let inset_y = (bh - text_h) / 2.0;
            entries.push(DevToolsTextEntry {
                text: char_text,
                x: bx + inset_x,
                y: by + inset_y.max(0.0),
                width: font_size + 4.0,
                font_size,
                color: badge_color,
                bold: true,
            });
        } else {
            let label = if self.badge_rotation == 180 {
                "PRISM".chars().rev().collect::<String>()
            } else {
                "PRISM".to_string()
            };
            entries.push(DevToolsTextEntry {
                text: label,
                x: bx + 8.0,
                y: by + 3.0,
                width: bw - 16.0,
                font_size: 11.0,
                color: badge_color,
                bold: false,
            });
        }

        if self.open {
            let panel_y = viewport_height - self.panel_height;

            // Tab labels (with badge counts) — widths come from `tab_layout`
            // so labels never wrap.
            for (tab, tx, tw, label) in self.tab_layout() {
                let is_active = tab == self.active_tab;

                let color = if is_active {
                    Color::WHITE
                } else if matches!(tab, DevToolsTab::Console) && self.console.error_count > 0 {
                    Color::new(1.0, 0.35, 0.35, 0.9)
                } else {
                    Color::new(0.6, 0.6, 0.6, 1.0)
                };

                entries.push(DevToolsTextEntry {
                    text: label,
                    x: tx + 14.0,
                    y: panel_y + 6.0,
                    width: tw - 16.0,
                    font_size: 12.0,
                    color,
                    bold: is_active,
                });
            }

            // Content area
            let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
            let content_h = self.panel_height - overlay::TAB_BAR_HEIGHT;

            match self.active_tab {
                DevToolsTab::Elements => {
                    let state = self.elements_state();
                    elements::text_entries_with_state(
                        &mut entries, doc, &state,
                        0.0, content_y, viewport_width, content_h,
                    );
                    // "Highlight" label next to the checkbox.
                    let (cx, cy, _size) = overlay::highlight_checkbox_box(viewport_width, content_y);
                    entries.push(DevToolsTextEntry {
                        text: "Highlight".to_string(),
                        x: cx - 70.0,
                        y: cy - 1.0,
                        width: 64.0,
                        font_size: 11.0,
                        color: if self.highlight_enabled {
                            Color::WHITE
                        } else {
                            Color::new(0.55, 0.55, 0.6, 1.0)
                        },
                        bold: false,
                    });
                }
                DevToolsTab::Console => {
                    console::text_entries_console(
                        &mut entries, &self.console, 8.0, content_y,
                        viewport_width, content_h, self.console_scroll,
                    );
                }
                DevToolsTab::Gpu => {
                    self.gpu_tab_entries(&mut entries, content_y, viewport_width);
                }
                DevToolsTab::Network => {
                    entries.push(DevToolsTextEntry {
                        text: "No network requests captured.".to_string(),
                        x: 12.0,
                        y: content_y + 12.0,
                        width: viewport_width - 24.0,
                        font_size: 12.0,
                        color: Color::new(0.5, 0.5, 0.5, 1.0),
                        bold: false,
                    });
                }
                DevToolsTab::Sources => {
                    placeholder_text(&mut entries, content_y, content_h, viewport_width,
                        "Sources", "Script files, breakpoints, and step-debugging \u{2014} coming soon.");
                }
                DevToolsTab::Performance => {
                    placeholder_text(&mut entries, content_y, content_h, viewport_width,
                        "Performance", "Frame timeline, flamechart, and CPU profile \u{2014} coming soon.");
                }
                DevToolsTab::Storage => {
                    placeholder_text(&mut entries, content_y, content_h, viewport_width,
                        "Storage", "localStorage, IndexedDB, and IPC namespace inspector \u{2014} coming soon.");
                }
            }
        }

        // Context menu labels (drawn on top of everything else).
        entries.extend(self.context_menu.text_entries());

        // Command palette text (above all panels and the context menu).
        palette::text_entries(&mut entries, &self.palette, viewport_width, viewport_height);

        entries
    }

    /// Generate GPU tab text entries (info + FPS sparkline labels).
    fn gpu_tab_entries(&self, entries: &mut Vec<DevToolsTextEntry>, content_y: f32, viewport_width: f32) {
        let mut y = content_y + 8.0;
        let line_h = 18.0;
        let info_lines = [
            format!("Adapter: {}", self.gpu_info),
            format!("FPS: {:.1}", self.fps),
            format!("Instances: {}", self.instance_count),
            format!("Vertices: {}", self.vertex_count),
            format!("Textures: {}", self.texture_count),
        ];
        for line in &info_lines {
            entries.push(DevToolsTextEntry {
                text: line.clone(),
                x: 12.0,
                y,
                width: viewport_width - 24.0,
                font_size: 12.0,
                color: Color::new(0.8, 0.8, 0.8, 1.0),
                bold: false,
            });
            y += line_h;
        }

        // FPS graph label
        y += 8.0;
        entries.push(DevToolsTextEntry {
            text: "FPS History (last 120 samples)".to_string(),
            x: 12.0,
            y,
            width: viewport_width - 24.0,
            font_size: 11.0,
            color: Color::new(0.5, 0.5, 0.55, 1.0),
            bold: false,
        });
        y += line_h;

        // Min/Max/Avg labels for FPS graph
        if !self.fps_history.is_empty() {
            let min_fps = self.fps_history.iter().cloned().fold(f32::INFINITY, f32::min);
            let max_fps = self.fps_history.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let avg_fps: f32 = self.fps_history.iter().sum::<f32>() / self.fps_history.len() as f32;
            entries.push(DevToolsTextEntry {
                text: format!("Min: {:.0}  Avg: {:.0}  Max: {:.0}", min_fps, avg_fps, max_fps),
                x: 12.0,
                y: y + overlay::FPS_GRAPH_HEIGHT + 4.0,
                width: viewport_width - 24.0,
                font_size: 10.0,
                color: Color::new(0.5, 0.5, 0.55, 1.0),
                bold: false,
            });
        }

        // Frame time stats
        if !self.frame_time_history.is_empty() {
            let avg_ms: f32 = self.frame_time_history.iter().sum::<f32>() / self.frame_time_history.len() as f32;
            let max_ms = self.frame_time_history.iter().cloned().fold(0.0f32, f32::max);
            entries.push(DevToolsTextEntry {
                text: format!("Frame Time — Avg: {:.1}ms  Max: {:.1}ms", avg_ms, max_ms),
                x: 12.0,
                y: y + overlay::FPS_GRAPH_HEIGHT + 18.0,
                width: viewport_width - 24.0,
                font_size: 10.0,
                color: Color::new(0.5, 0.5, 0.55, 1.0),
                bold: false,
            });
        }
    }

    /// Text entries for the context menu overlay (rendered on top of scene text).
    pub fn context_menu_text_entries(&self) -> Vec<DevToolsTextEntry> {
        self.context_menu.text_entries()
    }
}

/// Centered "Coming soon" text for placeholder tabs (Sources/Performance/Storage).
fn placeholder_text(
    out: &mut Vec<DevToolsTextEntry>,
    content_y: f32,
    content_h: f32,
    viewport_width: f32,
    title: &str,
    subtitle: &str,
) {
    let cx = viewport_width * 0.5 - 160.0;
    let cy = content_y + content_h * 0.5 - 24.0;
    out.push(DevToolsTextEntry {
        text: format!("{} \u{2014} coming soon", title),
        x: cx,
        y: cy,
        width: 320.0,
        font_size: 14.0,
        color: Color::new(0.85, 0.86, 0.90, 1.0),
        bold: true,
    });
    out.push(DevToolsTextEntry {
        text: subtitle.to_string(),
        x: cx - 80.0,
        y: cy + 22.0,
        width: 480.0,
        font_size: 11.0,
        color: Color::new(0.55, 0.55, 0.60, 1.0),
        bold: false,
    });
}

/// A text entry to render in the DevTools overlay.
pub struct DevToolsTextEntry {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub font_size: f32,
    pub color: Color,
    pub bold: bool,
}

/// Hit-test the highlight-toggle checkbox in the Elements tab header.
fn hit_highlight_checkbox(x: f32, y: f32, viewport_width: f32, content_y: f32) -> bool {
    let (cx, cy, size) = overlay::highlight_checkbox_box(viewport_width, content_y);
    // Generous hit area to make the small box easy to click.
    let pad = 4.0;
    x >= cx - pad && x <= cx + size + pad && y >= cy - pad && y <= cy + size + pad
}

/// Build a filled-and-outlined highlight rectangle (returns 1 instance).
fn highlight_rect(r: &crate::prd::value::Rect, fill: Color, border: Color, bw: f32) -> Vec<UiInstance> {
    if r.width <= 0.0 || r.height <= 0.0 {
        return Vec::new();
    }
    let mut flags = 0u32;
    if fill.a > 0.0 { flags |= UiInstance::FLAG_HAS_BACKGROUND; }
    if bw > 0.0 && border.a > 0.0 { flags |= UiInstance::FLAG_HAS_BORDER; }
    vec![UiInstance {
        rect: [r.x, r.y, r.width, r.height],
        bg_color: fill.to_array(),
        border_color: border.to_array(),
        border_width: [bw, bw, bw, bw],
        border_radius: [0.0; 4],
        clip_rect: [0.0, 0.0, 99999.0, 99999.0],
        texture_index: -1,
        opacity: 1.0,
        flags,
        _pad: 0,
    }]
}
