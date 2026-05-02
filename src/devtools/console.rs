// prism-runtime/src/devtools/console.rs
//
// Console log capture for the OpenRender DevTools.
// Stores log entries with level, message, and timestamp.

use std::time::Instant;
use crate::prd::value::Color;
use super::DevToolsTextEntry;

/// Log level for console entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Log,
    Info,
    Warn,
    Error,
}

/// Active log level filter for the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleFilter {
    All,
    Errors,
    Warnings,
    Info,
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
    /// Monotonic clock base for timestamps.
    start_time: Instant,
    /// Active filter.
    pub filter: ConsoleFilter,
    /// Cached error count.
    pub error_count: u32,
    /// Cached warning count.
    pub warn_count: u32,
    /// Whether new entries were added since last frame (for auto-scroll).
    pub has_new_entries: bool,
}

impl ConsoleLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 1000,
            start_time: Instant::now(),
            filter: ConsoleFilter::All,
            error_count: 0,
            warn_count: 0,
            has_new_entries: false,
        }
    }

    /// Add a log entry.
    pub fn log(&mut self, level: LogLevel, message: String) {
        let timestamp_ms = self.start_time.elapsed().as_secs_f64() * 1000.0;
        if self.entries.len() >= self.max_entries {
            // Track counts for evicted entry.
            let evicted = &self.entries[0];
            match evicted.level {
                LogLevel::Error => self.error_count = self.error_count.saturating_sub(1),
                LogLevel::Warn => self.warn_count = self.warn_count.saturating_sub(1),
                _ => {}
            }
            self.entries.remove(0);
        }
        match level {
            LogLevel::Error => self.error_count += 1,
            LogLevel::Warn => self.warn_count += 1,
            _ => {}
        }
        self.entries.push(ConsoleEntry {
            level,
            message,
            timestamp_ms,
        });
        self.has_new_entries = true;
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.error_count = 0;
        self.warn_count = 0;
    }

    /// Total content height for scroll calculations.
    pub fn total_content_height(&self) -> f32 {
        let filtered = self.filtered_count();
        filtered as f32 * 18.0
    }

    /// Count of entries matching the current filter.
    fn filtered_count(&self) -> usize {
        match self.filter {
            ConsoleFilter::All => self.entries.len(),
            ConsoleFilter::Errors => self.entries.iter().filter(|e| e.level == LogLevel::Error).count(),
            ConsoleFilter::Warnings => self.entries.iter().filter(|e| matches!(e.level, LogLevel::Warn | LogLevel::Error)).count(),
            ConsoleFilter::Info => self.entries.iter().filter(|e| matches!(e.level, LogLevel::Info | LogLevel::Log)).count(),
        }
    }

    /// Check if an entry passes the current filter.
    fn passes_filter(&self, entry: &ConsoleEntry) -> bool {
        match self.filter {
            ConsoleFilter::All => true,
            ConsoleFilter::Errors => entry.level == LogLevel::Error,
            ConsoleFilter::Warnings => matches!(entry.level, LogLevel::Warn | LogLevel::Error),
            ConsoleFilter::Info => matches!(entry.level, LogLevel::Info | LogLevel::Log),
        }
    }

    /// Cycle to the next filter.
    pub fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            ConsoleFilter::All => ConsoleFilter::Errors,
            ConsoleFilter::Errors => ConsoleFilter::Warnings,
            ConsoleFilter::Warnings => ConsoleFilter::Info,
            ConsoleFilter::Info => ConsoleFilter::All,
        };
    }

    /// Label for the filter button.
    pub fn filter_label(&self) -> &'static str {
        match self.filter {
            ConsoleFilter::All => "All",
            ConsoleFilter::Errors => "Errors",
            ConsoleFilter::Warnings => "Warnings",
            ConsoleFilter::Info => "Info",
        }
    }
}

/// Height of the filter bar above console entries.
pub const CONSOLE_FILTER_BAR_HEIGHT: f32 = 24.0;

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
    let filter_y = content_y;
    let entries_y = content_y + CONSOLE_FILTER_BAR_HEIGHT;
    let entries_h = content_h - CONSOLE_FILTER_BAR_HEIGHT;

    // Filter bar: [All] [Errors (N)] [Warnings (N)]  and clear button
    let filter_label = match console.filter {
        ConsoleFilter::All => format!("Filter: All ({})", console.entries.len()),
        ConsoleFilter::Errors => format!("Filter: Errors ({})", console.error_count),
        ConsoleFilter::Warnings => format!("Filter: Warnings ({})", console.warn_count),
        ConsoleFilter::Info => format!("Filter: Info"),
    };
    out.push(DevToolsTextEntry {
        text: filter_label,
        x: x + 4.0,
        y: filter_y + 4.0,
        width: 200.0,
        font_size: 11.0,
        color: Color::new(0.55, 0.55, 0.60, 1.0),
        bold: false,
    });

    // Error/warning summary on the right side of filter bar
    if console.error_count > 0 || console.warn_count > 0 {
        let summary = format!(
            "{}{}",
            if console.error_count > 0 { format!("{} errors  ", console.error_count) } else { String::new() },
            if console.warn_count > 0 { format!("{} warnings", console.warn_count) } else { String::new() },
        );
        out.push(DevToolsTextEntry {
            text: summary,
            x: viewport_width - 180.0,
            y: filter_y + 4.0,
            width: 170.0,
            font_size: 11.0,
            color: if console.error_count > 0 {
                Color::new(1.0, 0.35, 0.35, 0.8)
            } else {
                Color::new(1.0, 0.85, 0.3, 0.8)
            },
            bold: false,
        });
    }

    let line_h = 18.0;
    let visible_start = scroll;
    let visible_end = scroll + entries_h;

    // Collect filtered entries
    let filtered: Vec<(usize, &ConsoleEntry)> = console.entries.iter()
        .enumerate()
        .filter(|(_, e)| console.passes_filter(e))
        .collect();

    if filtered.is_empty() {
        out.push(DevToolsTextEntry {
            text: "No console output.".to_string(),
            x: x + 4.0,
            y: entries_y + 12.0,
            width: viewport_width - x - 16.0,
            font_size: 12.0,
            color: Color::new(0.4, 0.4, 0.4, 1.0),
            bold: false,
        });
        return;
    }

    for (vi, (_orig_idx, entry)) in filtered.iter().enumerate() {
        let entry_y = vi as f32 * line_h;
        if entry_y + line_h < visible_start || entry_y > visible_end {
            continue;
        }
        let render_y = entries_y + 4.0 + entry_y - scroll;

        // Format timestamp
        let secs = entry.timestamp_ms / 1000.0;
        let mins = (secs / 60.0).floor() as u32;
        let s = secs % 60.0;
        let ts = format!("{:02}:{:05.2}", mins, s);

        let (prefix, color) = match entry.level {
            LogLevel::Log => ("", Color::new(0.75, 0.75, 0.75, 1.0)),
            LogLevel::Info => ("[info] ", Color::new(0.4, 0.7, 1.0, 1.0)),
            LogLevel::Warn => ("[warn] ", Color::new(1.0, 0.85, 0.3, 1.0)),
            LogLevel::Error => ("[error] ", Color::new(1.0, 0.35, 0.35, 1.0)),
        };

        out.push(DevToolsTextEntry {
            text: format!("{} {}{}", ts, prefix, entry.message),
            x: x + 4.0,
            y: render_y,
            width: viewport_width - x - 16.0,
            font_size: 11.0,
            color,
            bold: matches!(entry.level, LogLevel::Error),
        });
    }
}

