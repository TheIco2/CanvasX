// openrender-runtime/src/scene/paint.rs
//
// Paint pass — converts a laid-out CXRD tree into a flat list of UiInstance
// draw calls for the GPU renderer. Depth-first traversal respects z-index
// and stacking context.

use crate::compiler::css::apply_property;
use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeId, NodeKind, CxrdNode};
use crate::cxrd::input::{InputKind, ButtonVariant, CheckboxStyle};
use crate::cxrd::style::{BorderStyle, ComputedStyle, Display, Background, GradientStop};
use crate::gpu::vertex::UiInstance;
use std::collections::HashMap;

/// Key for gradient texture caching: (gradient type, parameters, size)
#[derive(Hash, PartialEq, Eq, Clone)]
pub enum GradientCacheKey {
    Linear {
        angle_deg: i32,  // Round to nearest degree for cache key
        stops_hash: u64, // Hash of gradient stops
        width: u32,
        height: u32,
    },
    Radial {
        stops_hash: u64,
        width: u32,
        height: u32,
    },
}

/// A rasterized gradient texture to upload.
pub struct GradientTexture {
    /// Unique slot (high range, 20000+)
    pub slot: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Output from the paint pass.
pub struct PaintOutput {
    pub instances: Vec<UiInstance>,
    pub gradient_textures: Vec<GradientTexture>,
}

static NEXT_GRADIENT_SLOT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(20000);

/// Compute a hash of gradient stops for cache key generation.
/// Hashes color RGBA and position values. Colors are converted to bit representation
/// for hashing since f32 doesn't implement Hash.
fn hash_gradient_stops(stops: &[GradientStop]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    for stop in stops {
        let color_array = stop.color.to_array();
        // Hash each f32 component as bits since f32 doesn't implement Hash
        for &val in &color_array {
            val.to_bits().hash(&mut hasher);
        }
        stop.position.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Paint the entire document into a list of GPU instances.
/// Uses gradient cache to avoid re-rasterizing identical gradients.
pub fn paint_document(doc: &CxrdDocument, gradient_cache: &mut HashMap<GradientCacheKey, GradientTexture>) -> PaintOutput {
    // Reset gradient slot counter each frame to reuse slots
    NEXT_GRADIENT_SLOT.store(20000, std::sync::atomic::Ordering::Relaxed);

    let mut instances = Vec::with_capacity(doc.nodes.len() * 2);
    let mut gradient_textures = Vec::new();
    paint_node(doc, doc.root, &mut instances, &mut gradient_textures, gradient_cache);
    PaintOutput { instances, gradient_textures }
}

/// Recursively paint a node and its children.
fn paint_node(doc: &CxrdDocument, node_id: NodeId, out: &mut Vec<UiInstance>, grad_textures: &mut Vec<GradientTexture>, gradient_cache: &mut HashMap<GradientCacheKey, GradientTexture>) {
    let node = match doc.get_node(node_id) {
        Some(n) => n,
        None => return,
    };

    if matches!(node.style.display, Display::None) {
        return;
    }

    // Emit box-shadow quads BEHIND the node.
    if !node.style.box_shadow.is_empty() {
        let r = &node.layout.rect;
        let clip = node.layout.clip
            .map(|c| c.to_array())
            .unwrap_or([0.0, 0.0, 99999.0, 99999.0]);
        for shadow in &node.style.box_shadow {
            if shadow.inset { continue; } // skip inset shadows for now
            let c = shadow.color.to_array();
            let spread = shadow.spread_radius.max(0.0);
            let blur = shadow.blur_radius.max(0.0);
            let max_expand = spread + blur;

            // Base shadow body.
            let base_sx = r.x + shadow.offset_x - spread;
            let base_sy = r.y + shadow.offset_y - spread;
            let base_sw = r.width + spread * 2.0;
            let base_sh = r.height + spread * 2.0;
            let base_alpha = c[3] * 0.16;
            out.push(filled_rect(
                base_sx,
                base_sy,
                base_sw,
                base_sh,
                [c[0], c[1], c[2], base_alpha],
                [
                    (node.style.border_radius.top_left + spread).max(0.0),
                    (node.style.border_radius.top_right + spread).max(0.0),
                    (node.style.border_radius.bottom_right + spread).max(0.0),
                    (node.style.border_radius.bottom_left + spread).max(0.0),
                ],
                clip,
                node.style.opacity,
            ));

            // Layered falloff shells approximate blur without a dedicated blur pass.
            let layers = blur.ceil().clamp(0.0, 3.0) as usize;
            for layer in 0..layers {
                let t = (layer + 1) as f32 / (layers as f32 + 1.0);
                let expand = spread + blur * t;
                let alpha = c[3] * 0.12 * (1.0 - t).powf(1.6);
                if alpha <= 0.001 { continue; }

                out.push(filled_rect(
                    r.x + shadow.offset_x - expand,
                    r.y + shadow.offset_y - expand,
                    r.width + expand * 2.0,
                    r.height + expand * 2.0,
                    [c[0], c[1], c[2], alpha],
                    [
                        (node.style.border_radius.top_left + expand).max(0.0),
                        (node.style.border_radius.top_right + expand).max(0.0),
                        (node.style.border_radius.bottom_right + expand).max(0.0),
                        (node.style.border_radius.bottom_left + expand).max(0.0),
                    ],
                    clip,
                    node.style.opacity,
                ));
            }

            // Outermost soft fringe.
            if max_expand > spread {
                let fringe_expand = max_expand;
                out.push(filled_rect(
                    r.x + shadow.offset_x - fringe_expand,
                    r.y + shadow.offset_y - fringe_expand,
                    r.width + fringe_expand * 2.0,
                    r.height + fringe_expand * 2.0,
                    [c[0], c[1], c[2], c[3] * 0.025],
                    [
                        (node.style.border_radius.top_left + fringe_expand).max(0.0),
                        (node.style.border_radius.top_right + fringe_expand).max(0.0),
                        (node.style.border_radius.bottom_right + fringe_expand).max(0.0),
                        (node.style.border_radius.bottom_left + fringe_expand).max(0.0),
                    ],
                    clip,
                    node.style.opacity,
                ));
            }
        }
    }

    // Only emit an instance if the node has visible visual properties.
    if should_paint(node) {
        // Check for gradient backgrounds — use cached rasterization if available.
        // Larger gradient sizes (up to 2048x2048) provide better quality.
        match &node.style.background {
            Background::LinearGradient { angle_deg, stops } if !stops.is_empty() => {
                let r = &node.layout.rect;
                if r.width > 0.0 && r.height > 0.0 {
                    let slot = NEXT_GRADIENT_SLOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // Increased gradient sizes: up to 2048px instead of 512px
                    // This reduces visible banding in large gradient elements
                    let gw = (r.width.ceil() as u32).min(2048);
                    let gh = (r.height.ceil() as u32).min(2048);
                    
                    // Check gradient cache to avoid re-rasterization
                    let stops_hash = hash_gradient_stops(stops);
                    let cache_key = GradientCacheKey::Linear {
                        angle_deg: *angle_deg as i32,
                        stops_hash,
                        width: gw,
                        height: gh,
                    };
                    
                    let gradient_tex = if let Some(cached_tex) = gradient_cache.get(&cache_key) {
                        // Reuse cached texture data
                        GradientTexture {
                            slot,
                            width: cached_tex.width,
                            height: cached_tex.height,
                            rgba: cached_tex.rgba.clone(),
                        }
                    } else {
                        // Rasterize new gradient and cache it
                        let rgba = rasterize_linear_gradient(*angle_deg, stops, gw, gh);
                        let tex = GradientTexture {
                            slot,
                            width: gw,
                            height: gh,
                            rgba: rgba.clone(),
                        };
                        gradient_cache.insert(cache_key, GradientTexture {
                            slot: 0, // Don't care about slot in cache
                            width: gw,
                            height: gh,
                            rgba,
                        });
                        tex
                    };
                    
                    grad_textures.push(gradient_tex);
                    // Emit a textured quad
                    let clip = node.layout.clip
                        .map(|c| c.to_array())
                        .unwrap_or([0.0, 0.0, 99999.0, 99999.0]);
                    let mut flags = UiInstance::FLAG_HAS_BACKGROUND | UiInstance::FLAG_HAS_TEXTURE;
                    if node.layout.clip.is_some() { flags |= UiInstance::FLAG_HAS_CLIP; }
                    let has_border = node.style.border_width.top > 0.0
                        || node.style.border_width.right > 0.0
                        || node.style.border_width.bottom > 0.0
                        || node.style.border_width.left > 0.0;
                    if has_border { flags |= UiInstance::FLAG_HAS_BORDER; }
                    out.push(UiInstance {
                        rect: r.to_array(),
                        bg_color: [0.0, 0.0, 0.0, 0.0],
                        border_color: node.style.border_color.to_array(),
                        border_width: [node.style.border_width.top, node.style.border_width.right,
                                       node.style.border_width.bottom, node.style.border_width.left],
                        border_radius: node.style.border_radius.to_array(),
                        clip_rect: clip,
                        texture_index: slot as i32,
                        opacity: node.style.opacity,
                        flags,
                        _pad: 0,
                    });
                }
            }
            Background::RadialGradient { stops } if !stops.is_empty() => {
                let r = &node.layout.rect;
                if r.width > 0.0 && r.height > 0.0 {
                    let slot = NEXT_GRADIENT_SLOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let gw = (r.width.ceil() as u32).min(2048);
                    let gh = (r.height.ceil() as u32).min(2048);
                    
                    // Check gradient cache to avoid re-rasterization
                    let stops_hash = hash_gradient_stops(stops);
                    let cache_key = GradientCacheKey::Radial {
                        stops_hash,
                        width: gw,
                        height: gh,
                    };
                    
                    let gradient_tex = if let Some(cached_tex) = gradient_cache.get(&cache_key) {
                        // Reuse cached texture data
                        GradientTexture {
                            slot,
                            width: cached_tex.width,
                            height: cached_tex.height,
                            rgba: cached_tex.rgba.clone(),
                        }
                    } else {
                        // Rasterize new gradient and cache it
                        let rgba = rasterize_radial_gradient(stops, gw, gh);
                        let tex = GradientTexture {
                            slot,
                            width: gw,
                            height: gh,
                            rgba: rgba.clone(),
                        };
                        gradient_cache.insert(cache_key, GradientTexture {
                            slot: 0, // Don't care about slot in cache
                            width: gw,
                            height: gh,
                            rgba,
                        });
                        tex
                    };
                    
                    grad_textures.push(gradient_tex);
                    let clip = node.layout.clip
                        .map(|c| c.to_array())
                        .unwrap_or([0.0, 0.0, 99999.0, 99999.0]);
                    let mut flags = UiInstance::FLAG_HAS_BACKGROUND | UiInstance::FLAG_HAS_TEXTURE;
                    if node.layout.clip.is_some() { flags |= UiInstance::FLAG_HAS_CLIP; }
                    out.push(UiInstance {
                        rect: r.to_array(),
                        bg_color: [0.0, 0.0, 0.0, 0.0],
                        border_color: node.style.border_color.to_array(),
                        border_width: [node.style.border_width.top, node.style.border_width.right,
                                       node.style.border_width.bottom, node.style.border_width.left],
                        border_radius: node.style.border_radius.to_array(),
                        clip_rect: clip,
                        texture_index: slot as i32,
                        opacity: node.style.opacity,
                        flags,
                        _pad: 0,
                    });
                }
            }
            _ => {
                out.push(node_to_instance(node));
            }
        }
    }

    // For Input nodes, emit extra widget-specific quads.
    if let NodeKind::Input(ref input) = node.kind {
        paint_input_widget(node, input, out);
    }

    // For Canvas nodes, emit a textured quad (texture upload handled by main loop).
    if let NodeKind::Canvas { .. } = &node.kind {
        let r = &node.layout.rect;
        if r.width > 0.0 && r.height > 0.0 {
            let placeholder = -((node_id as i32) + 2);
            let mut flags = UiInstance::FLAG_HAS_BACKGROUND | UiInstance::FLAG_HAS_TEXTURE;
            if node.layout.clip.is_some() {
                flags |= UiInstance::FLAG_HAS_CLIP;
            }
            let has_border = node.style.border_width.top > 0.0
                || node.style.border_width.right > 0.0
                || node.style.border_width.bottom > 0.0
                || node.style.border_width.left > 0.0;
            if has_border {
                flags |= UiInstance::FLAG_HAS_BORDER;
            }
            out.push(UiInstance {
                rect: [r.x, r.y, r.width, r.height],
                bg_color: [0.0, 0.0, 0.0, 0.0],
                border_color: node.style.border_color.to_array(),
                border_width: [
                    node.style.border_width.top,
                    node.style.border_width.right,
                    node.style.border_width.bottom,
                    node.style.border_width.left,
                ],
                border_radius: node.style.border_radius.to_array(),
                clip_rect: node
                    .layout
                    .clip
                    .map(|c| c.to_array())
                    .unwrap_or([0.0, 0.0, 99999.0, 99999.0]),
                texture_index: placeholder,
                opacity: node.style.opacity,
                flags,
                _pad: 0,
            });
        }
    }

    // Paint children — use pre-existing order if no z-index is set (avoid sort).
    let children = &node.children;
    let needs_sort = children.iter().any(|&cid| {
        doc.get_node(cid).map(|c| c.style.z_index != 0).unwrap_or(false)
    });

    if needs_sort {
        let mut child_ids = children.clone();
        child_ids.sort_by_key(|&cid| {
            doc.get_node(cid).map(|c| c.style.z_index).unwrap_or(0)
        });
        for cid in child_ids {
            paint_node(doc, cid, out, grad_textures, gradient_cache);
        }
    } else {
        for &cid in children {
            paint_node(doc, cid, out, grad_textures, gradient_cache);
        }
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

    // Pseudo-class overrides might add background or border even when none in base style.
    let pseudo_adds_visual = {
        let check = |styles: &[(String, String)]| styles.iter().any(|(p, _)| {
            matches!(p.as_str(), "background" | "background-color" | "border" | "border-color"
                | "border-width" | "border-top-width" | "border-right-width"
                | "border-bottom-width" | "border-left-width")
        });
        (node.hovered && check(&node.hover_style))
            || (node.active && check(&node.active_style))
            || (node.focused && check(&node.focus_style))
    };

    has_bg || has_border || has_shadow || pseudo_adds_visual
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
            if matches!(node.style.background, Background::None) {
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
        }

        // ---- TextInput ----
        InputKind::TextInput { .. } => {
            if matches!(node.style.background, Background::None) {
                let bg = [0.12, 0.13, 0.15, 1.0];        // dark input bg
                let border = [0.30, 0.33, 0.37, 1.0];     // subtle border
                let radius = [4.0; 4];
                out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
            }
        }

        // ---- TextArea ----
        InputKind::TextArea { .. } => {
            if matches!(node.style.background, Background::None) {
                let bg = [0.12, 0.13, 0.15, 1.0];
                let border = [0.30, 0.33, 0.37, 1.0];
                let radius = [4.0; 4];
                out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
            }
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
            if matches!(node.style.background, Background::None) {
                let bg = [0.12, 0.13, 0.15, 1.0];
                let border = if *disabled {
                    [0.25, 0.27, 0.30, 1.0]
                } else {
                    [0.30, 0.33, 0.37, 1.0]
                };
                let radius = [4.0; 4];
                out.push(bordered_rect(r.x, r.y, r.width, r.height, bg, border, 1.0, radius, clip, opacity));
            }
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

/// Return the effective style for a node, applying pseudo-class overrides.
/// Priority: base → :hover → :focus → :active (highest wins).
fn effective_style(node: &CxrdNode) -> ComputedStyle {
    let needs_override = (node.hovered && !node.hover_style.is_empty())
        || (node.focused && !node.focus_style.is_empty())
        || (node.active && !node.active_style.is_empty());

    if needs_override {
        let mut style = node.style.clone();
        let empty_vars = HashMap::new();
        // Apply in order of specificity: hover, then focus, then active.
        if node.hovered && !node.hover_style.is_empty() {
            for (prop, val) in &node.hover_style {
                apply_property(&mut style, prop, val, &empty_vars);
            }
        }
        if node.focused && !node.focus_style.is_empty() {
            for (prop, val) in &node.focus_style {
                apply_property(&mut style, prop, val, &empty_vars);
            }
        }
        if node.active && !node.active_style.is_empty() {
            for (prop, val) in &node.active_style {
                apply_property(&mut style, prop, val, &empty_vars);
            }
        }
        style
    } else {
        node.style.clone()
    }
}

/// Convert a CXRD node into a GPU UiInstance.
fn node_to_instance(node: &CxrdNode) -> UiInstance {
    let s = &effective_style(node);
    let r = &node.layout.rect;

    let bg_color = match &s.background {
        Background::Solid(c) => apply_backdrop_fallback(c.to_array(), s.backdrop_blur),
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
    let has_border = !matches!(s.border_style, BorderStyle::None | BorderStyle::Hidden)
        && (s.border_width.top > 0.0 || s.border_width.right > 0.0
        || s.border_width.bottom > 0.0 || s.border_width.left > 0.0);
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

/// Approximate CSS `backdrop-filter: blur(...)` for translucent panels.
///
/// Native OpenRender doesn't blur the framebuffer yet, so we emulate the
/// readability effect by slightly increasing alpha and lifting luminance.
fn apply_backdrop_fallback(mut color: [f32; 4], backdrop_blur: f32) -> [f32; 4] {
    if backdrop_blur <= 0.0 || color[3] <= 0.0 || color[3] >= 1.0 {
        return color;
    }

    let strength = (backdrop_blur / 24.0).clamp(0.0, 1.0);
    // Keep fallback subtle: slightly improve readability without noticeably
    // brightening translucent HUD panels.
    if color[3] < 0.10 {
        let min_alpha = 0.08 + 0.05 * strength;
        color[3] = color[3].max(min_alpha);
    } else {
        color[3] = (color[3] + 0.06 * strength).min(1.0);
    }

    let lift = 0.04 * strength;
    color[0] = (color[0] + (1.0 - color[0]) * lift).clamp(0.0, 1.0);
    color[1] = (color[1] + (1.0 - color[1]) * lift).clamp(0.0, 1.0);
    color[2] = (color[2] + (1.0 - color[2]) * lift).clamp(0.0, 1.0);

    color
}
// ---------------------------------------------------------------------------
// Gradient rasterization
// ---------------------------------------------------------------------------

/// Interpolate between gradient stops at position t (0..1).
/// Interpolation is done in sRGB space to match browser CSS gradient default.
fn sample_gradient(stops: &[GradientStop], t: f32) -> [u8; 4] {
    if stops.is_empty() { return [0, 0, 0, 0]; }
    if stops.len() == 1 {
        let c = stops[0].color.to_array();
        return [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8];
    }

    let t = t.clamp(0.0, 1.0);

    // Find the two stops to interpolate between
    if t <= stops[0].position {
        let c = stops[0].color.to_array();
        return [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8];
    }
    if t >= stops[stops.len() - 1].position {
        let c = stops[stops.len() - 1].color.to_array();
        return [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8];
    }

    for i in 0..stops.len() - 1 {
        if t >= stops[i].position && t <= stops[i + 1].position {
            let range = stops[i + 1].position - stops[i].position;
            let f = if range > 0.0 { (t - stops[i].position) / range } else { 0.0 };
            let a = stops[i].color.to_array();
            let b = stops[i + 1].color.to_array();
            // Interpolate in sRGB space (CSS default behaviour).
            let r = a[0] + (b[0] - a[0]) * f;
            let g = a[1] + (b[1] - a[1]) * f;
            let bl = a[2] + (b[2] - a[2]) * f;
            let alpha = a[3] + (b[3] - a[3]) * f;
            return [
                (r.clamp(0.0, 1.0) * 255.0) as u8,
                (g.clamp(0.0, 1.0) * 255.0) as u8,
                (bl.clamp(0.0, 1.0) * 255.0) as u8,
                (alpha.clamp(0.0, 1.0) * 255.0) as u8,
            ];
        }
    }

    [0, 0, 0, 0]
}

/// Rasterize a linear gradient into RGBA pixels.
/// Gradients up to 2048×2048 are supported for high quality.
fn rasterize_linear_gradient(angle_deg: f32, stops: &[GradientStop], w: u32, h: u32) -> Vec<u8> {
    // Allow up to 2048 (was 512) for better quality
    let w = w.min(2048).max(1);
    let h = h.min(2048).max(1);
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    let angle_rad = angle_deg.to_radians();
    // CSS gradient angle: 0deg = to top, 90deg = to right, 180deg = to bottom
    let dx = angle_rad.sin();
    let dy = -angle_rad.cos();

    for y in 0..h {
        for x in 0..w {
            // Normalize to -0.5..0.5 range
            let nx = x as f32 / w as f32 - 0.5;
            let ny = y as f32 / h as f32 - 0.5;
            // Project onto gradient direction
            let t = (nx * dx + ny * dy) + 0.5;
            let pixel = sample_gradient(stops, t);
            let idx = ((y * w + x) * 4) as usize;
            rgba[idx] = pixel[0];
            rgba[idx + 1] = pixel[1];
            rgba[idx + 2] = pixel[2];
            rgba[idx + 3] = pixel[3];
        }
    }

    rgba
}

/// Rasterize a radial gradient into RGBA pixels.
fn rasterize_radial_gradient(stops: &[GradientStop], w: u32, h: u32) -> Vec<u8> {
    let w = w.min(2048).max(1);
    let h = h.min(2048).max(1);
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let max_r = (cx * cx + cy * cy).sqrt();

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let t = (dx * dx + dy * dy).sqrt() / max_r;
            let pixel = sample_gradient(stops, t);
            let idx = ((y * w + x) * 4) as usize;
            rgba[idx] = pixel[0];
            rgba[idx + 1] = pixel[1];
            rgba[idx + 2] = pixel[2];
            rgba[idx + 3] = pixel[3];
        }
    }

    rgba
}