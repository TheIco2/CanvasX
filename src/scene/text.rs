// canvasx-runtime/src/scene/text.rs
//
// Text painting — extracts all text nodes from the CXRD tree and builds
// glyphon TextArea entries for the text renderer.

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeId, NodeKind};
use crate::cxrd::style::{Display, TextAlign, TextTransform};
use glyphon::{Attrs, Buffer, Color as GlyphonColor, Family, Metrics, Shaping, TextArea, TextBounds, Weight};
use glyphon::cosmic_text::Align;
use std::collections::HashMap;

/// Holds prepared text buffers for all text nodes in a document.
pub struct TextPainter {
    /// Prepared text buffers keyed by node ID.
    pub buffers: HashMap<u32, Buffer>,
}

impl TextPainter {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Prepare text buffers for all text nodes in the document.
    /// Call this after layout and before rendering.
    pub fn prepare(
        &mut self,
        doc: &CxrdDocument,
        font_system: &mut glyphon::FontSystem,
        data_values: &HashMap<String, String>,
    ) {
        self.buffers.clear();
        self.prepare_node(doc, doc.root, font_system, data_values);
    }

    fn prepare_node(
        &mut self,
        doc: &CxrdDocument,
        node_id: NodeId,
        font_system: &mut glyphon::FontSystem,
        data_values: &HashMap<String, String>,
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
            NodeKind::DataBound { binding, format } => {
                let raw = data_values.get(binding).cloned().unwrap_or_default();
                Some(format_data_value(&raw, format.as_deref()))
            }
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
                let font_size = style.font_size;
                let line_height = style.line_height * font_size;
                let metrics = Metrics::new(font_size, line_height);

                let mut buffer = Buffer::new(font_system, metrics);

                let family = if style.font_family.is_empty() {
                    Family::SansSerif
                } else {
                    Family::Name(&style.font_family)
                };

                let weight = Weight(style.font_weight.0);

                let attrs = Attrs::new()
                    .family(family)
                    .weight(weight);

                let alignment = match style.text_align {
                    TextAlign::Right => Some(Align::Right),
                    TextAlign::Center => Some(Align::Center),
                    TextAlign::Left => None, // Left is the default
                };

                buffer.set_size(font_system, Some(rect.width), Some(rect.height));
                buffer.set_text(font_system, &content, &attrs, Shaping::Advanced, alignment);
                buffer.shape_until_scroll(font_system, false);

                self.buffers.insert(node.id, buffer);
            }
        }

        for &child_id in &node.children {
            self.prepare_node(doc, child_id, font_system, data_values);
        }
    }

    /// Build TextArea references for the renderer.
    /// The returned Vec borrows from `self.buffers`, so `self` must outlive the render call.
    pub fn text_areas<'a>(&'a self, doc: &'a CxrdDocument) -> Vec<TextArea<'a>> {
        let mut areas = Vec::new();

        for (node_id, buffer) in &self.buffers {
            let node = match doc.get_node(*node_id) {
                Some(n) => n,
                None => continue,
            };

            let rect = &node.layout.content_rect;
            let color = &node.style.color;

            areas.push(TextArea {
                buffer,
                left: rect.x,
                top: rect.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: rect.x as i32,
                    top: rect.y as i32,
                    right: (rect.x + rect.width) as i32,
                    bottom: (rect.y + rect.height) as i32,
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
