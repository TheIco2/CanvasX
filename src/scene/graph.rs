// canvasx-runtime/src/scene/graph.rs
//
// The SceneGraph is the top-level coordinator: it owns a CXRD document,
// runs layout, generates paint calls, and manages text buffers.
// The main loop calls SceneGraph methods each frame.

use std::collections::HashMap;
use crate::cxrd::document::CxrdDocument;
use crate::cxrd::value::Color;
use crate::gpu::vertex::UiInstance;
use crate::layout::engine::compute_layout;
use crate::scene::paint::paint_document;
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
    cached_instances: Vec<UiInstance>,
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
        }
    }

    /// Load a new document (e.g. when wallpaper changes).
    pub fn load_document(&mut self, doc: CxrdDocument) {
        self.document = doc;
        self.layout_dirty = true;
        self.cached_instances.clear();
        self.timeline = AnimationTimeline::new();
    }

    /// Update live data from IPC.
    pub fn update_data(&mut self, key: String, value: String) {
        self.data_values.insert(key, value);
        // Data changes don't require re-layout (text content changes don't
        // affect box sizes in our simplified model — they just reflow within
        // their container).
    }

    /// Bulk update data values.
    pub fn update_data_batch(&mut self, values: HashMap<String, String>) {
        self.data_values.extend(values);
    }

    /// Mark layout as dirty (e.g. on resize, document change).
    pub fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
    }

    /// Run a frame tick: layout → animate → paint.
    /// Returns the instance list and clear color.
    pub fn tick(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        dt: f32,
        font_system: &mut glyphon::FontSystem,
    ) -> (&[UiInstance], Color) {
        // 1. Re-layout if dirty.
        if self.layout_dirty {
            compute_layout(&mut self.document, viewport_width, viewport_height);
            self.layout_dirty = false;
        }

        // 2. Advance animations.
        self.timeline.advance(&mut self.document, dt);

        // 3. Prepare text.
        self.text_painter.prepare(&self.document, font_system, &self.data_values);

        // 4. Paint.
        self.cached_instances = paint_document(&self.document);

        (&self.cached_instances, self.document.background)
    }

    /// Get text areas for the current frame (call after tick).
    pub fn text_areas(&self) -> Vec<glyphon::TextArea<'_>> {
        self.text_painter.text_areas(&self.document)
    }
}
