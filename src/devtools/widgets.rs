// prism-runtime/src/devtools/widgets.rs
//
// Reusable paint primitives for the DevTools UI. All shapes are emitted as
// `UiInstance`s so they live in the same render layer as the rest of the
// devtools chrome. Text labels are emitted via `DevToolsTextEntry` so the
// caller can batch them with the rest of the panel's text.

use crate::gpu::vertex::UiInstance;
use crate::prd::value::Color;
use super::DevToolsTextEntry;
use super::theme;

// ---------------------------------------------------------------------------
// Filled / outlined rect
// ---------------------------------------------------------------------------

/// Build a filled (and optionally bordered) rect instance with uniform radius.
pub fn rect(
    x: f32, y: f32, w: f32, h: f32,
    bg: Color,
    border: Option<Color>,
    radius: f32,
) -> UiInstance {
    let (bc, bw) = match border {
        Some(c) => (c, 1.0),
        None => (Color::TRANSPARENT, 0.0),
    };
    rect_styled(x, y, w, h, bg, bc, bw, [radius; 4], [0.0, 0.0, 99999.0, 99999.0])
}

/// Build a clipped rect with explicit per-side border width and per-corner radius.
pub fn rect_styled(
    x: f32, y: f32, w: f32, h: f32,
    bg: Color, border: Color, border_w: f32,
    radius: [f32; 4],
    clip: [f32; 4],
) -> UiInstance {
    let mut flags = 0u32;
    if bg.a > 0.0 { flags |= UiInstance::FLAG_HAS_BACKGROUND; }
    if border_w > 0.0 && border.a > 0.0 { flags |= UiInstance::FLAG_HAS_BORDER; }
    UiInstance {
        rect: [x, y, w, h],
        bg_color: bg.to_array(),
        border_color: border.to_array(),
        border_width: [border_w; 4],
        border_radius: radius,
        clip_rect: clip,
        texture_index: -1,
        opacity: 1.0,
        flags,
        _pad: 0,
    }
}

/// Outline only — no fill.
pub fn outline(x: f32, y: f32, w: f32, h: f32, color: Color, weight: f32, radius: f32) -> UiInstance {
    rect_styled(x, y, w, h, Color::TRANSPARENT, color, weight, [radius; 4], [0.0, 0.0, 99999.0, 99999.0])
}

/// Single-pixel horizontal line.
pub fn hline(x: f32, y: f32, w: f32, color: Color) -> UiInstance {
    rect(x, y, w, 1.0, color, None, 0.0)
}

/// Single-pixel vertical line.
pub fn vline(x: f32, y: f32, h: f32, color: Color) -> UiInstance {
    rect(x, y, 1.0, h, color, None, 0.0)
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Push a text entry with sensible defaults.
#[inline]
pub fn text(out: &mut Vec<DevToolsTextEntry>, s: impl Into<String>, x: f32, y: f32, w: f32, size: f32, color: Color) {
    out.push(DevToolsTextEntry {
        text: s.into(), x, y, width: w.max(1.0), font_size: size, color, bold: false, clip: None,
    });
}

/// Bold variant of [`text`].
#[inline]
pub fn text_bold(out: &mut Vec<DevToolsTextEntry>, s: impl Into<String>, x: f32, y: f32, w: f32, size: f32, color: Color) {
    out.push(DevToolsTextEntry {
        text: s.into(), x, y, width: w.max(1.0), font_size: size, color, bold: true, clip: None,
    });
}

// ---------------------------------------------------------------------------
// Button (paint-only — caller hit-tests against the same rect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState { Idle, Hover, Active, Disabled }

/// Paint a chromed button with a centred label. Returns nothing — the caller
/// is responsible for hit-testing the same `(x, y, w, h)` rect.
pub fn button(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    x: f32, y: f32, w: f32, h: f32,
    label: &str,
    state: ButtonState,
) {
    let (bg, fg) = match state {
        ButtonState::Idle     => (theme::BG_TOOLBAR, theme::TEXT_PRIMARY),
        ButtonState::Hover    => (theme::BG_TAB_HOVER, theme::TEXT_PRIMARY),
        ButtonState::Active   => (theme::ACCENT, theme::TEXT_INVERSE),
        ButtonState::Disabled => (theme::BG_TOOLBAR, theme::TEXT_DISABLED),
    };
    rects.push(rect(x, y, w, h, bg, Some(theme::LINE_SOFT), 3.0));
    // Approximate centred-y for an 11pt cap-height in our row metric.
    let ty = y + (h - 11.0) * 0.5 - 1.0;
    text(texts, label, x + theme::SP_3, ty, w - theme::SP_3 * 2.0, theme::FONT_SMALL, fg);
}

/// Paint a small icon button (12pt glyph) with hover/active state.
pub fn icon_button(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    x: f32, y: f32, size: f32,
    glyph: &str,
    state: ButtonState,
) {
    let bg = match state {
        ButtonState::Idle     => Color::TRANSPARENT,
        ButtonState::Hover    => theme::BG_ROW_HOVER,
        ButtonState::Active   => theme::ACCENT_DIM,
        ButtonState::Disabled => Color::TRANSPARENT,
    };
    let fg = match state {
        ButtonState::Disabled => theme::TEXT_DISABLED,
        ButtonState::Active   => theme::ACCENT,
        _                     => theme::TEXT_SECONDARY,
    };
    if bg.a > 0.0 {
        rects.push(rect(x, y, size, size, bg, None, 3.0));
    }
    let glyph_size = (size - 6.0).max(10.0);
    text(texts, glyph, x + (size - glyph_size) * 0.5 - 1.0, y + (size - glyph_size) * 0.5 - 1.0, size, glyph_size, fg);
}

// ---------------------------------------------------------------------------
// Scrollbar
// ---------------------------------------------------------------------------

/// Paint a vertical scrollbar (track + thumb). No-op when content fits.
pub fn vscrollbar(
    rects: &mut Vec<UiInstance>,
    track_x: f32, area_y: f32, area_h: f32,
    total_content: f32, scroll: f32,
) {
    if total_content <= area_h || area_h <= 1.0 { return; }
    rects.push(rect(track_x, area_y, theme::SCROLLBAR_W, area_h, theme::alpha(theme::LINE_SOFT, 0.4), None, 3.0));
    let thumb_h = ((area_h * area_h) / total_content).max(theme::SCROLLBAR_MIN);
    let scroll_range = (total_content - area_h).max(1.0);
    let thumb_y = area_y + (scroll / scroll_range).clamp(0.0, 1.0) * (area_h - thumb_h);
    rects.push(rect(track_x, thumb_y, theme::SCROLLBAR_W, thumb_h, theme::alpha(theme::TEXT_MUTED, 0.6), None, 3.0));
}

/// Paint a horizontal scrollbar (track + thumb). No-op when content fits.
pub fn hscrollbar(
    rects: &mut Vec<UiInstance>,
    area_x: f32, track_y: f32, area_w: f32,
    total_content: f32, scroll: f32,
) {
    if total_content <= area_w || area_w <= 1.0 { return; }
    rects.push(rect(area_x, track_y, area_w, theme::SCROLLBAR_W, theme::alpha(theme::LINE_SOFT, 0.4), None, 3.0));
    let thumb_w = ((area_w * area_w) / total_content).max(theme::SCROLLBAR_MIN);
    let scroll_range = (total_content - area_w).max(1.0);
    let thumb_x = area_x + (scroll / scroll_range).clamp(0.0, 1.0) * (area_w - thumb_w);
    rects.push(rect(thumb_x, track_y, thumb_w, theme::SCROLLBAR_W, theme::alpha(theme::TEXT_MUTED, 0.6), None, 3.0));
}

// ---------------------------------------------------------------------------
// Tab strip
// ---------------------------------------------------------------------------

pub struct TabSpec<'a> {
    pub label: &'a str,
    pub is_active: bool,
    pub is_hover: bool,
    pub badge: Option<&'a str>,
    pub badge_color: Color,
}

/// Paint the tab bar background and tabs at `(bar_x, bar_y)` with width `bar_w`.
/// Returns the x-positions of each tab so the caller can hit-test them.
pub fn tab_bar(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    bar_x: f32, bar_y: f32, bar_w: f32,
    tabs: &[TabSpec],
) -> Vec<(f32, f32)> {
    rects.push(rect(bar_x, bar_y, bar_w, theme::TAB_BAR_HEIGHT, theme::BG_TAB_BAR, None, 0.0));
    rects.push(hline(bar_x, bar_y + theme::TAB_BAR_HEIGHT - 1.0, bar_w, theme::LINE));

    let mut positions = Vec::with_capacity(tabs.len());
    let mut x = bar_x + theme::SP_2;
    for tab in tabs {
        let label_w = (tab.label.len() as f32 * 6.5).max(40.0);
        let badge_w = tab.badge.map(|b| b.len() as f32 * 6.0 + 10.0).unwrap_or(0.0);
        let w = label_w + badge_w + theme::TAB_PADDING_X * 2.0;

        if tab.is_active {
            rects.push(rect(x, bar_y, w, theme::TAB_BAR_HEIGHT, theme::BG_TAB_ACTIVE, None, 0.0));
            // Top accent
            rects.push(rect(x, bar_y, w, theme::TAB_INDICATOR, theme::ACCENT, None, 0.0));
        } else if tab.is_hover {
            rects.push(rect(x, bar_y, w, theme::TAB_BAR_HEIGHT, theme::BG_TAB_HOVER, None, 0.0));
        }

        let fg = if tab.is_active { theme::TEXT_PRIMARY } else { theme::TEXT_SECONDARY };
        text(texts, tab.label, x + theme::TAB_PADDING_X, bar_y + 8.0, label_w, theme::FONT_BODY, fg);

        if let Some(b) = tab.badge {
            let bx = x + theme::TAB_PADDING_X + label_w + 4.0;
            let bw = b.len() as f32 * 6.0 + 6.0;
            rects.push(rect(bx, bar_y + 7.0, bw, 14.0, tab.badge_color, None, 7.0));
            text(texts, b, bx + 4.0, bar_y + 8.0, bw - 4.0, theme::FONT_TINY, theme::TEXT_INVERSE);
        }

        positions.push((x, x + w));
        x += w + theme::SP_1;
    }
    positions
}

// ---------------------------------------------------------------------------
// Search box
// ---------------------------------------------------------------------------

/// Paint a search input with a magnifier glyph and current query.
pub fn search_box(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    x: f32, y: f32, w: f32, h: f32,
    query: &str,
    placeholder: &str,
    focused: bool,
) {
    let border = if focused { theme::ACCENT } else { theme::LINE };
    rects.push(rect(x, y, w, h, theme::BG_INPUT, Some(border), 4.0));
    text(texts, "\u{1F50D}", x + 6.0, y + (h - 12.0) * 0.5 - 1.0, 14.0, 11.0, theme::TEXT_MUTED);
    let inner_x = x + 24.0;
    let inner_w = (w - 28.0).max(10.0);
    if query.is_empty() {
        text(texts, placeholder, inner_x, y + (h - 11.0) * 0.5 - 1.0, inner_w, theme::FONT_SMALL, theme::TEXT_DISABLED);
    } else {
        text(texts, query, inner_x, y + (h - 11.0) * 0.5 - 1.0, inner_w, theme::FONT_SMALL, theme::TEXT_PRIMARY);
        if focused {
            // Caret at end of text (approximate width 6px/char).
            let cx = inner_x + (query.len() as f32 * 6.2).min(inner_w - 4.0);
            rects.push(rect(cx, y + 4.0, 1.0, h - 8.0, theme::TEXT_PRIMARY, None, 0.0));
        }
    }
}

// ---------------------------------------------------------------------------
// Filter chips
// ---------------------------------------------------------------------------

pub struct ChipSpec<'a> {
    pub label: &'a str,
    pub active: bool,
    pub count: Option<u32>,
    pub color: Color,
}

/// Paint a row of filter chips. Returns x-ranges for hit-testing.
pub fn chips(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    x: f32, y: f32, h: f32,
    chips_in: &[ChipSpec],
) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(chips_in.len());
    let mut cx = x;
    for chip in chips_in {
        let label = if let Some(n) = chip.count {
            format!("{} ({})", chip.label, n)
        } else {
            chip.label.to_string()
        };
        let w = label.len() as f32 * 6.5 + 18.0;
        let bg = if chip.active { theme::alpha(chip.color, 0.18) } else { theme::BG_TOOLBAR };
        let border = if chip.active { chip.color } else { theme::LINE };
        rects.push(rect(cx, y, w, h, bg, Some(border), h * 0.5));
        let fg = if chip.active { chip.color } else { theme::TEXT_SECONDARY };
        text(texts, &label, cx + 9.0, y + (h - 11.0) * 0.5 - 1.0, w - 16.0, theme::FONT_SMALL, fg);
        out.push((cx, cx + w));
        cx += w + theme::SP_2;
    }
    out
}
