// canvasx-runtime/src/scene/paint.rs
//
// Paint pass — converts a laid-out CXRD tree into a flat list of UiInstance
// draw calls for the GPU renderer. Depth-first traversal respects z-index
// and stacking context.

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeId, NodeKind, CxrdNode};
use crate::cxrd::input::{InputKind, ButtonVariant, CheckboxStyle};
use crate::cxrd::style::{Display, Background};
use crate::gpu::vertex::UiInstance;

/// Paint the entire document into a list of GPU instances.
pub fn paint_document(doc: &CxrdDocument) -> Vec<UiInstance> {
    let mut instances = Vec::with_capacity(doc.nodes.len());
    paint_node(doc, doc.root, &mut instances);
    instances
}

/// Recursively paint a node and its children.
fn paint_node(doc: &CxrdDocument, node_id: NodeId, out: &mut Vec<UiInstance>) {
    let node = match doc.get_node(node_id) {
        Some(n) => n,
        None => return,
    };

    if matches!(node.style.display, Display::None) {
        return;
    }

    // Only emit an instance if the node has visible visual properties.
    if should_paint(node) {
        out.push(node_to_instance(node));
    }

    // For Input nodes, emit extra widget-specific quads.
    if let NodeKind::Input(ref input) = node.kind {
        paint_input_widget(node, input, out);
    }

    // For Canvas nodes, emit a textured quad (texture upload handled by main loop).
    if let NodeKind::Canvas { .. } = &node.kind {
        // Emit a full-rect textured quad. The texture_index will be patched by
        // the main loop after uploading the canvas pixmap from the JS runtime.
        // We use texture_index = -1 as a placeholder; main.rs replaces it with
        // the actual GPU texture slot once the canvas pixmap is uploaded.
        let r = &node.layout.rect;
        if r.width > 0.0 && r.height > 0.0 {
            out.push(UiInstance {
                rect: [r.x, r.y, r.width, r.height],
                bg_color: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [0.0; 4],
                clip_rect: [0.0, 0.0, 99999.0, 99999.0],
                texture_index: -1, // patched by main loop
                opacity: node.style.opacity,
                flags: UiInstance::FLAG_HAS_BACKGROUND | UiInstance::FLAG_HAS_TEXTURE,
                _pad: 0,
            });
        }
    }

    // Paint children (sorted by z-index for correct stacking).
    let mut child_ids = node.children.clone();
    child_ids.sort_by_key(|&cid| {
        doc.get_node(cid).map(|c| c.style.z_index).unwrap_or(0)
    });

    for cid in child_ids {
        paint_node(doc, cid, out);
    }
}

/// Does this node need a GPU draw call?
fn should_paint(node: &CxrdNode) -> bool {
    let s = &node.style;

    // Non-zero size?
    let has_size = node.layout.rect.width > 0.0 && node.layout.rect.height > 0.0;
    if !has_size { return false; }

    // Input widgets always paint (they have intrinsic visuals).
    if matches!(node.kind, NodeKind::Input(_)) {
        return true;
    }

    // Has background?
    let has_bg = !matches!(s.background, Background::None);

    // Has border?
    let has_border = s.border_width.top > 0.0
        || s.border_width.right > 0.0
        || s.border_width.bottom > 0.0
        || s.border_width.left > 0.0;

    // Has box shadow? (TODO: separate pass for shadows)
    let has_shadow = !s.box_shadow.is_empty();

    has_bg || has_border || has_shadow
}

// ---------------------------------------------------------------------------
// Widget-specific quad painting
// ---------------------------------------------------------------------------

/// Helper: create a simple filled rect instance.
fn filled_rect(
    x: f32, y: f32, w: f32, h: f32,
    color: [f32; 4],
    radius: [f32; 4],
    clip: [f32; 4],
    opacity: f32,
) -> UiInstance {
    UiInstance {
        rect: [x, y, w, h],
        bg_color: color,
        border_color: [0.0; 4],
        border_width: [0.0; 4],
        border_radius: radius,
        clip_rect: clip,
        texture_index: -1,
        opacity,
        flags: UiInstance::FLAG_HAS_BACKGROUND | UiInstance::FLAG_HAS_CLIP,
        _pad: 0,
    }
}

/// Helper: create a bordered rect instance.
fn bordered_rect(
    x: f32, y: f32, w: f32, h: f32,
    bg: [f32; 4],
    border_col: [f32; 4],
    bw: f32,
    radius: [f32; 4],
    clip: [f32; 4],
    opacity: f32,
) -> UiInstance {
    let mut flags = UiInstance::FLAG_HAS_BORDER | UiInstance::FLAG_HAS_CLIP;
    if bg[3] > 0.0 { flags |= UiInstance::FLAG_HAS_BACKGROUND; }
    UiInstance {
        rect: [x, y, w, h],
        bg_color: bg,
        border_color: border_col,
        border_width: [bw, bw, bw, bw],
        border_radius: radius,
        clip_rect: clip,
        texture_index: -1,
        opacity,
        flags,
        _pad: 0,
    }
}

/// Emit extra quads for interactive widgets.
///
/// The base quad (from `node_to_instance`) has already been emitted by the
/// normal paint path if the node has a CSS background/border. This function
/// adds *widget chrome* — the intrinsic visual parts of buttons, text fields,
/// checkboxes, sliders, etc.
fn paint_input_widget(node: &CxrdNode, input: &InputKind, out: &mut Vec<UiInstance>) {
    let r = &node.layout.rect;
    let clip = node.layout.clip
        .map(|c| c.to_array())
        .unwrap_or([0.0, 0.0, 99999.0, 99999.0]);
    let opacity = node.style.opacity;

    match input {
        // ---- Button ----
        InputKind::Button { variant, disabled, .. } => {
            let bg = if *disabled {
                [0.30, 0.30, 0.35, 1.0] // muted grey
            } else {
                match variant {
                    ButtonVariant::Primary   => [0.145, 0.388, 0.922, 1.0], // #2563eb
                    ButtonVariant::Secondary => [0.216, 0.255, 0.318, 1.0], // #374151
                    ButtonVariant::Danger    => [0.863, 0.149, 0.149, 1.0], // #dc2626
                    ButtonVariant::Ghost     => [0.0, 0.0, 0.0, 0.0],
                    ButtonVariant::Link      => [0.0, 0.0, 0.0, 0.0],
                }
            };
            let radius = [6.0; 4];
            out.push(filled_rect(r.x, r.y, r.width, r.height, bg, radius, clip, opacity));
        }

        // ---- TextInput ----
        InputKind::TextInput { .. } => {
            let bg = [0.12, 0.13, 0.15, 1.0];        // dark input bg
            let border = [0.30, 0.33, 0.37, 1.0];     // subtle border
            let radius = [4.0; 4];
            out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
        }

        // ---- TextArea ----
        InputKind::TextArea { .. } => {
            let bg = [0.12, 0.13, 0.15, 1.0];
            let border = [0.30, 0.33, 0.37, 1.0];
            let radius = [4.0; 4];
            out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
        }

        // ---- Checkbox ----
        InputKind::Checkbox { checked, style, disabled, .. } => {
            match style {
                CheckboxStyle::Checkbox => {
                    // 18×18 box at the left edge of the node rect, vertically centred.
                    let box_size = 18.0_f32.min(r.height);
                    let bx = r.x;
                    let by = r.y + (r.height - box_size) * 0.5;
                    let border_col = if *disabled {
                        [0.30, 0.30, 0.35, 1.0]
                    } else {
                        [0.40, 0.44, 0.50, 1.0]
                    };
                    out.push(bordered_rect(bx, by, box_size, box_size, [0.10, 0.11, 0.12, 1.0], border_col, 1.5, [3.0; 4], clip, opacity));

                    if *checked {
                        // Inner filled rect as check indicator.
                        let inset = 4.0;
                        let fill = if *disabled {
                            [0.30, 0.30, 0.35, 1.0]
                        } else {
                            [0.145, 0.388, 0.922, 1.0] // primary blue
                        };
                        out.push(filled_rect(bx + inset, by + inset, box_size - inset * 2.0, box_size - inset * 2.0, fill, [2.0; 4], clip, opacity));
                    }
                }
                CheckboxStyle::Toggle => {
                    // Toggle switch: 40×22 pill track + 18×18 thumb.
                    let track_w = 40.0_f32.min(r.width);
                    let track_h = 22.0_f32.min(r.height);
                    let tx = r.x;
                    let ty = r.y + (r.height - track_h) * 0.5;
                    let track_bg = if *checked {
                        [0.145, 0.388, 0.922, 1.0] // active blue
                    } else {
                        [0.25, 0.27, 0.30, 1.0] // inactive grey
                    };
                    out.push(filled_rect(tx, ty, track_w, track_h, track_bg, [track_h / 2.0; 4], clip, opacity));

                    // Thumb circle.
                    let thumb_size = track_h - 4.0;
                    let thumb_x = if *checked { tx + track_w - thumb_size - 2.0 } else { tx + 2.0 };
                    let thumb_y = ty + 2.0;
                    out.push(filled_rect(thumb_x, thumb_y, thumb_size, thumb_size, [1.0, 1.0, 1.0, 1.0], [thumb_size / 2.0; 4], clip, opacity));
                }
            }
        }

        // ---- Slider ----
        InputKind::Slider { value, min, max, disabled, .. } => {
            let track_h = 4.0;
            let thumb_size = 16.0;

            // Track
            let track_y = r.y + (r.height - track_h) * 0.5;
            let track_bg = [0.25, 0.27, 0.30, 1.0];
            out.push(filled_rect(r.x, track_y, r.width, track_h, track_bg, [track_h / 2.0; 4], clip, opacity));

            // Filled portion
            let range = if (*max - *min).abs() > f64::EPSILON { *max - *min } else { 1.0 };
            let pct = ((*value - *min) / range).clamp(0.0, 1.0) as f32;
            let fill_w = pct * r.width;
            let fill_color = if *disabled {
                [0.30, 0.33, 0.37, 1.0]
            } else {
                [0.145, 0.388, 0.922, 1.0]
            };
            out.push(filled_rect(r.x, track_y, fill_w, track_h, fill_color, [track_h / 2.0; 4], clip, opacity));

            // Thumb
            let thumb_x = r.x + fill_w - thumb_size * 0.5;
            let thumb_y = r.y + (r.height - thumb_size) * 0.5;
            let thumb_col = if *disabled {
                [0.50, 0.50, 0.55, 1.0]
            } else {
                [0.145, 0.388, 0.922, 1.0]
            };
            out.push(filled_rect(thumb_x, thumb_y, thumb_size, thumb_size, thumb_col, [thumb_size / 2.0; 4], clip, opacity));
        }

        // ---- Dropdown ----
        InputKind::Dropdown { disabled, .. } => {
            let bg = [0.12, 0.13, 0.15, 1.0];
            let border = if *disabled {
                [0.25, 0.27, 0.30, 1.0]
            } else {
                [0.30, 0.33, 0.37, 1.0]
            };
            let radius = [4.0; 4];
            out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));

            // Chevron indicator: small right-aligned square.
            let chev_size = 10.0;
            let chev_x = r.x + r.width - chev_size - 8.0;
            let chev_y = r.y + (r.height - chev_size) * 0.5;
            out.push(filled_rect(chev_x, chev_y, chev_size, chev_size, [0.50, 0.54, 0.60, 1.0], [2.0; 4], clip, opacity));
        }

        // ---- ColorPicker ----
        InputKind::ColorPicker { value, .. } => {
            let swatch_size = r.height.min(r.width).min(32.0);
            out.push(bordered_rect(r.x, r.y, swatch_size, swatch_size, value.to_array(), [0.40, 0.44, 0.50, 1.0], 1.0, [4.0; 4], clip, opacity));
        }

        // ---- TabBar ----
        InputKind::TabBar { tabs, active_tab } => {
            if tabs.is_empty() { return; }
            let tab_w = r.width / tabs.len() as f32;
            for (i, tab) in tabs.iter().enumerate() {
                let tx = r.x + i as f32 * tab_w;
                let active = tab.id == *active_tab;
                let bg = if active {
                    [0.145, 0.388, 0.922, 0.2]
                } else {
                    [0.0, 0.0, 0.0, 0.0]
                };
                out.push(filled_rect(tx, r.y, tab_w, r.height, bg, [0.0; 4], clip, opacity));

                // Active indicator bar at bottom.
                if active {
                    let bar_h = 2.0;
                    out.push(filled_rect(tx, r.y + r.height - bar_h, tab_w, bar_h, [0.145, 0.388, 0.922, 1.0], [1.0; 4], clip, opacity));
                }
            }
        }

        // ---- ScrollView ----
        InputKind::ScrollView { .. } => {
            // ScrollView has no intrinsic chrome — children are clipped by the
            // layout engine via overflow:hidden. We could add scrollbar tracks
            // here in the future.
        }

        // ---- Link ----
        InputKind::Link { .. } => {
            // Links are rendered as text (handled by text.rs). An underline
            // could be added here as a thin rect, but for now the text color
            // is sufficient.
        }

        // ---- AssetSelector ----
        InputKind::AssetSelector { .. } => {
            let bg = [0.12, 0.13, 0.15, 1.0];
            let border = [0.30, 0.33, 0.37, 1.0];
            let radius = [4.0; 4];
            out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
        }
    }
}

/// Convert a CXRD node into a GPU UiInstance.
fn node_to_instance(node: &CxrdNode) -> UiInstance {
    let s = &node.style;
    let r = &node.layout.rect;

    let bg_color = match &s.background {
        Background::Solid(c) => c.to_array(),
        Background::LinearGradient { stops, .. } => {
            // For now, use the first stop color. Full gradient support
            // will require a separate gradient shader pass.
            stops.first().map(|s| s.color.to_array()).unwrap_or([0.0; 4])
        }
        Background::RadialGradient { stops } => {
            stops.first().map(|s| s.color.to_array()).unwrap_or([0.0; 4])
        }
        _ => [0.0, 0.0, 0.0, 0.0],
    };

    let has_bg = !matches!(s.background, Background::None);
    let has_border = s.border_width.top > 0.0 || s.border_width.right > 0.0
        || s.border_width.bottom > 0.0 || s.border_width.left > 0.0;
    let has_texture = matches!(s.background, Background::Image { .. });
    let has_clip = node.layout.clip.is_some();

    let mut flags = 0u32;
    if has_bg { flags |= UiInstance::FLAG_HAS_BACKGROUND; }
    if has_border { flags |= UiInstance::FLAG_HAS_BORDER; }
    if has_texture { flags |= UiInstance::FLAG_HAS_TEXTURE; }
    if has_clip { flags |= UiInstance::FLAG_HAS_CLIP; }

    let texture_index = match &s.background {
        Background::Image { asset_index } => *asset_index as i32,
        _ => -1,
    };

    let clip_rect = node.layout.clip
        .map(|c| c.to_array())
        .unwrap_or([0.0, 0.0, 99999.0, 99999.0]);

    UiInstance {
        rect: r.to_array(),
        bg_color,
        border_color: s.border_color.to_array(),
        border_width: [s.border_width.top, s.border_width.right, s.border_width.bottom, s.border_width.left],
        border_radius: s.border_radius.to_array(),
        clip_rect,
        texture_index,
        opacity: s.opacity,
        flags,
        _pad: 0,
    }
}
