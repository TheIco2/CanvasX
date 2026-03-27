// canvasx-runtime/src/scene/graph.rs
//
// The SceneGraph is the top-level coordinator: it owns a CXRD document,
// runs layout, generates paint calls, and manages text buffers.
// The main loop calls SceneGraph methods each frame.

use std::collections::HashMap;
use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::NodeKind;
use crate::cxrd::value::Dimension;
use crate::gpu::vertex::UiInstance;
use crate::layout::engine::compute_layout;
use crate::scene::paint::{paint_document, GradientTexture, GradientCacheKey};
use crate::scene::text::TextPainter;
use crate::animate::timeline::AnimationTimeline;

/// The scene graph coordinator.
pub struct SceneGraph {
    /// The current CXRD document being rendered.
    pub document: CxrdDocument,

    /// Text painting state.
    pub text_painter: TextPainter,

    /// Live data values from IPC (e.g., "cpu.usage" → "47%").
    pub data_values: HashMap<String, String>,

    /// Animation timeline.
    pub timeline: AnimationTimeline,

    /// Whether layout needs recomputation.
    layout_dirty: bool,

    /// Cached paint output.
    pub cached_instances: Vec<UiInstance>,

    /// Gradient textures from the last paint pass (to be uploaded each frame).
    pub cached_gradient_textures: Vec<GradientTexture>,

    /// Whether gradient textures were regenerated and need GPU upload.
    gradient_textures_dirty: bool,

    /// Whether paint output needs regeneration.
    paint_dirty: bool,

    /// Cached gradient textures (keyed by gradient parameters + size).
    /// Avoids re-rasterizing identical gradients every frame.
    gradient_cache: HashMap<GradientCacheKey, GradientTexture>,

    /// Whether gradient cache was used this frame (avoid re-uploading unchanged gradients).
    gradient_cache_dirty: bool,

    /// Cached list of nodes with data-bind attributes.
    data_bound_nodes: Vec<u32>,

    /// Whether the data-bound nodes list needs rebuilding.
    data_bound_dirty: bool,
}

impl SceneGraph {
    /// Create a new scene graph with a given document.
    pub fn new(document: CxrdDocument) -> Self {
        Self {
            document,
            text_painter: TextPainter::new(),
            data_values: HashMap::new(),
            timeline: AnimationTimeline::new(),
            layout_dirty: true,
            cached_instances: Vec::new(),
            cached_gradient_textures: Vec::new(),
            gradient_textures_dirty: true,
            paint_dirty: true,
            data_bound_nodes: Vec::new(),
            data_bound_dirty: true,
            gradient_cache: HashMap::new(),
            gradient_cache_dirty: true,
        }
    }

    /// Load a new document (e.g. when wallpaper changes).
    pub fn load_document(&mut self, doc: CxrdDocument) {
        self.document = doc;
        self.layout_dirty = true;
        self.cached_instances.clear();
        self.cached_gradient_textures.clear();
        self.timeline = AnimationTimeline::new();
        self.paint_dirty = true;
        self.gradient_textures_dirty = true;
        self.data_bound_dirty = true;
        // Keep gradient_cache for reuse across document changes
        // (safe because cache keys include size)
        self.gradient_cache_dirty = true;
    }

    /// Merge changes from the JS runtime's document without replacing the
    /// entire scene document.  This preserves runtime-only state (`hovered`,
    /// `layout`, `clip`) on every node, avoiding the hover-reset bug and the
    /// cost of a full clone + re-layout when only a handful of nodes changed.
    pub fn merge_js_document(&mut self, js_doc: &CxrdDocument) {
        let our = &mut self.document;

        // If the node count changed (innerHTML added/removed nodes), resize.
        if js_doc.nodes.len() > our.nodes.len() {
            our.nodes.resize_with(js_doc.nodes.len(), || {
                crate::cxrd::node::CxrdNode::container(0)
            });
        }

        // Sync per-node content fields from JS while keeping runtime fields.
        for (i, js_node) in js_doc.nodes.iter().enumerate() {
            let node = &mut our.nodes[i];
            // Sync DOM-authoritative fields.
            node.id = js_node.id;
            node.tag = js_node.tag.clone();
            node.html_id = js_node.html_id.clone();
            node.classes = js_node.classes.clone();
            node.attributes = js_node.attributes.clone();
            node.kind = js_node.kind.clone();
            node.style = js_node.style.clone();
            node.hover_style = js_node.hover_style.clone();
            node.active_style = js_node.active_style.clone();
            node.focus_style = js_node.focus_style.clone();
            node.children = js_node.children.clone();
            node.events = js_node.events.clone();
            node.animations = js_node.animations.clone();
            // `hovered` and `layout` are NOT overwritten — they're runtime-only.
        }

        // Sync document-level fields that JS may have changed.
        our.free_list = js_doc.free_list.clone();
        our.variables = js_doc.variables.clone();
        our.background = js_doc.background;

        self.layout_dirty = true;
        self.paint_dirty = true;
        self.data_bound_dirty = true;
    }

    /// Update live data from IPC.
    pub fn update_data(&mut self, key: String, value: String) {
        self.data_values.insert(key, value);
        self.apply_custom_data_tags();
    }

    /// Bulk update data values. Takes ownership to avoid clone.
    pub fn update_data_batch(&mut self, values: HashMap<String, String>) {
        if values.is_empty() {
            return;
        }
        self.data_values.extend(values);
        self.apply_custom_data_tags();
    }

    /// Mark layout as dirty (e.g. on resize, document change).
    pub fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
        self.paint_dirty = true;
    }

    /// Mark only the paint pass as dirty (e.g. on hover change).
    pub fn invalidate_paint(&mut self) {
        self.paint_dirty = true;
    }

    /// Run a frame tick: layout → animate → paint.
    /// Updates cached_instances and cached_gradient_textures.
    /// Access results via cached_instances, document.background, and text_areas() after calling.
    pub fn tick(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        dt: f32,
        font_system: &mut glyphon::FontSystem,
    ) {
        // 1. Re-layout if dirty.
        if self.layout_dirty {
            compute_layout(&mut self.document, viewport_width, viewport_height);
            self.layout_dirty = false;
            self.paint_dirty = true;
        }

        // 2. Advance animations.
        if self.timeline.has_active() {
            self.timeline.advance(&mut self.document, dt);
            self.paint_dirty = true;
        }

        // 3. Prepare text.
        self.text_painter.prepare(&self.document, font_system, &self.data_values);

        // 4. Paint only when necessary.
        if self.paint_dirty {
            let paint_output = paint_document(&self.document, &mut self.gradient_cache);
            self.cached_instances = paint_output.instances;
            self.cached_gradient_textures = paint_output.gradient_textures;
            self.gradient_textures_dirty = true;
            self.paint_dirty = false;
        }
    }

    /// Returns whether gradient textures were regenerated since last check.
    pub fn take_gradient_textures_dirty(&mut self) -> bool {
        let dirty = self.gradient_textures_dirty;
        self.gradient_textures_dirty = false;
        dirty
    }

    /// Get text areas for the current frame (call after tick).
    pub fn text_areas(&self) -> Vec<glyphon::TextArea<'_>> {
        self.text_painter.text_areas(&self.document)
    }

    fn apply_custom_data_tags(&mut self) {
        // Rebuild cached list of data-bound nodes if needed.
        if self.data_bound_dirty {
            self.data_bound_nodes.clear();
            for idx in 0..self.document.nodes.len() {
                let node = &self.document.nodes[idx];
                let has_binding = node.attributes.contains_key("binding")
                    || node.attributes.contains_key("data-binding")
                    || node.attributes.contains_key("data-bind");
                if has_binding {
                    self.data_bound_nodes.push(idx as u32);
                }
            }
            self.data_bound_dirty = false;
        }

        let mut any_layout_changed = false;
        // Clone the list since we'll mutate document
        let bound_nodes = self.data_bound_nodes.clone();

        for &node_id in &bound_nodes {
            let (tag, attrs, children) = match self.document.get_node(node_id) {
                Some(node) => (
                    node.tag.clone().unwrap_or_default(),
                    node.attributes.clone(),
                    node.children.clone(),
                ),
                None => continue,
            };

            let binding_raw = attrs
                .get("binding")
                .or_else(|| attrs.get("data-binding"))
                .or_else(|| attrs.get("data-bind"))
                .cloned();
            let Some(binding_raw) = binding_raw else { continue; };

            let keys = parse_binding_keys(&binding_raw);
            if keys.is_empty() {
                continue;
            }

            let meta = attrs.get("meta").cloned().unwrap_or_default().to_ascii_lowercase();
            let stack = meta.split_whitespace().any(|m| m == "stack") || meta.contains("stack");

            if tag == "data-bind" {
                let values: Vec<String> = keys
                    .iter()
                    .filter_map(|k| self.data_values.get(k).cloned())
                    .collect();
                let text = if stack { values.join("\n") } else { values.join(" ") };

                let mut text_child = children.iter().find_map(|cid| {
                    self.document.get_node(*cid).and_then(|n| {
                        if matches!(n.kind, NodeKind::Text { .. }) { Some(*cid) } else { None }
                    })
                });

                if text_child.is_none() {
                    let mut new_text = crate::cxrd::node::CxrdNode::text(0, "");
                    if let Some(parent) = self.document.get_node(node_id) {
                        new_text.style.color = parent.style.color;
                        new_text.style.font_size = parent.style.font_size;
                        new_text.style.font_family = parent.style.font_family.clone();
                        new_text.style.font_weight = parent.style.font_weight;
                        new_text.style.letter_spacing = parent.style.letter_spacing;
                        new_text.style.line_height = parent.style.line_height;
                        new_text.style.text_align = parent.style.text_align;
                        new_text.style.text_transform = parent.style.text_transform;
                    }
                    let tid = self.document.add_node(new_text);
                    self.document.add_child(node_id, tid);
                    text_child = Some(tid);
                }

                if let Some(tid) = text_child {
                    if let Some(node) = self.document.get_node_mut(tid) {
                        if let NodeKind::Text { content } = &mut node.kind {
                            if *content != text {
                                *content = text;
                            }
                        }
                    }
                }
            } else if tag == "data-bar" {
                let max = attrs
                    .get("max")
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|v| *v > 0.0)
                    .unwrap_or(100.0);

                let nums: Vec<f32> = keys
                    .iter()
                    .filter_map(|k| self.data_values.get(k))
                    .filter_map(|v| parse_numeric_value(v))
                    .collect();
                if nums.is_empty() {
                    continue;
                }

                let pct = if stack && nums.len() > 1 {
                    let sum: f32 = nums.iter().copied().sum();
                    ((sum / (max * nums.len() as f32)) * 100.0).clamp(0.0, 100.0)
                } else {
                    ((nums[0] / max) * 100.0).clamp(0.0, 100.0)
                };

                if let Some(node) = self.document.get_node_mut(node_id) {
                    match node.style.width {
                        Dimension::Percent(old) if (old - pct).abs() < 0.1 => {}
                        _ => {
                            node.style.width = Dimension::Percent(pct);
                            any_layout_changed = true;
                        }
                    }
                }
            }
        }

        if any_layout_changed {
            self.layout_dirty = true;
            self.paint_dirty = true;
        }
    }
}

fn parse_binding_keys(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c == '|' || c == ';' || c == '\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn parse_numeric_value(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed
        .trim_end_matches('%')
        .replace(',', "")
        .replace("MB/s", "")
        .replace("GB/s", "")
        .replace("KB/s", "")
        .replace("B/s", "")
        .replace('W', "")
        .trim()
        .to_string();
    cleaned.parse::<f32>().ok()
}
