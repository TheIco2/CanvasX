// prism-runtime/src/scene/text.rs
//
// Text painting — extracts all text nodes from the PRD tree and builds
// glyphon TextArea entries for the text renderer.

use crate::prd::document::PrdDocument;
use crate::prd::node::{NodeId, NodeKind};
use crate::prd::input::InputKind;
use crate::prd::style::{Display, FontStyle, TextAlign, TextTransform, WhiteSpace};
use glyphon::{Attrs, Buffer, Color as GlyphonColor, Family, Metrics, Shaping, TextArea, TextBounds, Weight};
use glyphon::cosmic_text::Align;
use std::collections::HashMap;

/// Holds prepared text buffers for all text nodes in a document.
pub struct TextPainter {
    /// Prepared text buffers keyed by node ID.
    pub buffers: HashMap<u32, Buffer>,
    /// Cache of (content_hash, font_size, font_family, font_weight, text_align, container_width)
    /// per node, to detect when a buffer needs recreation.
    buffer_keys: HashMap<u32, u64>,
}

impl TextPainter {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            buffer_keys: HashMap::new(),
        }
    }

    /// Prepare text buffers for all text nodes in the document.
    /// Call this after layout and before rendering.
    /// Only recreates buffers when text content or style has changed.
    ///
    /// `scale_factor` is the device pixel ratio. Text is shaped at
    /// `font_size * scale_factor` so glyphs rasterize at physical pixel
    /// resolution and stay crisp on high-DPI displays.
    pub fn prepare(
        &mut self,
        doc: &PrdDocument,
        font_system: &mut glyphon::FontSystem,
        data_values: &HashMap<String, String>,
        scale_factor: f32,
    ) {
        // Collect live node IDs so we can prune stale buffers
        let mut live_ids = Vec::new();
        self.prepare_node(doc, doc.root, font_system, data_values, &mut live_ids, scale_factor);
        // Remove buffers for nodes that no longer exist
        self.buffers.retain(|k, _| live_ids.contains(k));
        self.buffer_keys.retain(|k, _| live_ids.contains(k));
    }

    fn prepare_node(
        &mut self,
        doc: &PrdDocument,
        node_id: NodeId,
        font_system: &mut glyphon::FontSystem,
        data_values: &HashMap<String, String>,
        live_ids: &mut Vec<u32>,
        scale_factor: f32,
    ) {
        let node = match doc.get_node(node_id) {
            Some(n) => n,
            None => return,
        };

        if matches!(node.style.display, Display::None) {
            return;
        }

        let text_content = match &node.kind {
            NodeKind::Text { content } => Some(content.clone()),
            _ => None,
        };

        if let Some(content) = text_content {
            // Apply text-transform.
            let content = match node.style.text_transform {
                TextTransform::Uppercase => content.to_uppercase(),
                TextTransform::Lowercase => content.to_lowercase(),
                TextTransform::Capitalize => {
                    content.split_whitespace()
                        .map(|word| {
                            let mut chars = word.chars();
                            match chars.next() {
                                Some(c) => {
                                    let upper: String = c.to_uppercase().collect();
                                    format!("{}{}", upper, chars.as_str())
                                }
                                None => String::new(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }
                TextTransform::None => content,
            };

            let style = &node.style;
            let rect = &node.layout.content_rect;

            if rect.width > 0.0 && !content.is_empty() {
                live_ids.push(node.id);

                // Compute a cache key from content + style + layout dimensions.
                // Use a simple hash to avoid recomputing expensive text shaping.
                let cache_key = {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    content.hash(&mut hasher);
                    style.font_size.to_bits().hash(&mut hasher);
                    style.font_family.hash(&mut hasher);
                    style.font_weight.0.hash(&mut hasher);
                    (rect.width as u32).hash(&mut hasher);
                    (rect.height as u32).hash(&mut hasher);
                    std::mem::discriminant(&style.text_align).hash(&mut hasher);
                    style.line_height.to_bits().hash(&mut hasher);
                    style.letter_spacing.to_bits().hash(&mut hasher);
                    scale_factor.to_bits().hash(&mut hasher);
                    hasher.finish()
                };

                // Skip recreation if buffer already exists with same key.
                if let Some(&existing_key) = self.buffer_keys.get(&node.id) {
                    if existing_key == cache_key && self.buffers.contains_key(&node.id) {
                        // Buffer is still valid
                        for &child_id in &node.children {
                            self.prepare_node(doc, child_id, font_system, data_values, live_ids, scale_factor);
                        }
                        return;
                    }
                }

                let font_size = style.font_size;
                let line_height = style.line_height * font_size;
                // Shape at physical pixel metrics so glyphs are crisp.
                let metrics = Metrics::new(font_size * scale_factor, line_height * scale_factor);

                let mut buffer = Buffer::new(font_system, metrics);

                let family = if style.font_family.is_empty() {
                    Family::SansSerif
                } else {
                    Family::Name(&style.font_family)
                };

                let weight = Weight(style.font_weight.0);

                let mut attrs = Attrs::new()
                    .family(family)
                    .weight(weight);

                // Apply letter-spacing (stored in px, cosmic-text expects EM)
                if style.letter_spacing.abs() > 0.001 && font_size > 0.0 {
                    attrs = attrs.letter_spacing(style.letter_spacing / font_size);
                }

                let alignment = match style.text_align {
                    TextAlign::Right => Some(Align::Right),
                    TextAlign::Center => Some(Align::Center),
                    TextAlign::Left => None, // Left is the default
                };

                // Add 1px to buffer width so glyphs at the trailing edge are not
                // clipped due to font metrics extending past the em-square.
                // Width is in physical pixels because metrics are physical.
                let buf_width = match style.white_space {
                    WhiteSpace::NoWrap | WhiteSpace::Pre => f32::MAX,
                    _ => (rect.width + 1.0) * scale_factor,
                };
                buffer.set_size(font_system, Some(buf_width), None);
                buffer.set_text(font_system, &content, &attrs, Shaping::Advanced, alignment);
                buffer.shape_until_scroll(font_system, false);

                self.buffers.insert(node.id, buffer);
                self.buffer_keys.insert(node.id, cache_key);
            }
        }

        // Render text labels for input widgets (button labels, dropdown values, etc.)
        // Input widgets use skip_children=true in the HTML compiler, so their labels
        // are NOT child text nodes – they live in the InputKind enum fields instead.
        if let NodeKind::Input(input_kind) = &node.kind {
            let rect = &node.layout.content_rect;
            if rect.width > 0.0 {
                let (label_text, center_align) = match input_kind {
                    InputKind::Button { label, .. } => (label.clone(), true),
                    InputKind::Link { label, .. } => (label.clone(), false),
                    InputKind::Dropdown { selected, options, placeholder, .. } => {
                        let resolved = if let Some(sel) = selected {
                            options.iter()
                                .find(|o| &o.0 == sel)
                                .map(|o| o.1.clone())
                                .unwrap_or_else(|| sel.clone())
                        } else {
                            placeholder.clone()
                        };
                        (resolved, false)
                    }
                    InputKind::TextInput { value, placeholder, .. } => {
                        let txt: String = if value.is_empty() {
                            placeholder.clone()
                        } else {
                            value.clone()
                        };
                        (txt, false)
                    }
                    InputKind::TextArea { value, placeholder, .. } => {
                        let txt: String = if value.is_empty() {
                            placeholder.clone()
                        } else {
                            value.clone()
                        };
                        (txt, false)
                    }
                    _ => (String::new(), false),
                };

                if !label_text.is_empty() {
                    live_ids.push(node.id);

                    let style = &node.style;
                    let cache_key = {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        label_text.hash(&mut hasher);
                        style.font_size.to_bits().hash(&mut hasher);
                        style.font_family.hash(&mut hasher);
                        style.font_weight.0.hash(&mut hasher);
                        (rect.width as u32).hash(&mut hasher);
                        center_align.hash(&mut hasher);
                        scale_factor.to_bits().hash(&mut hasher);
                        hasher.finish()
                    };

                    if let Some(&existing_key) = self.buffer_keys.get(&node.id) {
                        if existing_key == cache_key && self.buffers.contains_key(&node.id) {
                            for &child_id in &node.children {
                                self.prepare_node(doc, child_id, font_system, data_values, live_ids, scale_factor);
                            }
                            return;
                        }
                    }

                    let font_size = style.font_size;
                    let line_height = style.line_height * font_size;
                    let metrics = glyphon::Metrics::new(font_size * scale_factor, line_height * scale_factor);
                    let mut buffer = glyphon::Buffer::new(font_system, metrics);

                    let family = if style.font_family.is_empty() {
                        glyphon::Family::SansSerif
                    } else {
                        glyphon::Family::Name(&style.font_family)
                    };
                    let weight = glyphon::Weight(style.font_weight.0);
                    let glyph_style = match style.font_style {
                        FontStyle::Italic => glyphon::Style::Italic,
                        FontStyle::Oblique => glyphon::Style::Italic, // glyphon maps oblique to italic
                        FontStyle::Normal => glyphon::Style::Normal,
                    };
                    let attrs = glyphon::Attrs::new().family(family).weight(weight).style(glyph_style);
                    let alignment = if center_align {
                        Some(glyphon::cosmic_text::Align::Center)
                    } else {
                        None
                    };

                    buffer.set_size(font_system, Some(rect.width * scale_factor), None);
                    buffer.set_text(font_system, &label_text, &attrs, glyphon::Shaping::Advanced, alignment);
                    buffer.shape_until_scroll(font_system, false);

                    self.buffers.insert(node.id, buffer);
                    self.buffer_keys.insert(node.id, cache_key);
                }
            }
        }

        for &child_id in &node.children {
            self.prepare_node(doc, child_id, font_system, data_values, live_ids, scale_factor);
        }
    }

    /// Build TextArea references for the renderer.
    /// The returned Vec borrows from `self.buffers`, so `self` must outlive the render call.
    pub fn text_areas<'a>(&'a self, doc: &'a PrdDocument) -> Vec<TextArea<'a>> {
        let mut areas = Vec::new();

        for (node_id, buffer) in &self.buffers {
            let node = match doc.get_node(*node_id) {
                Some(n) => n,
                None => continue,
            };

            let rect = &node.layout.content_rect;
            // Determine text color with pseudo-class overrides.
            // Priority: :active > :focus > :hover > base.
            let color = {
                let mut c = node.style.color.clone();
                let mut resolved = false;

                // Check for :active override (highest priority).
                if node.active && !node.active_style.is_empty() {
                    for (prop, val) in &node.active_style {
                        if prop == "color" {
                            if let Some(parsed) = crate::compiler::css::parse_color(val) {
                                c = parsed;
                                resolved = true;
                            }
                        }
                    }
                }
                // Check for :focus override.
                if !resolved && node.focused && !node.focus_style.is_empty() {
                    for (prop, val) in &node.focus_style {
                        if prop == "color" {
                            if let Some(parsed) = crate::compiler::css::parse_color(val) {
                                c = parsed;
                                resolved = true;
                            }
                        }
                    }
                }
                // Check for :hover override (own or inherited from ancestor).
                if !resolved && node.hovered {
                    if !node.hover_style.is_empty() {
                        for (prop, val) in &node.hover_style {
                            if prop == "color" {
                                if let Some(parsed) = crate::compiler::css::parse_color(val) {
                                    c = parsed;
                                    resolved = true;
                                }
                            }
                        }
                    }
                    if !resolved {
                        // Check ancestors for inheritable color overrides.
                        let mut ancestor_id = doc.find_parent(*node_id);
                        while let Some(aid) = ancestor_id {
                            if let Some(ancestor) = doc.get_node(aid) {
                                // Check active first, then focus, then hover on ancestor.
                                if ancestor.active && !ancestor.active_style.is_empty() {
                                    for (prop, val) in &ancestor.active_style {
                                        if prop == "color" {
                                            if let Some(parsed) = crate::compiler::css::parse_color(val) {
                                                c = parsed;
                                                resolved = true;
                                            }
                                        }
                                    }
                                    if resolved { break; }
                                }
                                if ancestor.hovered && !ancestor.hover_style.is_empty() {
                                    for (prop, val) in &ancestor.hover_style {
                                        if prop == "color" {
                                            if let Some(parsed) = crate::compiler::css::parse_color(val) {
                                                c = parsed;
                                                resolved = true;
                                            }
                                        }
                                    }
                                    if resolved { break; }
                                }
                            }
                            ancestor_id = doc.find_parent(aid);
                        }
                    }
                }
                c
            };

            // Use floor/ceil to prevent fractional-pixel clipping at text edges.
            // Add small buffers: +1px right (matches buffer width +1 in layout),
            // +2px bottom for font descender overflow.
            let mut left = rect.x.floor() as i32;
            let baseline_offset = 1.0;
            let mut top = (rect.y + baseline_offset).floor() as i32;
            let mut right = (rect.x + rect.width + 1.0).ceil() as i32;
            let mut bottom = (rect.y + rect.height + 2.0).ceil() as i32;

            // Intersect with the overflow clip rect from the nearest overflow:hidden ancestor.
            if let Some(clip) = &node.layout.clip {
                left = left.max(clip.x as i32);
                top = top.max(clip.y as i32);
                right = right.min((clip.x + clip.width) as i32);
                bottom = bottom.min((clip.y + clip.height) as i32);
                if right <= left || bottom <= top {
                    continue; // Fully clipped
                }
            }

            areas.push(TextArea {
                buffer,
                left: rect.x,
                top: rect.y + baseline_offset, // Apply same baseline adjustment to text area position
                scale: 1.0,
                bounds: TextBounds {
                    left,
                    top,
                    right,
                    bottom,
                },
                default_color: GlyphonColor::rgba(
                    (color.r * 255.0) as u8,
                    (color.g * 255.0) as u8,
                    (color.b * 255.0) as u8,
                    (color.a * 255.0) as u8,
                ),
                custom_glyphs: &[],
            });
        }

        areas
    }
}

// ─── Data-bind format helpers ───

/// Format a raw data value using an optional format string.
///
/// Supported format tokens (matched as the *entire* format string):
///   `{bytes}`  — human-readable bytes (e.g. "16.0 GB")
///   `{speed}`  — bytes-per-second    (e.g. "1.2 MB/s")
///   `{uptime}` — seconds → "Xd Yh Zm"
///   `{.N}`     — round numeric value to N decimal places
///
/// If the format string contains `{}`, the raw value is substituted at that
/// position (e.g. `"{}%"` → `"47.5%"`).
///
/// If no format string is provided the raw value is returned as-is.
#[allow(dead_code)]
fn format_data_value(raw: &str, format: Option<&str>) -> String {
    let fmt = match format {
        Some(f) => f,
        None => return raw.to_string(),
    };

    // If there's no data yet, don't show the format template literally.
    if raw.is_empty() {
        return String::new();
    }

    match fmt {
        "{bytes}" => format_bytes(raw),
        "{speed}" => {
            let bytes = format_bytes(raw);
            format!("{}/s", bytes)
        }
        "{uptime}" => format_uptime(raw),
        _ if fmt.starts_with("{.") && fmt.ends_with('}') => {
            // Exact `{.N}` — precision only, no suffix
            if let Ok(prec) = fmt[2..fmt.len() - 1].parse::<usize>() {
                if let Ok(v) = raw.parse::<f64>() {
                    format!("{:.prec$}", v, prec = prec)
                } else {
                    raw.to_string()
                }
            } else {
                fmt.replace("{}", raw)
            }
        }
        _ if fmt.contains("{.") => {
            // `{.N}` with surrounding text, e.g. `{.1}%` or `{.0} W`
            if let Some(start) = fmt.find("{.") {
                if let Some(end) = fmt[start..].find('}') {
                    let spec = &fmt[start + 2..start + end]; // e.g. "1"
                    if let Ok(prec) = spec.parse::<usize>() {
                        if let Ok(v) = raw.parse::<f64>() {
                            let formatted = format!("{:.prec$}", v, prec = prec);
                            let mut result = String::new();
                            result.push_str(&fmt[..start]);
                            result.push_str(&formatted);
                            result.push_str(&fmt[start + end + 1..]);
                            return result;
                        }
                    }
                }
            }
            fmt.replace("{}", raw)
        }
        _ => fmt.replace("{}", raw),
    }
}

#[allow(dead_code)]
fn format_bytes(raw: &str) -> String {
    let v: f64 = match raw.parse() {
        Ok(v) => v,
        Err(_) => return raw.to_string(),
    };
    if v >= 1_099_511_627_776.0 {
        format!("{:.1} TB", v / 1_099_511_627_776.0)
    } else if v >= 1_073_741_824.0 {
        format!("{:.1} GB", v / 1_073_741_824.0)
    } else if v >= 1_048_576.0 {
        format!("{:.1} MB", v / 1_048_576.0)
    } else if v >= 1024.0 {
        format!("{:.1} KB", v / 1024.0)
    } else {
        format!("{:.0} B", v)
    }
}

#[allow(dead_code)]
fn format_uptime(raw: &str) -> String {
    let sec: u64 = match raw.parse::<f64>() {
        Ok(v) => v as u64,
        Err(_) => return raw.to_string(),
    };
    let d = sec / 86400;
    let h = (sec % 86400) / 3600;
    let m = (sec % 3600) / 60;
    let mut s = String::new();
    if d > 0 { s += &format!("{}d ", d); }
    if h > 0 || d > 0 { s += &format!("{}h ", h); }
    s += &format!("{}m", m);
    s
}

