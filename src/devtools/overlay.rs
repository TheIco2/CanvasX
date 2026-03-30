// openrender-runtime/src/devtools/overlay.rs
//
// Paints the OpenRender badge and DevTools panel as GPU instances.

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::value::Color;
use crate::gpu::vertex::UiInstance;
use super::DevTools;

// Badge dimensions and position.
pub const BADGE_WIDTH: f32 = 72.0;
pub const BADGE_HEIGHT: f32 = 22.0;
pub const BADGE_MARGIN: f32 = 8.0;

// Panel dimensions.
pub const PANEL_HEIGHT: f32 = 300.0;
pub const TAB_BAR_HEIGHT: f32 = 30.0;
pub const TAB_WIDTH: f32 = 90.0;

/// Colors used in the DevTools UI.
const BG_DARK: Color = Color { r: 0.07, g: 0.07, b: 0.07, a: 0.95 };
const BG_TAB_BAR: Color = Color { r: 0.10, g: 0.10, b: 0.10, a: 1.0 };
const BG_TAB_ACTIVE: Color = Color { r: 0.15, g: 0.15, b: 0.15, a: 1.0 };
const BORDER_COLOR: Color = Color { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
const ACCENT: Color = Color { r: 0.39, g: 0.40, b: 0.95, a: 1.0 }; // #6366f1

fn make_rect_instance(
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

/// Paint the "OpenRender" badge in the bottom-left corner.
/// Renders as dim text only — no background or border.
pub fn paint_badge(
    _out: &mut Vec<UiInstance>,
    _viewport_width: f32,
    _viewport_height: f32,
) {
    // No background or border — just a hit target for click detection.
    // The text is rendered separately via text_entries().
}

/// Paint the full DevTools panel (tab bar + content area).
pub fn paint_panel(
    out: &mut Vec<UiInstance>,
    devtools: &DevTools,
    _doc: &CxrdDocument,
    viewport_width: f32,
    viewport_height: f32,
) {
    let panel_y = viewport_height - PANEL_HEIGHT;

    // Panel background
    out.push(make_rect_instance(
        0.0, panel_y, viewport_width, PANEL_HEIGHT,
        BG_DARK,
        Some(BORDER_COLOR),
        0.0,
    ));

    // Tab bar background
    out.push(make_rect_instance(
        0.0, panel_y, viewport_width, TAB_BAR_HEIGHT,
        BG_TAB_BAR,
        None,
        0.0,
    ));

    // Tab buttons
    let tabs = devtools.visible_tabs();
    for (i, tab) in tabs.iter().enumerate() {
        let tx = i as f32 * TAB_WIDTH;
        let is_active = *tab == devtools.active_tab;
        if is_active {
            // Active tab highlight
            out.push(make_rect_instance(
                tx, panel_y, TAB_WIDTH, TAB_BAR_HEIGHT,
                BG_TAB_ACTIVE,
                None,
                0.0,
            ));
            // Accent underline
            out.push(make_rect_instance(
                tx, panel_y + TAB_BAR_HEIGHT - 2.0, TAB_WIDTH, 2.0,
                ACCENT,
                None,
                0.0,
            ));
        }
    }

    // Tab bar bottom border
    out.push(make_rect_instance(
        0.0, panel_y + TAB_BAR_HEIGHT, viewport_width, 1.0,
        BORDER_COLOR,
        None,
        0.0,
    ));
}
