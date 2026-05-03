// prism-runtime/src/devtools/overlay.rs
//
// Paints the OpenRender badge and DevTools panel as GPU instances.
// Includes: panel background, tab bar, scrollbar, resize handle,
// FPS sparkline graph, element highlight overlay, and console filter bar.

use crate::prd::document::PrdDocument;
use crate::prd::value::Color;
use crate::gpu::vertex::UiInstance;
use super::DevTools;
use super::DevToolsTab;

// Badge dimensions and position.
pub const BADGE_WIDTH: f32 = 72.0;
pub const BADGE_HEIGHT: f32 = 22.0;
pub const BADGE_MARGIN: f32 = 8.0;

// Panel dimensions.
pub const PANEL_HEIGHT: f32 = 300.0;
pub const TAB_BAR_HEIGHT: f32 = 30.0;
pub const TAB_WIDTH: f32 = 90.0;

// FPS graph.
pub const FPS_GRAPH_HEIGHT: f32 = 60.0;
const FPS_GRAPH_X: f32 = 12.0;
const FPS_GRAPH_WIDTH: f32 = 360.0;

// Scrollbar.
const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_MIN_THUMB: f32 = 16.0;

/// Colors used in the DevTools UI.
const BG_DARK: Color = Color { r: 0.07, g: 0.07, b: 0.07, a: 0.95 };
const BG_TAB_BAR: Color = Color { r: 0.10, g: 0.10, b: 0.10, a: 1.0 };
const BG_TAB_ACTIVE: Color = Color { r: 0.15, g: 0.15, b: 0.15, a: 1.0 };
const BORDER_COLOR: Color = Color { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
const ACCENT: Color = Color { r: 0.39, g: 0.40, b: 0.95, a: 1.0 }; // #6366f1
const SCROLLBAR_TRACK: Color = Color { r: 0.12, g: 0.12, b: 0.14, a: 0.5 };
const SCROLLBAR_THUMB: Color = Color { r: 0.35, g: 0.35, b: 0.40, a: 0.7 };
const HIGHLIGHT_COLOR: Color = Color { r: 0.25, g: 0.45, b: 0.85, a: 0.25 };
const HIGHLIGHT_BORDER: Color = Color { r: 0.35, g: 0.55, b: 0.95, a: 0.6 };
const RESIZE_HANDLE: Color = Color { r: 0.3, g: 0.3, b: 0.35, a: 0.8 };
const FILTER_BAR_BG: Color = Color { r: 0.09, g: 0.09, b: 0.11, a: 1.0 };

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

/// Paint the "PRISM" badge — no background/border, just a hit target.
pub fn paint_badge(
    _out: &mut Vec<UiInstance>,
    _viewport_width: f32,
    _viewport_height: f32,
) {
    // Text rendered via text_entries().
}

/// Paint the full DevTools panel.
pub fn paint_panel(
    out: &mut Vec<UiInstance>,
    devtools: &DevTools,
    doc: &PrdDocument,
    viewport_width: f32,
    viewport_height: f32,
) {
    let panel_h = devtools.panel_height;
    let panel_y = viewport_height - panel_h;

    // Resize handle (top 3px of panel)
    out.push(make_rect_instance(
        0.0, panel_y - 1.0, viewport_width, 3.0,
        RESIZE_HANDLE,
        None,
        0.0,
    ));

    // Panel background
    out.push(make_rect_instance(
        0.0, panel_y, viewport_width, panel_h,
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

    // Tab buttons — widths come from `DevTools::tab_layout()` so the active
    // pill matches the label rendered in `mod.rs::text_entries`.
    for (tab, tx, tw, _label) in devtools.tab_layout() {
        let is_active = tab == devtools.active_tab;
        if is_active {
            out.push(make_rect_instance(
                tx, panel_y, tw, TAB_BAR_HEIGHT,
                BG_TAB_ACTIVE,
                None,
                0.0,
            ));
            out.push(make_rect_instance(
                tx, panel_y + TAB_BAR_HEIGHT - 2.0, tw, 2.0,
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

    let content_y = panel_y + TAB_BAR_HEIGHT;
    let content_h = panel_h - TAB_BAR_HEIGHT;

    // Tab-specific overlays
    match devtools.active_tab {
        DevToolsTab::Console => {
            // Console filter bar background
            out.push(make_rect_instance(
                0.0, content_y, viewport_width, super::console::CONSOLE_FILTER_BAR_HEIGHT,
                FILTER_BAR_BG,
                None,
                0.0,
            ));
            // Filter bar bottom separator
            out.push(make_rect_instance(
                0.0, content_y + super::console::CONSOLE_FILTER_BAR_HEIGHT - 1.0,
                viewport_width, 1.0,
                BORDER_COLOR,
                None,
                0.0,
            ));

            // Console scrollbar
            let total = devtools.console.total_content_height() + super::console::CONSOLE_FILTER_BAR_HEIGHT;
            let scroll_area_y = content_y + super::console::CONSOLE_FILTER_BAR_HEIGHT;
            let scroll_area_h = content_h - super::console::CONSOLE_FILTER_BAR_HEIGHT;
            paint_scrollbar(out, viewport_width, scroll_area_y, scroll_area_h, total, devtools.console_scroll);
        }
        DevToolsTab::Elements => {
            // Delegate the entire panel content (tree, sidebar, breadcrumb,
            // scrollbar, search box, etc.) to the new elements pipeline.
            let state = devtools.elements_state();
            super::elements::paint_rects_with_state(
                out, doc, &state,
                /*content_x*/ 0.0, content_y,
                viewport_width, content_h,
            );

            // Highlight checkbox (top-right of panel content area) — kept
            // separate because its hit-test lives in mod.rs.
            paint_highlight_checkbox(out, viewport_width, content_y, devtools.highlight_enabled);

            // Element highlight overlay on the viewport (above the scene, below
            // the panel) — Chrome-style margin/padding/content tinting.
            if devtools.highlight_enabled {
            if let Some(sel_id) = devtools.selected_node {
                if let Some(node) = doc.get_node(sel_id) {
                    let r = &node.layout.rect;
                    if r.width > 0.0 && r.height > 0.0 {
                        // Semi-transparent fill
                        out.push(make_rect_instance(
                            r.x, r.y, r.width, r.height,
                            HIGHLIGHT_COLOR,
                            Some(HIGHLIGHT_BORDER),
                            0.0,
                        ));
                        // Margin overlay (orange tint)
                        let m = &node.layout.margin;
                        if m.top > 0.0 {
                            out.push(make_rect_instance(
                                r.x - m.left, r.y - m.top, r.width + m.left + m.right, m.top,
                                Color { r: 0.9, g: 0.6, b: 0.2, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if m.bottom > 0.0 {
                            out.push(make_rect_instance(
                                r.x - m.left, r.y + r.height, r.width + m.left + m.right, m.bottom,
                                Color { r: 0.9, g: 0.6, b: 0.2, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if m.left > 0.0 {
                            out.push(make_rect_instance(
                                r.x - m.left, r.y, m.left, r.height,
                                Color { r: 0.9, g: 0.6, b: 0.2, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if m.right > 0.0 {
                            out.push(make_rect_instance(
                                r.x + r.width, r.y, m.right, r.height,
                                Color { r: 0.9, g: 0.6, b: 0.2, a: 0.15 }, None, 0.0,
                            ));
                        }
                        // Padding overlay (green tint)
                        let p = &node.layout.padding;
                        let cr = &node.layout.content_rect;
                        if p.top > 0.0 {
                            out.push(make_rect_instance(
                                cr.x, r.y + node.style.border_width.top, cr.width, p.top,
                                Color { r: 0.3, g: 0.8, b: 0.3, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if p.bottom > 0.0 {
                            out.push(make_rect_instance(
                                cr.x, cr.y + cr.height, cr.width, p.bottom,
                                Color { r: 0.3, g: 0.8, b: 0.3, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if p.left > 0.0 {
                            out.push(make_rect_instance(
                                r.x + node.style.border_width.left, cr.y, p.left, cr.height,
                                Color { r: 0.3, g: 0.8, b: 0.3, a: 0.15 }, None, 0.0,
                            ));
                        }
                        if p.right > 0.0 {
                            out.push(make_rect_instance(
                                cr.x + cr.width, cr.y, p.right, cr.height,
                                Color { r: 0.3, g: 0.8, b: 0.3, a: 0.15 }, None, 0.0,
                            ));
                        }
                    }
                }
            }
            } // end if devtools.highlight_enabled
        }
        DevToolsTab::Gpu => {
            // FPS sparkline graph
            paint_fps_graph(out, devtools, content_y);
        }
        _ => {}
    }
}

/// Paint a scrollbar track + thumb.
fn paint_scrollbar(
    out: &mut Vec<UiInstance>,
    viewport_width: f32,
    area_y: f32,
    area_h: f32,
    total_content: f32,
    scroll: f32,
) {
    if total_content <= area_h {
        return; // no scroll needed
    }

    let track_x = viewport_width - SCROLLBAR_WIDTH - 2.0;
    // Track
    out.push(make_rect_instance(
        track_x, area_y, SCROLLBAR_WIDTH, area_h,
        SCROLLBAR_TRACK,
        None,
        3.0,
    ));

    // Thumb
    let thumb_ratio = area_h / total_content;
    let thumb_h = (area_h * thumb_ratio).max(SCROLLBAR_MIN_THUMB);
    let scroll_range = total_content - area_h;
    let thumb_offset = if scroll_range > 0.0 {
        (scroll / scroll_range) * (area_h - thumb_h)
    } else {
        0.0
    };
    out.push(make_rect_instance(
        track_x, area_y + thumb_offset, SCROLLBAR_WIDTH, thumb_h,
        SCROLLBAR_THUMB,
        None,
        3.0,
    ));
}

/// Paint the FPS sparkline graph (bar chart).
fn paint_fps_graph(
    out: &mut Vec<UiInstance>,
    devtools: &DevTools,
    content_y: f32,
) {
    if devtools.fps_history.is_empty() {
        return;
    }

    // Graph area starts below the text info lines
    let graph_y = content_y + 8.0 + 18.0 * 5.0 + 8.0 + 18.0; // after 5 info lines + label
    let graph_w = FPS_GRAPH_WIDTH;
    let graph_h = FPS_GRAPH_HEIGHT;

    // Graph background
    out.push(make_rect_instance(
        FPS_GRAPH_X, graph_y, graph_w, graph_h,
        Color { r: 0.05, g: 0.05, b: 0.08, a: 0.8 },
        Some(Color { r: 0.15, g: 0.15, b: 0.2, a: 1.0 }),
        2.0,
    ));

    // Target line at 60 FPS
    let max_fps = devtools.fps_history.iter().cloned().fold(0.0f32, f32::max).max(60.0);
    let target_y = graph_y + graph_h - (60.0 / max_fps * graph_h);
    if target_y > graph_y && target_y < graph_y + graph_h {
        out.push(make_rect_instance(
            FPS_GRAPH_X, target_y, graph_w, 1.0,
            Color { r: 0.3, g: 0.5, b: 0.3, a: 0.5 },
            None,
            0.0,
        ));
    }

    // Bars
    let bar_w = (graph_w / 120.0).max(1.0);
    for (i, &fps) in devtools.fps_history.iter().enumerate() {
        let ratio = (fps / max_fps).clamp(0.0, 1.0);
        let bar_h = ratio * (graph_h - 2.0);
        let bx = FPS_GRAPH_X + 1.0 + i as f32 * bar_w;
        let by = graph_y + graph_h - 1.0 - bar_h;

        // Color: green > 50fps, yellow 30-50, red < 30
        let color = if fps >= 50.0 {
            Color { r: 0.2, g: 0.8, b: 0.3, a: 0.8 }
        } else if fps >= 30.0 {
            Color { r: 0.9, g: 0.7, b: 0.2, a: 0.8 }
        } else {
            Color { r: 0.9, g: 0.2, b: 0.2, a: 0.8 }
        };

        out.push(make_rect_instance(
            bx, by, bar_w.max(2.0), bar_h,
            color,
            None,
            0.0,
        ));
    }
}



// ---------------------------------------------------------------------------
// Highlight checkbox (Elements tab)
// ---------------------------------------------------------------------------

pub const HIGHLIGHT_CHECKBOX_SIZE: f32 = 14.0;
pub const HIGHLIGHT_CHECKBOX_RIGHT_INSET: f32 = SCROLLBAR_WIDTH + 110.0;

/// Top-left corner of the highlight checkbox box (square only — label sits to its left).
pub fn highlight_checkbox_box(viewport_width: f32, content_y: f32) -> (f32, f32, f32) {
    let x = viewport_width - HIGHLIGHT_CHECKBOX_RIGHT_INSET;
    let y = content_y + 6.0;
    (x, y, HIGHLIGHT_CHECKBOX_SIZE)
}

fn paint_highlight_checkbox(out: &mut Vec<UiInstance>, viewport_width: f32, content_y: f32, on: bool) {
    let (cx, cy, size) = highlight_checkbox_box(viewport_width, content_y);
    out.push(make_rect_instance(
        cx, cy, size, size,
        if on { ACCENT } else { Color { r: 0.18, g: 0.18, b: 0.22, a: 1.0 } },
        Some(BORDER_COLOR),
        3.0,
    ));
    if on {
        // Inner check square.
        let pad = 3.0;
        out.push(make_rect_instance(
            cx + pad, cy + pad, size - pad * 2.0, size - pad * 2.0,
            Color::WHITE,
            None,
            1.5,
        ));
    }
}
