// openrender-runtime/src/devtools/console.rs
//
// Console log capture for the OpenRender DevTools.
// Stores log entries with level, message, and timestamp.

use crate::cxrd::value::Color;
use super::DevToolsTextEntry;

/// Log level for console entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Log,
    Info,
    Warn,
    Error,
}

/// A single console log entry.
#[derive(Debug, Clone)]
pub struct ConsoleEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp_ms: f64,
}

/// The console log buffer.
pub struct ConsoleLog {
    pub entries: Vec<ConsoleEntry>,
    /// Maximum number of entries to keep.
    max_entries: usize,
}

impl ConsoleLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 1000,
        }
    }

    /// Add a log entry.
    pub fn log(&mut self, level: LogLevel, message: String) {
        // Use a monotonic counter as timestamp (actual time comes from runtime).
        let timestamp_ms = self.entries.len() as f64;
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(ConsoleEntry {
            level,
            message,
            timestamp_ms,
        });
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Generate text entries for the console panel.
pub fn text_entries_console(
    out: &mut Vec<DevToolsTextEntry>,
    console: &ConsoleLog,
    x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
) {
    let line_h = 18.0;
    let visible_start = scroll;
    let visible_end = scroll + content_h;

    if console.entries.is_empty() {
        out.push(DevToolsTextEntry {
            text: "No console output.".to_string(),
            x: x + 4.0,
            y: content_y + 12.0,
            width: viewport_width - x - 16.0,
            font_size: 12.0,
            color: Color::new(0.4, 0.4, 0.4, 1.0),
            bold: false,
        });
        return;
    }

    for (i, entry) in console.entries.iter().enumerate() {
        let entry_y = i as f32 * line_h;
        if entry_y + line_h < visible_start || entry_y > visible_end {
            continue;
        }
        let render_y = content_y + 4.0 + entry_y - scroll;

        let (prefix, color) = match entry.level {
            LogLevel::Log => ("", Color::new(0.75, 0.75, 0.75, 1.0)),
            LogLevel::Info => ("[info] ", Color::new(0.4, 0.7, 1.0, 1.0)),
            LogLevel::Warn => ("[warn] ", Color::new(1.0, 0.85, 0.3, 1.0)),
            LogLevel::Error => ("[error] ", Color::new(1.0, 0.35, 0.35, 1.0)),
        };

        out.push(DevToolsTextEntry {
            text: format!("{}{}", prefix, entry.message),
            x: x + 4.0,
            y: render_y,
            width: viewport_width - x - 16.0,
            font_size: 11.0,
            color,
            bold: matches!(entry.level, LogLevel::Error),
        });
    }
}
