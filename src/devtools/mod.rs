// openrender-runtime/src/devtools/mod.rs
//
// Built-in developer tools for OpenRender Runtime.
// Provides an Elements panel (DOM tree view with collapsible nodes and
// computed-styles sidebar), Console (logs/errors with filtering),
// GPU info (with FPS graph), and Network panel.
// Activated by clicking the "OpenRender" badge or pressing F12.

pub mod overlay;
pub mod console;
pub mod elements;
pub mod context_menu;
pub mod debug_server;

use std::collections::HashSet;
use crate::gpu::vertex::UiInstance;
use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::NodeId;
use crate::cxrd::value::Color;

/// Which DevTools tab is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsTab {
    Elements,
    Console,
    Gpu,
    Network,
}

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

        let tabs = self.visible_tabs();
        let tab_width = overlay::TAB_WIDTH;
        for (i, tab) in tabs.iter().enumerate() {
            let tx = i as f32 * tab_width;
            if x >= tx && x <= tx + tab_width {
                return Some(*tab);
            }
        }
        None
    }

    /// Returns the list of visible tabs (Network only if has_network).
    pub fn visible_tabs(&self) -> Vec<DevToolsTab> {
        let mut tabs = vec![DevToolsTab::Elements, DevToolsTab::Console, DevToolsTab::Gpu];
        if self.has_network {
            tabs.push(DevToolsTab::Network);
        }
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
    pub fn handle_elements_click(&mut self, x: f32, y: f32, viewport_height: f32, doc: &CxrdDocument) -> bool {
        if !self.open || self.active_tab != DevToolsTab::Elements {
            return false;
        }
        let panel_y = viewport_height - self.panel_height;
        let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
        let content_h = self.panel_height - overlay::TAB_BAR_HEIGHT;

        if y < content_y || y > content_y + content_h {
            return false;
        }

        let line_h = 16.0;
        let relative_y = (y - content_y - 4.0) + self.elements_scroll;
        if relative_y < 0.0 {
            return false;
        }
        let line_idx = (relative_y / line_h) as usize;

        if let Some(nid) = elements::node_id_at_line(doc, line_idx, &self.expanded_nodes) {
            // If clicking on a node with children, toggle expand/collapse
            if elements::node_has_children_at_line(doc, line_idx, &self.expanded_nodes) {
                // Check if click is in the arrow region (roughly the first 16px of the line)
                let depth_line = line_idx as f32 * line_h;
                let _indent = 12.0 + line_idx as f32; // rough
                // Simple: if already selected, toggle expand. Otherwise, select.
                if self.selected_node == Some(nid) {
                    // Toggle expand/collapse
                    if self.expanded_nodes.contains(&nid) {
                        self.expanded_nodes.remove(&nid);
                    } else {
                        self.expanded_nodes.insert(nid);
                    }
                } else {
                    self.selected_node = Some(nid);
                    // Auto-expand on first click
                    self.expanded_nodes.insert(nid);
                }
            } else {
                self.selected_node = Some(nid);
            }
            return true;
        }
        false
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

        let line_h = 16.0;
        let relative_y = (y - content_y - 4.0) + self.elements_scroll;
        if relative_y < 0.0 {
            self.hovered_element_line = None;
            return;
        }
        self.hovered_element_line = Some((relative_y / line_h) as u32);
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
        doc: &CxrdDocument,
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

        instances
    }

    /// GPU instances for the context menu overlay (rendered on top of scene text).
    pub fn context_menu_instances(&self) -> Vec<UiInstance> {
        self.context_menu.paint()
    }

    /// Generate text areas for the DevTools overlay.
    pub fn text_entries(
        &self,
        doc: &CxrdDocument,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<DevToolsTextEntry> {
        let mut entries = Vec::new();

        // Badge text "OpenRender"
        let (bx, by, bw, bh) = self.badge_rect(viewport_width, viewport_height);
        let is_vertical = self.badge_rotation == 90 || self.badge_rotation == 270;
        let badge_color = Color::new(0.45, 0.45, 0.50, 0.5);

        if is_vertical {
            let label = if self.badge_rotation == 270 {
                "OpenRender".to_string()
            } else {
                "OpenRender".chars().rev().collect::<String>()
            };
            let char_text = label.chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let font_size = 11.0;
            let line_h = font_size * 1.3;
            let text_h = line_h * 7.0;
            let inset_x = (bw - font_size) / 2.0;
            let inset_y = (bh - text_h) / 2.0;
            entries.push(DevToolsTextEntry {
                text: char_text,
                x: bx + inset_x,
                y: by + inset_y.max(0.0),
                width: font_size + 4.0,
                font_size,
                color: badge_color,
                bold: false,
            });
        } else {
            let label = if self.badge_rotation == 180 {
                "OpenRender".chars().rev().collect::<String>()
            } else {
                "OpenRender".to_string()
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

            // Tab labels (with badge counts)
            let tabs = self.visible_tabs();
            for (i, tab) in tabs.iter().enumerate() {
                let tx = i as f32 * overlay::TAB_WIDTH;
                let is_active = *tab == self.active_tab;
                let label = match tab {
                    DevToolsTab::Elements => "Elements".to_string(),
                    DevToolsTab::Console => {
                        if self.console.error_count > 0 {
                            format!("Console ({})", self.console.error_count)
                        } else {
                            "Console".to_string()
                        }
                    }
                    DevToolsTab::Gpu => "GPU".to_string(),
                    DevToolsTab::Network => "Network".to_string(),
                };

                let color = if is_active {
                    Color::WHITE
                } else if matches!(tab, DevToolsTab::Console) && self.console.error_count > 0 {
                    Color::new(1.0, 0.35, 0.35, 0.9)
                } else {
                    Color::new(0.6, 0.6, 0.6, 1.0)
                };

                entries.push(DevToolsTextEntry {
                    text: label,
                    x: tx + 12.0,
                    y: panel_y + 6.0,
                    width: overlay::TAB_WIDTH - 24.0,
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
                    elements::text_entries_elements(
                        &mut entries, doc, 8.0, content_y,
                        viewport_width, content_h, self.elements_scroll,
                        self.selected_node, &self.expanded_nodes,
                        self.hovered_element_line,
                    );
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
            }
        }

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
