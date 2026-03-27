// canvasx-runtime/src/devtools/mod.rs
//
// Built-in developer tools for CanvasX Runtime.
// Provides an Elements panel (DOM tree view), Console (logs/errors),
// GPU info, and Network panel. Activated by clicking the "CanvasX"
// watermark badge in the bottom-left corner.

pub mod overlay;
pub mod console;
pub mod elements;
pub mod context_menu;
pub mod debug_server;

use crate::gpu::vertex::UiInstance;
use crate::cxrd::document::CxrdDocument;
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
    /// Selected node ID in elements panel (for highlighting).
    pub selected_node: Option<u32>,
    /// Right-click context menu state.
    pub context_menu: context_menu::ContextMenu,
    /// Badge anchor position (default: BottomRight).
    pub badge_position: BadgePosition,
    /// Badge rotation in degrees (0, 90, 180, 270). Controls text flow direction.
    pub badge_rotation: u16,
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
            context_menu: context_menu::ContextMenu::new(),
            badge_position: BadgePosition::BottomRight,
            badge_rotation: 0,
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

    /// Compute the badge bounding rect (x, y, w, h) based on position and rotation.
    fn badge_rect(&self, viewport_width: f32, viewport_height: f32) -> (f32, f32, f32, f32) {
        let m = overlay::BADGE_MARGIN;
        let is_vertical = self.badge_rotation == 90 || self.badge_rotation == 270;
        // When rotated 90/270, swap width/height for the bounding box.
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

    /// Check if a click at (x, y) hits the CanvasX watermark badge.
    pub fn hit_test_badge(&self, x: f32, y: f32, viewport_width: f32, viewport_height: f32) -> bool {
        let (bx, by, bw, bh) = self.badge_rect(viewport_width, viewport_height);
        x >= bx && x <= bx + bw && y >= by && y <= by + bh
    }

    /// Check if a click at (x, y) hits one of the tab buttons.
    /// Returns Some(tab) if hit, None otherwise.
    pub fn hit_test_tab(&self, x: f32, y: f32, _viewport_width: f32, viewport_height: f32) -> Option<DevToolsTab> {
        if !self.open {
            return None;
        }
        let panel_x = 0.0;
        let panel_y = viewport_height - overlay::PANEL_HEIGHT;
        let tab_y = panel_y;
        let tab_h = overlay::TAB_BAR_HEIGHT;

        if y < tab_y || y > tab_y + tab_h {
            return None;
        }

        let tabs = self.visible_tabs();
        let tab_width = overlay::TAB_WIDTH;
        for (i, tab) in tabs.iter().enumerate() {
            let tx = panel_x + i as f32 * tab_width;
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
        y >= viewport_height - overlay::PANEL_HEIGHT
    }

    /// Generate GPU instances for the DevTools overlay (badge + panel).
    pub fn paint(
        &self,
        doc: &CxrdDocument,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<UiInstance> {
        let mut instances = Vec::new();

        // Always paint the badge.
        overlay::paint_badge(&mut instances, viewport_width, viewport_height);

        // Paint the panel if open.
        if self.open {
            overlay::paint_panel(
                &mut instances,
                self,
                doc,
                viewport_width,
                viewport_height,
            );
        }

        // Context menu instances are rendered separately in the overlay layer
        // via context_menu_instances() so they appear on top of scene text.

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

        // Badge text "CanvasX"
        let (bx, by, bw, bh) = self.badge_rect(viewport_width, viewport_height);
        let is_vertical = self.badge_rotation == 90 || self.badge_rotation == 270;
        let badge_color = Color::new(0.45, 0.45, 0.50, 0.5);

        if is_vertical {
            // Vertical text: render each character stacked on its own line.
            let label = if self.badge_rotation == 270 {
                // 270°: read top-to-bottom (reversed so first char is at top)
                "CanvasX".to_string()
            } else {
                // 90°: read bottom-to-top
                "CanvasX".chars().rev().collect::<String>()
            };
            let char_text = label.chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let font_size = 11.0;
            let line_h = font_size * 1.3;
            let text_h = line_h * 7.0; // 7 chars
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
            // Horizontal text (0° or 180°).
            let label = if self.badge_rotation == 180 {
                "CanvasX".chars().rev().collect::<String>()
            } else {
                "CanvasX".to_string()
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
            let panel_y = viewport_height - overlay::PANEL_HEIGHT;

            // Tab labels
            let tabs = self.visible_tabs();
            for (i, tab) in tabs.iter().enumerate() {
                let tx = i as f32 * overlay::TAB_WIDTH;
                let is_active = *tab == self.active_tab;
                let label = match tab {
                    DevToolsTab::Elements => "Elements",
                    DevToolsTab::Console => "Console",
                    DevToolsTab::Gpu => "GPU",
                    DevToolsTab::Network => "Network",
                };
                entries.push(DevToolsTextEntry {
                    text: label.to_string(),
                    x: tx + 12.0,
                    y: panel_y + 6.0,
                    width: overlay::TAB_WIDTH - 24.0,
                    font_size: 12.0,
                    color: if is_active {
                        Color::WHITE
                    } else {
                        Color::new(0.6, 0.6, 0.6, 1.0)
                    },
                    bold: is_active,
                });
            }

            // Content area
            let content_y = panel_y + overlay::TAB_BAR_HEIGHT;
            let content_h = overlay::PANEL_HEIGHT - overlay::TAB_BAR_HEIGHT;

            match self.active_tab {
                DevToolsTab::Elements => {
                    elements::text_entries_elements(
                        &mut entries, doc, 8.0, content_y,
                        viewport_width, content_h, self.elements_scroll,
                        self.selected_node,
                    );
                }
                DevToolsTab::Console => {
                    console::text_entries_console(
                        &mut entries, &self.console, 8.0, content_y,
                        viewport_width, content_h, self.console_scroll,
                    );
                }
                DevToolsTab::Gpu => {
                    let mut y = content_y + 8.0;
                    let line_h = 18.0;
                    let info_lines = [
                        format!("Adapter: {}", self.gpu_info),
                        format!("FPS: {:.1}", self.fps),
                        format!("Draw Calls: {}", self.draw_calls),
                        format!("Instances: {}", self.instance_count),
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

        // Context menu text is rendered separately in the overlay layer
        // via context_menu_text_entries() so it appears on top of scene text.

        entries
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
