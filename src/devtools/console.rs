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

/// Height of a single console row (text + a hairline separator).
pub const CONSOLE_ROW_HEIGHT: f32 = 18.0;

/// Public iterator over filtered entries (oldest → newest).
pub fn filtered_entries<'a>(console: &'a ConsoleLog) -> Vec<&'a ConsoleEntry> {
    console.entries.iter().filter(|e| match console.filter {
        ConsoleFilter::All      => true,
        ConsoleFilter::Errors   => e.level == LogLevel::Error,
        ConsoleFilter::Warnings => matches!(e.level, LogLevel::Warn | LogLevel::Error),
        ConsoleFilter::Info     => matches!(e.level, LogLevel::Info | LogLevel::Log),
    }).collect()
}

fn level_color(level: LogLevel) -> Color {
    use crate::devtools::theme;
    match level {
        LogLevel::Error => theme::SEVERE_ERROR,
        LogLevel::Warn  => theme::SEVERE_WARN,
        LogLevel::Info  => theme::SEVERE_INFO,
        LogLevel::Log   => theme::TEXT_PRIMARY,
    }
}

fn level_bg_tint(level: LogLevel) -> Option<Color> {
    match level {
        LogLevel::Error => Some(Color { r: 0.50, g: 0.10, b: 0.12, a: 0.18 }),
        LogLevel::Warn  => Some(Color { r: 0.55, g: 0.40, b: 0.04, a: 0.14 }),
        _ => None,
    }
}

fn level_glyph(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "\u{2716}", // ✖
        LogLevel::Warn  => "\u{26A0}", // ⚠
        LogLevel::Info  => "\u{2139}", // ℹ
        LogLevel::Log   => "\u{2022}", // •
    }
}

/// Paint rectangles for the console body: per-row tint stripes for
/// errors/warnings and the filter-bar pill backgrounds. The filter bar
/// background and the global scrollbar are painted in `overlay.rs`.
pub fn paint_rects_console(
    out: &mut Vec<crate::gpu::vertex::UiInstance>,
    console: &ConsoleLog,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
) {
    use crate::devtools::theme;
    use crate::devtools::overlay::make_rect_instance_pub as mk;

    // Filter pills along the bar.
    let pills_y = content_y + 3.0;
    let pills_h = CONSOLE_FILTER_BAR_HEIGHT - 6.0;
    let mut x = 6.0;
    for f in [ConsoleFilter::All, ConsoleFilter::Errors, ConsoleFilter::Warnings, ConsoleFilter::Info] {
        let label = match f {
            ConsoleFilter::All      => "All",
            ConsoleFilter::Errors   => "Errors",
            ConsoleFilter::Warnings => "Warnings",
            ConsoleFilter::Info     => "Info / Log",
        };
        let w = (label.len() as f32) * 6.5 + 16.0;
        let active = console.filter == f;
        out.push(mk(
            x, pills_y, w, pills_h,
            if active { theme::BG_TAB_ACTIVE } else { theme::BG_TAB_BAR },
            if active { Some(theme::ACCENT) } else { Some(theme::LINE_SOFT) },
            4.0,
        ));
        x += w + 6.0;
    }

    // Row tints — body starts after filter bar.
    let body_y = content_y + CONSOLE_FILTER_BAR_HEIGHT;
    let body_h = content_h - CONSOLE_FILTER_BAR_HEIGHT;
    if body_h <= 0.0 { return; }
    let entries = filtered_entries(console);
    let scroll_top = scroll.max(0.0);
    let scroll_bot = scroll_top + body_h;

    for (i, e) in entries.iter().enumerate() {
        if let Some(bg) = level_bg_tint(e.level) {
            let row_top = i as f32 * CONSOLE_ROW_HEIGHT;
            let row_bot = row_top + CONSOLE_ROW_HEIGHT;
            if row_bot < scroll_top || row_top > scroll_bot { continue; }
            let y = body_y + (row_top - scroll_top);
            out.push(mk(
                0.0, y, viewport_width, CONSOLE_ROW_HEIGHT,
                bg, None, 0.0,
            ));
        }
    }

    // Filter-bar bottom hairline already drawn by overlay.rs.
    let _ = scroll_bot;
}

/// Render text rows for the Console tab. Lays out one entry per line:
///   `[hh:mm:ss.mmm]  ✖  message…`
/// Entries are filtered, scrolled, and clipped to the panel content area.
pub fn text_entries_console(
    out: &mut Vec<DevToolsTextEntry>,
    console: &ConsoleLog,
    _x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
) {
    use crate::devtools::theme;

    // ── Filter bar labels ───────────────────────────────────────────
    let pills_y = content_y + 6.0;
    let mut px = 6.0;
    for f in [ConsoleFilter::All, ConsoleFilter::Errors, ConsoleFilter::Warnings, ConsoleFilter::Info] {
        let label = match f {
            ConsoleFilter::All      => "All",
            ConsoleFilter::Errors   => "Errors",
            ConsoleFilter::Warnings => "Warnings",
            ConsoleFilter::Info     => "Info / Log",
        };
        let w = (label.len() as f32) * 6.5 + 16.0;
        let active = console.filter == f;
        out.push(DevToolsTextEntry {
            text: label.to_string(),
            x: px + 8.0,
            y: pills_y,
            width: w,
            font_size: 11.0,
            color: if active { Color::WHITE } else { theme::TEXT_SECONDARY },
            bold: active,
            clip: None,
        });
        px += w + 6.0;
    }

    // Counts on the right side of the filter bar.
    let counts = format!(
        "{} entries  ·  {} errors  ·  {} warnings",
        console.entries.len(), console.error_count, console.warn_count,
    );
    out.push(DevToolsTextEntry {
        text: counts,
        x: viewport_width - 260.0,
        y: pills_y,
        width: 252.0,
        font_size: 11.0,
        color: theme::TEXT_MUTED,
        bold: false,
        clip: None,
    });

    // ── Body ────────────────────────────────────────────────────────
    let body_y = content_y + CONSOLE_FILTER_BAR_HEIGHT;
    let body_h = content_h - CONSOLE_FILTER_BAR_HEIGHT;
    if body_h <= 0.0 { return; }
    let body_clip = Some([0.0_f32, body_y, viewport_width, body_y + body_h]);

    let entries = filtered_entries(console);
    if entries.is_empty() {
        out.push(DevToolsTextEntry {
            text: if console.entries.is_empty() {
                "No console messages yet.".to_string()
            } else {
                "No messages match the current filter.".to_string()
            },
            x: 12.0,
            y: body_y + 12.0,
            width: viewport_width - 24.0,
            font_size: 11.0,
            color: theme::TEXT_MUTED,
            bold: false,
            clip: body_clip,
        });
        return;
    }

    let scroll_top = scroll.max(0.0);
    let scroll_bot = scroll_top + body_h;
    let ts_x   = 8.0;
    let glyph_x = 78.0;
    let msg_x  = 96.0;

    for (i, e) in entries.iter().enumerate() {
        let row_top = i as f32 * CONSOLE_ROW_HEIGHT;
        let row_bot = row_top + CONSOLE_ROW_HEIGHT;
        if row_bot < scroll_top || row_top > scroll_bot { continue; }
        let y = body_y + (row_top - scroll_top) + 3.0;

        // Timestamp (relative seconds, 3 decimals).
        let secs = e.timestamp_ms / 1000.0;
        out.push(DevToolsTextEntry {
            text: format!("{:>8.3}s", secs),
            x: ts_x,
            y,
            width: 64.0,
            font_size: 11.0,
            color: theme::TEXT_MUTED,
            bold: false,
            clip: body_clip,
        });
        // Level glyph in the level color.
        out.push(DevToolsTextEntry {
            text: level_glyph(e.level).to_string(),
            x: glyph_x,
            y,
            width: 14.0,
            font_size: 12.0,
            color: level_color(e.level),
            bold: true,
            clip: body_clip,
        });
        // Message — we render the whole string on one line; long messages
        // are clipped horizontally by the body clip rect.
        out.push(DevToolsTextEntry {
            text: e.message.clone(),
            x: msg_x,
            y,
            width: viewport_width - msg_x - 18.0,
            font_size: 12.0,
            color: match e.level {
                LogLevel::Error => theme::SEVERE_ERROR,
                LogLevel::Warn  => theme::TEXT_PRIMARY,
                _ => theme::TEXT_PRIMARY,
            },
            bold: false,
            clip: body_clip,
        });
    }
}

