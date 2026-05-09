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
    #[allow(dead_code)] // Used by future console rewrite.
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

/// Phase-1 placeholder. The full Console panel rewrite (formatted objects,
/// stack traces, eager eval, etc.) lives in a follow-up. For now we just
/// render a centered "Coming soon" message.
pub fn text_entries_console(
    out: &mut Vec<DevToolsTextEntry>,
    console: &ConsoleLog,
    _x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    _scroll: f32,
) {
    let cx = viewport_width * 0.5 - 160.0;
    let cy = content_y + content_h * 0.5 - 24.0;
    out.push(DevToolsTextEntry {
        text: "Console — coming soon".to_string(),
        x: cx,
        y: cy,
        width: 320.0,
        font_size: 14.0,
        color: Color::new(0.85, 0.86, 0.90, 1.0),
        bold: true, clip: None,
    });
    out.push(DevToolsTextEntry {
        text: format!(
            "Buffered {} entries ({} errors, {} warnings) — UI under reconstruction.",
            console.entries.len(), console.error_count, console.warn_count,
        ),
        x: cx - 80.0,
        y: cy + 22.0,
        width: 480.0,
        font_size: 11.0,
        color: Color::new(0.55, 0.55, 0.60, 1.0),
        bold: false, clip: None,
    });
}

