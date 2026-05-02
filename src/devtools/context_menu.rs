// prism-runtime/src/devtools/context_menu.rs
//
// In-window right-click context menu rendered as a GPU overlay.
// Provides access to DevTools, Reload, and Exit without a system tray.

use crate::prd::value::Color;
use crate::gpu::vertex::UiInstance;
use super::DevToolsTextEntry;

// Visual constants.
const MENU_WIDTH: f32 = 220.0;
const ITEM_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const MENU_PADDING: f32 = 4.0;
const MENU_RADIUS: f32 = 6.0;

const BG: Color = Color { r: 0.10, g: 0.10, b: 0.12, a: 0.96 };
const HOVER_BG: Color = Color { r: 0.20, g: 0.20, b: 0.28, a: 1.0 };
const BORDER: Color = Color { r: 0.25, g: 0.25, b: 0.30, a: 1.0 };
const SEPARATOR_COLOR: Color = Color { r: 0.22, g: 0.22, b: 0.28, a: 1.0 };
const TEXT_COLOR: Color = Color { r: 0.85, g: 0.85, b: 0.88, a: 1.0 };
const TEXT_DISABLED: Color = Color { r: 0.45, g: 0.45, b: 0.48, a: 1.0 };
const ACCENT: Color = Color { r: 0.39, g: 0.40, b: 0.95, a: 1.0 };

/// An item in the context menu.
#[derive(Debug, Clone)]
pub enum ContextMenuEntry {
    Item {
        label: String,
        shortcut: Option<String>,
        action: ContextAction,
        enabled: bool,
    },
    Separator,
}

/// Actions that can be triggered from the context menu.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextAction {
    ToggleDevTools,
    PopoutDevTools,
    InspectElement,
    DebugServer,
    Reload,
    Home,
    Back,
    Forward,
    Exit,
    /// Navigate to a named page/route.
    NavigateTo(String),
    /// Evaluate a JavaScript expression in the active page.
    Eval(String),
}

/// The in-window context menu state.
pub struct ContextMenu {
    /// Whether the menu is visible.
    pub open: bool,
    /// Top-left position (in logical pixels).
    pub x: f32,
    pub y: f32,
    /// Index of the currently hovered item (None if no hover).
    pub hovered: Option<usize>,
    /// The menu entries.
    entries: Vec<ContextMenuEntry>,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            open: false,
            x: 0.0,
            y: 0.0,
            hovered: None,
            entries: Self::default_entries(),
        }
    }

    fn default_entries() -> Vec<ContextMenuEntry> {
        Self::built_in_entries(&[])
    }

    /// Built-in entries with optional name-based hiding. Recognised hide
    /// names (case-insensitive): `inspect`, `devtools`, `popout-devtools`,
    /// `debug-server`, `home`, `back`, `forward`, `reload`, `exit`.
    fn built_in_entries(hide: &[String]) -> Vec<ContextMenuEntry> {
        let hidden: std::collections::HashSet<String> =
            hide.iter().map(|s| s.trim().to_ascii_lowercase()).collect();

        let candidates: &[(&str, &str, Option<&str>, ContextAction)] = &[
            ("inspect",         "Inspect Element", Some("Ctrl+Shift+C"), ContextAction::InspectElement),
            ("devtools",        "DevTools",        Some("F12"),          ContextAction::ToggleDevTools),
            ("popout-devtools", "Pop Out DevTools", None,                ContextAction::PopoutDevTools),
            ("debug-server",    "Debug Server",    None,                 ContextAction::DebugServer),
        ];
        let nav_candidates: &[(&str, &str, Option<&str>, ContextAction)] = &[
            ("home",    "Home",    None,              ContextAction::Home),
            ("back",    "Back",    Some("Alt+Left"),  ContextAction::Back),
            ("forward", "Forward", Some("Alt+Right"), ContextAction::Forward),
            ("reload",  "Reload",  Some("Ctrl+R"),    ContextAction::Reload),
        ];

        let mut out: Vec<ContextMenuEntry> = Vec::new();
        for (id, label, sc, action) in candidates {
            if hidden.contains(*id) { continue; }
            out.push(ContextMenuEntry::Item {
                label: label.to_string(),
                shortcut: sc.map(|s| s.to_string()),
                action: action.clone(),
                enabled: true,
            });
        }
        if !out.is_empty() {
            out.push(ContextMenuEntry::Separator);
        }
        let nav_start = out.len();
        for (id, label, sc, action) in nav_candidates {
            if hidden.contains(*id) { continue; }
            out.push(ContextMenuEntry::Item {
                label: label.to_string(),
                shortcut: sc.map(|s| s.to_string()),
                action: action.clone(),
                enabled: true,
            });
        }
        if out.len() > nav_start {
            out.push(ContextMenuEntry::Separator);
        }
        if !hidden.contains("exit") {
            out.push(ContextMenuEntry::Item {
                label: "Exit".to_string(),
                shortcut: None,
                action: ContextAction::Exit,
                enabled: true,
            });
        }
        // Trim trailing separator if any.
        if matches!(out.last(), Some(ContextMenuEntry::Separator)) {
            out.pop();
        }
        out
    }

    /// Build a context menu from a list of built-in hide names + extra items.
    pub fn with_config(extra_items: Vec<ContextMenuEntry>, hide_defaults: &[String]) -> Self {
        let mut entries = Self::built_in_entries(hide_defaults);
        if !extra_items.is_empty() {
            // Insert one separator between built-ins and extras (unless the
            // built-in list is empty or already ends in one).
            if !entries.is_empty() && !matches!(entries.last(), Some(ContextMenuEntry::Separator)) {
                entries.push(ContextMenuEntry::Separator);
            }
            entries.extend(extra_items);
        }
        Self {
            open: false,
            x: 0.0,
            y: 0.0,
            hovered: None,
            entries,
        }
    }

    /// Show the context menu at the given position.
    /// Adjusts position if the menu would overflow the viewport.
    pub fn show(&mut self, x: f32, y: f32, vw: f32, vh: f32) {
        let menu_h = self.total_height();
        // Clamp so the menu stays within the viewport.
        self.x = x.min(vw - MENU_WIDTH - 4.0).max(0.0);
        self.y = y.min(vh - menu_h - 4.0).max(0.0);
        self.hovered = None;
        self.open = true;
    }

    /// Hide the context menu.
    pub fn hide(&mut self) {
        self.open = false;
        self.hovered = None;
    }

    /// Update hover state based on mouse position.
    pub fn update_hover(&mut self, mx: f32, my: f32) {
        if !self.open {
            return;
        }
        self.hovered = self.hit_test_item(mx, my);
    }

    /// Check if a point is inside the context menu area.
    pub fn hit_test(&self, mx: f32, my: f32) -> bool {
        if !self.open {
            return false;
        }
        let h = self.total_height();
        mx >= self.x && mx <= self.x + MENU_WIDTH && my >= self.y && my <= self.y + h
    }

    /// Get the action for a click at the given position, or None.
    pub fn click(&mut self, mx: f32, my: f32) -> Option<ContextAction> {
        if !self.open {
            return None;
        }
        if let Some(idx) = self.hit_test_item(mx, my) {
            if let Some(ContextMenuEntry::Item { action, enabled, .. }) = self.entries.get(idx) {
                if *enabled {
                    let action = action.clone();
                    self.hide();
                    return Some(action);
                }
            }
        }
        // Click outside menu → dismiss.
        self.hide();
        None
    }

    /// Compute which item index (if any) is at position (mx, my).
    fn hit_test_item(&self, mx: f32, my: f32) -> Option<usize> {
        if mx < self.x || mx > self.x + MENU_WIDTH {
            return None;
        }
        let mut y = self.y + MENU_PADDING;
        for (i, entry) in self.entries.iter().enumerate() {
            let h = match entry {
                ContextMenuEntry::Item { .. } => ITEM_HEIGHT,
                ContextMenuEntry::Separator => SEPARATOR_HEIGHT,
            };
            if my >= y && my < y + h {
                return match entry {
                    ContextMenuEntry::Item { enabled, .. } if *enabled => Some(i),
                    _ => None,
                };
            }
            y += h;
        }
        None
    }

    fn total_height(&self) -> f32 {
        let content: f32 = self.entries.iter().map(|e| match e {
            ContextMenuEntry::Item { .. } => ITEM_HEIGHT,
            ContextMenuEntry::Separator => SEPARATOR_HEIGHT,
        }).sum();
        content + MENU_PADDING * 2.0
    }

    /// Returns the bounding rect `(x, y, w, h)` if the context menu is open, else `None`.
    pub fn overlay_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if !self.open {
            return None;
        }
        Some((self.x, self.y, MENU_WIDTH, self.total_height()))
    }

    /// Generate GPU instances for the context menu.
    pub fn paint(&self) -> Vec<UiInstance> {
        if !self.open {
            return Vec::new();
        }
        let mut out = Vec::new();
        let menu_h = self.total_height();

        // Background
        out.push(make_rect(self.x, self.y, MENU_WIDTH, menu_h, BG, Some(BORDER), MENU_RADIUS));

        // Items
        let mut y = self.y + MENU_PADDING;
        for (i, entry) in self.entries.iter().enumerate() {
            match entry {
                ContextMenuEntry::Item { .. } => {
                    // Hover highlight
                    if self.hovered == Some(i) {
                        out.push(make_rect(
                            self.x + MENU_PADDING,
                            y,
                            MENU_WIDTH - MENU_PADDING * 2.0,
                            ITEM_HEIGHT,
                            HOVER_BG,
                            None,
                            4.0,
                        ));
                    }
                    y += ITEM_HEIGHT;
                }
                ContextMenuEntry::Separator => {
                    // Separator line
                    out.push(make_rect(
                        self.x + 12.0,
                        y + SEPARATOR_HEIGHT / 2.0 - 0.5,
                        MENU_WIDTH - 24.0,
                        1.0,
                        SEPARATOR_COLOR,
                        None,
                        0.0,
                    ));
                    y += SEPARATOR_HEIGHT;
                }
            }
        }

        out
    }

    /// Generate text entries for the context menu.
    pub fn text_entries(&self) -> Vec<DevToolsTextEntry> {
        if !self.open {
            return Vec::new();
        }
        let mut entries = Vec::new();
        let mut y = self.y + MENU_PADDING;

        for (i, entry) in self.entries.iter().enumerate() {
            match entry {
                ContextMenuEntry::Item { label, shortcut, enabled, action } => {
                    let color = if !enabled {
                        TEXT_DISABLED
                    } else if self.hovered == Some(i) {
                        ACCENT
                    } else {
                        TEXT_COLOR
                    };
                    entries.push(DevToolsTextEntry {
                        text: label.clone(),
                        x: self.x + 14.0,
                        y: y + 5.0,
                        width: MENU_WIDTH - 28.0,
                        font_size: 12.0,
                        color,
                        bold: matches!(action, ContextAction::InspectElement) && self.hovered == Some(i),
                    });
                    // Shortcut hint (right-aligned, dimmer)
                    if let Some(sc) = shortcut {
                        entries.push(DevToolsTextEntry {
                            text: sc.clone(),
                            x: self.x + MENU_WIDTH - 90.0,
                            y: y + 6.0,
                            width: 76.0,
                            font_size: 10.0,
                            color: if self.hovered == Some(i) {
                                Color::new(0.6, 0.6, 0.7, 1.0)
                            } else {
                                TEXT_DISABLED
                            },
                            bold: false,
                        });
                    }
                    y += ITEM_HEIGHT;
                }
                ContextMenuEntry::Separator => {
                    y += SEPARATOR_HEIGHT;
                }
            }
        }

        entries
    }
}

fn make_rect(
    x: f32, y: f32, w: f32, h: f32,
    bg: Color,
    border: Option<Color>,
    radius: f32,
) -> UiInstance {
    let bc = border.unwrap_or(Color::TRANSPARENT);
    let bw = if border.is_some() { 1.0 } else { 0.0 };
    let mut flags = 0u32;
    if bg.a > 0.0 { flags |= UiInstance::FLAG_HAS_BACKGROUND; }
    if bw > 0.0 { flags |= UiInstance::FLAG_HAS_BORDER; }
    UiInstance {
        rect: [x, y, w, h],
        bg_color: bg.to_array(),
        border_color: bc.to_array(),
        border_width: [bw, bw, bw, bw],
        border_radius: [radius, radius, radius, radius],
        clip_rect: [0.0, 0.0, 99999.0, 99999.0],
        texture_index: -1,
        opacity: 1.0,
        flags,
        _pad: 0,
    }
}

