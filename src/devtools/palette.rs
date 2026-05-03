// prism-runtime/src/devtools/palette.rs
//
// Command palette (Ctrl+Shift+P) for PRISM DevTools. Centered modal that
// fuzzy-filters a list of named commands. Keyboard-first; mouse hover/click
// also supported. Painted as a GPU overlay; glyphs piped through glyphon.

use crate::gpu::vertex::UiInstance;
use crate::prd::value::Color;
use super::{theme, widgets, DevToolsTextEntry, DevToolsTab};

const W: f32 = 520.0;
const ROW_H: f32 = 28.0;
const HEADER_H: f32 = 38.0;
const MAX_VISIBLE: usize = 10;
const PAD: f32 = 8.0;

/// A single palette command.
#[derive(Debug, Clone)]
pub struct Command {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub action: PaletteAction,
}

/// What the palette asks the host to do when an entry is invoked.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    SwitchTab(DevToolsTab),
    ToggleHighlight,
    Reload,
    ClearConsole,
    FocusElementsSearch,
    CloseDevTools,
}

/// Persistent palette state.
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub hovered: usize,
    commands: Vec<Command>,
}

impl PaletteState {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            hovered: 0,
            commands: default_commands(),
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.hovered = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.hovered = 0;
    }

    /// Indices of commands matching the current query (case-insensitive substring).
    pub fn filtered(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.commands.len()).collect();
        }
        let q = self.query.to_ascii_lowercase();
        self.commands.iter()
            .enumerate()
            .filter(|(_, c)| c.label.to_ascii_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn move_up(&mut self) {
        let len = self.filtered().len();
        if len == 0 { return; }
        if self.hovered == 0 { self.hovered = len - 1; } else { self.hovered -= 1; }
    }

    pub fn move_down(&mut self) {
        let len = self.filtered().len();
        if len == 0 { return; }
        self.hovered = (self.hovered + 1) % len;
    }

    pub fn invoke_selected(&mut self) -> Option<PaletteAction> {
        let f = self.filtered();
        let idx = *f.get(self.hovered)?;
        let action = self.commands[idx].action.clone();
        self.close();
        Some(action)
    }

    pub fn type_char(&mut self, c: char) {
        if c.is_control() { return; }
        self.query.push(c);
        self.hovered = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.hovered = 0;
    }
}

fn default_commands() -> Vec<Command> {
    vec![
        Command { label: "Show Elements", shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Elements) },
        Command { label: "Show Console",  shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Console)  },
        Command { label: "Show Sources",  shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Sources)  },
        Command { label: "Show Network",  shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Network)  },
        Command { label: "Show Performance", shortcut: "",       action: PaletteAction::SwitchTab(DevToolsTab::Performance) },
        Command { label: "Show Storage",  shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Storage)  },
        Command { label: "Show GPU",      shortcut: "",          action: PaletteAction::SwitchTab(DevToolsTab::Gpu)      },
        Command { label: "Toggle Element Highlight", shortcut: "", action: PaletteAction::ToggleHighlight },
        Command { label: "Focus Elements Search", shortcut: "/", action: PaletteAction::FocusElementsSearch },
        Command { label: "Reload Page",   shortcut: "Ctrl+R",    action: PaletteAction::Reload },
        Command { label: "Clear Console", shortcut: "",          action: PaletteAction::ClearConsole },
        Command { label: "Close DevTools", shortcut: "F12",       action: PaletteAction::CloseDevTools },
    ]
}

/// Compute the modal rect (x, y, w, h) for the current viewport.
pub fn rect(viewport_width: f32, viewport_height: f32, visible_rows: usize) -> (f32, f32, f32, f32) {
    let h = HEADER_H + (visible_rows.min(MAX_VISIBLE) as f32 * ROW_H) + PAD * 2.0;
    let x = (viewport_width - W) * 0.5;
    let y = (viewport_height * 0.25).max(40.0);
    (x, y, W, h)
}

/// Paint the palette rects.
pub fn paint(out: &mut Vec<UiInstance>, state: &PaletteState, viewport_width: f32, viewport_height: f32) {
    if !state.open { return; }
    let filt = state.filtered();
    let visible = filt.len().min(MAX_VISIBLE).max(1);
    let (x, y, w, h) = rect(viewport_width, viewport_height, visible);

    // Backdrop dim
    out.push(widgets::rect(0.0, 0.0, viewport_width, viewport_height, Color::new(0.0, 0.0, 0.0, 0.35), None, 0.0));
    // Panel
    out.push(widgets::rect(x, y, w, h, theme::BG_PANEL, Some(theme::LINE), 8.0));

    // Header / search box border
    out.push(widgets::hline(x, y + HEADER_H, w, theme::LINE));

    // Selection highlight
    let list_y = y + HEADER_H + PAD;
    for (i, _) in filt.iter().take(visible).enumerate() {
        if i == state.hovered {
            out.push(widgets::rect(x + PAD, list_y + i as f32 * ROW_H, w - PAD * 2.0, ROW_H,
                theme::BG_ROW_SELECT, None, 4.0));
        }
    }
}

/// Paint the palette text entries.
pub fn text_entries(out: &mut Vec<DevToolsTextEntry>, state: &PaletteState, viewport_width: f32, viewport_height: f32) {
    if !state.open { return; }
    let filt = state.filtered();
    let visible = filt.len().min(MAX_VISIBLE).max(1);
    let (x, y, w, _h) = rect(viewport_width, viewport_height, visible);

    // Search query (or placeholder)
    let (query_text, query_color) = if state.query.is_empty() {
        ("Type to search commands\u{2026}".to_string(), theme::TEXT_MUTED)
    } else {
        (state.query.clone(), theme::TEXT_PRIMARY)
    };
    out.push(DevToolsTextEntry {
        text: query_text,
        x: x + 14.0,
        y: y + 10.0,
        width: w - 28.0,
        font_size: theme::FONT_HEADER,
        color: query_color,
        bold: false,
    });

    // Commands
    let list_y = y + HEADER_H + PAD;
    for (i, &cmd_idx) in filt.iter().take(visible).enumerate() {
        let cmd = &state.commands[cmd_idx];
        let row_y = list_y + i as f32 * ROW_H;
        out.push(DevToolsTextEntry {
            text: cmd.label.to_string(),
            x: x + 18.0,
            y: row_y + 6.0,
            width: w - 140.0,
            font_size: theme::FONT_BODY,
            color: if i == state.hovered { theme::TEXT_PRIMARY } else { theme::TEXT_SECONDARY },
            bold: i == state.hovered,
        });
        if !cmd.shortcut.is_empty() {
            out.push(DevToolsTextEntry {
                text: cmd.shortcut.to_string(),
                x: x + w - 110.0,
                y: row_y + 7.0,
                width: 96.0,
                font_size: theme::FONT_SMALL,
                color: theme::TEXT_MUTED,
                bold: false,
            });
        }
    }

    if filt.is_empty() {
        out.push(DevToolsTextEntry {
            text: "No matching commands".to_string(),
            x: x + 18.0,
            y: list_y + 6.0,
            width: w - 36.0,
            font_size: theme::FONT_BODY,
            color: theme::TEXT_MUTED,
            bold: false,
        });
    }
}

/// Hit-test a click. Returns the index into `filtered()` if a row was clicked,
/// or `Some(usize::MAX)` if the click was inside the panel but not on a row
/// (caller should swallow it). Returns `None` if the click was outside the
/// panel (caller should close the palette).
pub fn hit_test(state: &PaletteState, viewport_width: f32, viewport_height: f32, x: f32, y: f32) -> Option<usize> {
    if !state.open { return None; }
    let filt = state.filtered();
    let visible = filt.len().min(MAX_VISIBLE).max(1);
    let (px, py, pw, ph) = rect(viewport_width, viewport_height, visible);
    if x < px || x > px + pw || y < py || y > py + ph {
        return None;
    }
    let list_y = py + HEADER_H + PAD;
    if y < list_y { return Some(usize::MAX); }
    let i = ((y - list_y) / ROW_H) as usize;
    if i < visible { Some(i) } else { Some(usize::MAX) }
}
