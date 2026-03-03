// canvasx-runtime/src/scripting/canvas2d.rs
//
// Canvas 2D rendering context backed by tiny-skia.
// Each canvas element in the DOM gets a CanvasBuffer which holds a pixel buffer.
// JS draw calls are translated to tiny-skia operations.
// The resulting pixel data is uploaded to a wgpu texture each frame.

use std::collections::HashMap;

/// Unique identifier for a canvas instance.
pub type CanvasId = u32;

/// Unique identifier for a gradient object created by JS.
pub type GradientId = u32;

/// A stored gradient definition (for deferred application).
#[derive(Clone)]
pub enum GradientDef {
    Linear {
        x0: f32, y0: f32, x1: f32, y1: f32,
        stops: Vec<(f32, tiny_skia::Color)>,
    },
    Radial {
        x0: f32, y0: f32, r0: f32,
        x1: f32, y1: f32, r1: f32,
        stops: Vec<(f32, tiny_skia::Color)>,
    },
}

/// The paint source for fill/stroke — can be a solid color, gradient, or pattern.
#[derive(Clone)]
pub enum PaintStyle {
    Color(tiny_skia::Color),
    Gradient(GradientId),
    Pattern(CanvasId),
}

impl Default for PaintStyle {
    fn default() -> Self {
        PaintStyle::Color(tiny_skia::Color::BLACK)
    }
}

/// Saved state entry (for save/restore).
#[derive(Clone)]
struct SavedState {
    transform: tiny_skia::Transform,
    fill_style: PaintStyle,
    stroke_style: PaintStyle,
    line_width: f32,
    global_alpha: f32,
    blend_mode: tiny_skia::BlendMode,
    font_size: f32,
    font_family: String,
    text_align: String,
    text_baseline: String,
    clip_path: Option<tiny_skia::Path>,
}

/// A single canvas pixel buffer with full Canvas 2D state machine.
pub struct CanvasBuffer {
    pub width: u32,
    pub height: u32,
    pub pixmap: tiny_skia::Pixmap,
    pub dirty: bool,

    // --- State machine ---
    transform: tiny_skia::Transform,
    fill_style: PaintStyle,
    stroke_style: PaintStyle,
    line_width: f32,
    global_alpha: f32,
    blend_mode: tiny_skia::BlendMode,
    font_size: f32,
    font_family: String,
    text_align: String,
    text_baseline: String,
    clip_path: Option<tiny_skia::Path>,

    // Path building
    path_builder: Option<tiny_skia::PathBuilder>,

    // State stack (save/restore)
    state_stack: Vec<SavedState>,
}

impl CanvasBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))
            .unwrap_or_else(|| tiny_skia::Pixmap::new(1, 1).unwrap());
        Self {
            width: width.max(1),
            height: height.max(1),
            pixmap,
            dirty: true,
            transform: tiny_skia::Transform::identity(),
            fill_style: PaintStyle::Color(tiny_skia::Color::BLACK),
            stroke_style: PaintStyle::Color(tiny_skia::Color::BLACK),
            line_width: 1.0,
            global_alpha: 1.0,
            blend_mode: tiny_skia::BlendMode::SourceOver,
            font_size: 10.0,
            font_family: "sans-serif".into(),
            text_align: "start".into(),
            text_baseline: "alphabetic".into(),
            clip_path: None,
            path_builder: None,
            state_stack: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        if w != self.width || h != self.height {
            self.width = w;
            self.height = h;
            self.pixmap = tiny_skia::Pixmap::new(w, h)
                .unwrap_or_else(|| tiny_skia::Pixmap::new(1, 1).unwrap());
            self.dirty = true;
        }
    }

    /// Get current fill style type as string (for diagnostics).
    pub fn fill_style_type(&self) -> &'static str {
        match &self.fill_style {
            PaintStyle::Color(_) => "Color",
            PaintStyle::Gradient(_) => "Gradient",
            PaintStyle::Pattern(_) => "Pattern",
        }
    }

    /// Get the raw RGBA pixel data.
    pub fn pixels(&self) -> &[u8] {
        self.pixmap.data()
    }

    // ─── Drawing methods ────────────────────────────────────────────

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = match tiny_skia::Rect::from_xywh(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        // Fill with transparent black
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::TRANSPARENT);
        paint.blend_mode = tiny_skia::BlendMode::Source; // overwrite
        self.pixmap.fill_rect(rect, &paint, self.transform, None);
        self.dirty = true;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let rect = match tiny_skia::Rect::from_xywh(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        let paint = self.make_fill_paint();
        self.pixmap.fill_rect(rect, &paint, self.transform, None);
        self.dirty = true;
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + h);
        pb.line_to(x, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            let paint = self.make_stroke_paint();
            let stroke = self.make_stroke();
            self.pixmap.stroke_path(&path, &paint, &stroke, self.transform, None);
            self.dirty = true;
        }
    }

    pub fn begin_path(&mut self) {
        self.path_builder = Some(tiny_skia::PathBuilder::new());
    }

    pub fn close_path(&mut self) {
        if let Some(ref mut pb) = self.path_builder {
            pb.close();
        }
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        if let Some(ref mut pb) = self.path_builder {
            pb.move_to(x, y);
        }
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        if let Some(ref mut pb) = self.path_builder {
            pb.line_to(x, y);
        }
    }

    pub fn arc(&mut self, x: f32, y: f32, radius: f32, start_angle: f32, end_angle: f32, _anticlockwise: bool) {
        if let Some(ref mut pb) = self.path_builder {
            // Approximate arc with line segments (tiny-skia doesn't have native arc)
            let steps = ((end_angle - start_angle).abs() / (std::f32::consts::PI / 16.0)).ceil() as usize;
            let steps = steps.max(4).min(128);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = start_angle + t * (end_angle - start_angle);
                let px = x + radius * angle.cos();
                let py = y + radius * angle.sin();
                if i == 0 {
                    // If path has previous points, lineTo; otherwise moveTo
                    pb.line_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
        }
    }

    pub fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        if let Some(ref mut pb) = self.path_builder {
            pb.cubic_to(cp1x, cp1y, cp2x, cp2y, x, y);
        }
    }

    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        if let Some(ref mut pb) = self.path_builder {
            pb.quad_to(cpx, cpy, x, y);
        }
    }

    pub fn fill(&mut self) {
        let path = match self.path_builder.take() {
            Some(pb) => pb.finish(),
            None => return,
        };
        if let Some(path) = path {
            let paint = self.make_fill_paint();
            self.pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, self.transform, None);
            self.dirty = true;
        }
        // Restore path builder so further operations can continue
        self.path_builder = Some(tiny_skia::PathBuilder::new());
    }

    pub fn stroke(&mut self) {
        let path = match self.path_builder.take() {
            Some(pb) => pb.finish(),
            None => return,
        };
        if let Some(path) = path {
            let paint = self.make_stroke_paint();
            let stroke = self.make_stroke();
            self.pixmap.stroke_path(&path, &paint, &stroke, self.transform, None);
            self.dirty = true;
        }
        self.path_builder = Some(tiny_skia::PathBuilder::new());
    }

    // ─── Transform ──────────────────────────────────────────────────

    pub fn save(&mut self) {
        self.state_stack.push(SavedState {
            transform: self.transform,
            fill_style: self.fill_style.clone(),
            stroke_style: self.stroke_style.clone(),
            line_width: self.line_width,
            global_alpha: self.global_alpha,
            blend_mode: self.blend_mode,
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            text_align: self.text_align.clone(),
            text_baseline: self.text_baseline.clone(),
            clip_path: self.clip_path.clone(),
        });
    }

    pub fn restore(&mut self) {
        if let Some(saved) = self.state_stack.pop() {
            self.transform = saved.transform;
            self.fill_style = saved.fill_style;
            self.stroke_style = saved.stroke_style;
            self.line_width = saved.line_width;
            self.global_alpha = saved.global_alpha;
            self.blend_mode = saved.blend_mode;
            self.font_size = saved.font_size;
            self.font_family = saved.font_family;
            self.text_align = saved.text_align;
            self.text_baseline = saved.text_baseline;
            self.clip_path = saved.clip_path;
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.transform = self.transform.post_translate(x, y);
    }

    pub fn rotate(&mut self, angle: f32) {
        // tiny-skia rotate takes degrees
        let degrees = angle * 180.0 / std::f32::consts::PI;
        self.transform = self.transform.post_concat(
            tiny_skia::Transform::from_rotate(degrees)
        );
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.transform = self.transform.post_scale(sx, sy);
    }

    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.transform = tiny_skia::Transform::from_row(a, b, c, d, e, f);
    }

    pub fn reset_transform(&mut self) {
        self.transform = tiny_skia::Transform::identity();
    }

    // ─── Style setters ──────────────────────────────────────────────
    /// Returns true if the current fill style uses a gradient or pattern.
    pub fn uses_gradient_fill(&self) -> bool {
        matches!(self.fill_style, PaintStyle::Gradient(_) | PaintStyle::Pattern(_))
    }

    /// Returns true if the current stroke style uses a gradient or pattern.
    pub fn uses_gradient_stroke(&self) -> bool {
        matches!(self.stroke_style, PaintStyle::Gradient(_) | PaintStyle::Pattern(_))
    }
    pub fn set_fill_style_color(&mut self, color: tiny_skia::Color) {
        self.fill_style = PaintStyle::Color(color);
    }

    pub fn set_fill_style_gradient(&mut self, gradient_id: GradientId) {
        self.fill_style = PaintStyle::Gradient(gradient_id);
    }

    pub fn set_fill_style_pattern(&mut self, canvas_id: CanvasId) {
        self.fill_style = PaintStyle::Pattern(canvas_id);
    }

    pub fn set_stroke_style_color(&mut self, color: tiny_skia::Color) {
        self.stroke_style = PaintStyle::Color(color);
    }

    pub fn set_stroke_style_gradient(&mut self, gradient_id: GradientId) {
        self.stroke_style = PaintStyle::Gradient(gradient_id);
    }

    pub fn set_line_width(&mut self, w: f32) {
        self.line_width = w;
    }

    pub fn set_global_alpha(&mut self, a: f32) {
        self.global_alpha = a.clamp(0.0, 1.0);
    }

    pub fn set_blend_mode(&mut self, mode: &str) {
        self.blend_mode = match mode {
            "source-over" => tiny_skia::BlendMode::SourceOver,
            "source-in" => tiny_skia::BlendMode::SourceIn,
            "source-out" => tiny_skia::BlendMode::SourceOut,
            "source-atop" => tiny_skia::BlendMode::SourceAtop,
            "destination-over" => tiny_skia::BlendMode::DestinationOver,
            "destination-in" => tiny_skia::BlendMode::DestinationIn,
            "destination-out" => tiny_skia::BlendMode::DestinationOut,
            "destination-atop" => tiny_skia::BlendMode::DestinationAtop,
            "lighter" | "add" => tiny_skia::BlendMode::Plus,
            "copy" => tiny_skia::BlendMode::Source,
            "xor" => tiny_skia::BlendMode::Xor,
            "multiply" => tiny_skia::BlendMode::Multiply,
            "screen" => tiny_skia::BlendMode::Screen,
            "overlay" => tiny_skia::BlendMode::Overlay,
            "darken" => tiny_skia::BlendMode::Darken,
            "lighten" => tiny_skia::BlendMode::Lighten,
            "color-dodge" => tiny_skia::BlendMode::ColorDodge,
            "color-burn" => tiny_skia::BlendMode::ColorBurn,
            "hard-light" => tiny_skia::BlendMode::HardLight,
            "soft-light" => tiny_skia::BlendMode::SoftLight,
            "difference" => tiny_skia::BlendMode::Difference,
            "exclusion" => tiny_skia::BlendMode::Exclusion,
            "hue" => tiny_skia::BlendMode::Hue,
            "saturation" => tiny_skia::BlendMode::Saturation,
            "color" => tiny_skia::BlendMode::Color,
            "luminosity" => tiny_skia::BlendMode::Luminosity,
            _ => tiny_skia::BlendMode::SourceOver,
        };
    }

    pub fn get_blend_mode_str(&self) -> &str {
        match self.blend_mode {
            tiny_skia::BlendMode::SourceOver => "source-over",
            tiny_skia::BlendMode::Screen => "screen",
            tiny_skia::BlendMode::Multiply => "multiply",
            tiny_skia::BlendMode::Overlay => "overlay",
            tiny_skia::BlendMode::Plus => "lighter",
            tiny_skia::BlendMode::Source => "copy",
            _ => "source-over",
        }
    }

    pub fn set_font(&mut self, font_str: &str) {
        // Parse basic CSS font string: "12px sans-serif", "bold 14px Arial"
        let parts: Vec<&str> = font_str.split_whitespace().collect();
        for part in &parts {
            if part.ends_with("px") {
                if let Ok(size) = part.trim_end_matches("px").parse::<f32>() {
                    self.font_size = size;
                }
            }
        }
        if let Some(last) = parts.last() {
            if !last.ends_with("px") && *last != "bold" && *last != "italic" && *last != "normal" {
                self.font_family = last.trim_matches('\'').trim_matches('"').to_string();
            }
        }
    }

    // ─── drawImage ──────────────────────────────────────────────────

    /// Draw another canvas buffer onto this one.
    pub fn draw_image(&mut self, source: &CanvasBuffer, dx: f32, dy: f32, dw: f32, dh: f32) {
        // Scale the source pixmap to fit dw×dh at (dx, dy)
        let sx = dw / source.width as f32;
        let sy = dh / source.height as f32;

        let paint = tiny_skia::PixmapPaint {
            opacity: self.global_alpha,
            blend_mode: self.blend_mode,
            quality: tiny_skia::FilterQuality::Bilinear,
        };

        // Compose transform: first scale source, then apply current transform
        let img_transform = self.transform
            .pre_translate(dx, dy)
            .pre_scale(sx, sy);

        self.pixmap.draw_pixmap(
            0, 0,
            source.pixmap.as_ref(),
            &paint,
            img_transform,
            None,
        );
        self.dirty = true;
    }

    /// Fill the entire canvas with transparent black.
    pub fn clear(&mut self) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        self.dirty = true;
    }

    // ─── fillText (basic) ───────────────────────────────────────────

    pub fn fill_text(&mut self, _text: &str, _x: f32, _y: f32) {
        // TODO: Implement text rendering via cosmic-text → tiny-skia.
        // For now, this is a no-op. Text rendering in Canvas 2D is complex
        // and not critical for the wallpaper background (decorative only).
        self.dirty = true;
    }

    // ─── Internal helpers ───────────────────────────────────────────

    fn make_fill_paint(&self) -> tiny_skia::Paint<'static> {
        let mut paint = tiny_skia::Paint::default();
        paint.anti_alias = true;
        paint.blend_mode = self.blend_mode;

        match &self.fill_style {
            PaintStyle::Color(c) => {
                // Apply global alpha
                let r = c.red();
                let g = c.green();
                let b = c.blue();
                let a = c.alpha() * self.global_alpha;
                paint.set_color(tiny_skia::Color::from_rgba(r, g, b, a).unwrap_or(*c));
            }
            PaintStyle::Gradient(_gid) => {
                // Gradient will be resolved by the CanvasManager gradient-aware methods.
                // Transparent fallback so unresolved gradient draws are invisible.
                paint.set_color(tiny_skia::Color::TRANSPARENT);
            }
            PaintStyle::Pattern(_) => {
                // Pattern will be resolved by pattern-aware fill methods.
                // Transparent fallback so unresolved pattern draws are invisible.
                paint.set_color(tiny_skia::Color::TRANSPARENT);
            }
        }
        paint
    }

    fn make_stroke_paint(&self) -> tiny_skia::Paint<'static> {
        let mut paint = tiny_skia::Paint::default();
        paint.anti_alias = true;
        paint.blend_mode = self.blend_mode;

        match &self.stroke_style {
            PaintStyle::Color(c) => {
                let r = c.red();
                let g = c.green();
                let b = c.blue();
                let a = c.alpha() * self.global_alpha;
                paint.set_color(tiny_skia::Color::from_rgba(r, g, b, a).unwrap_or(*c));
            }
            PaintStyle::Gradient(_gid) => {
                paint.set_color(tiny_skia::Color::TRANSPARENT);
            }
            PaintStyle::Pattern(_) => {
                paint.set_color(tiny_skia::Color::TRANSPARENT);
            }
        }
        paint
    }

    fn make_stroke(&self) -> tiny_skia::Stroke {
        let mut stroke = tiny_skia::Stroke::default();
        stroke.width = self.line_width;
        stroke
    }
}

/// Manages all canvas buffers and gradient definitions.
pub struct CanvasManager {
    pub buffers: HashMap<CanvasId, CanvasBuffer>,
    pub gradients: HashMap<GradientId, GradientDef>,
    next_canvas_id: CanvasId,
    next_gradient_id: GradientId,
}

impl CanvasManager {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            gradients: HashMap::new(),
            next_canvas_id: 1000, // Start high to avoid conflict with node IDs
            next_gradient_id: 1,
        }
    }

    /// Create a new canvas buffer, returns its ID.
    pub fn create_canvas(&mut self, width: u32, height: u32) -> CanvasId {
        let id = self.next_canvas_id;
        self.next_canvas_id += 1;
        self.buffers.insert(id, CanvasBuffer::new(width, height));
        id
    }

    /// Create a gradient definition, returns its ID.
    pub fn create_gradient(&mut self, def: GradientDef) -> GradientId {
        let id = self.next_gradient_id;
        self.next_gradient_id += 1;
        self.gradients.insert(id, def);
        id
    }

    /// Add a color stop to an existing gradient.
    pub fn add_gradient_stop(&mut self, gradient_id: GradientId, offset: f32, color: tiny_skia::Color) {
        if let Some(grad) = self.gradients.get_mut(&gradient_id) {
            match grad {
                GradientDef::Linear { stops, .. } | GradientDef::Radial { stops, .. } => {
                    stops.push((offset, color));
                }
            }
        }
    }

    /// Resolve a fill/stroke paint that may reference a gradient.
    /// Returns a fully-resolved tiny-skia Paint.
    pub fn resolve_paint<'a>(
        &'a self,
        style: &PaintStyle,
        global_alpha: f32,
        blend_mode: tiny_skia::BlendMode,
    ) -> tiny_skia::Paint<'a> {
        let mut paint = tiny_skia::Paint::default();
        paint.anti_alias = true;
        paint.blend_mode = blend_mode;

        match style {
            PaintStyle::Color(c) => {
                let a = c.alpha() * global_alpha;
                paint.set_color(
                    tiny_skia::Color::from_rgba(c.red(), c.green(), c.blue(), a)
                        .unwrap_or(*c)
                );
            }
            PaintStyle::Gradient(gid) => {
                if let Some(def) = self.gradients.get(gid) {
                    match def {
                        GradientDef::Linear { x0, y0, x1, y1, stops } => {
                            if let Some(shader) = self.make_linear_gradient(*x0, *y0, *x1, *y1, stops, global_alpha) {
                                paint.shader = shader;
                            }
                        }
                        GradientDef::Radial { x0, y0, r0: _, x1, y1, r1, stops } => {
                            if let Some(shader) = self.make_radial_gradient(*x0, *y0, *x1, *y1, *r1, stops, global_alpha) {
                                paint.shader = shader;
                            }
                        }
                    }
                }
            }
            PaintStyle::Pattern(cid) => {
                if let Some(source) = self.buffers.get(cid) {
                    paint.shader = tiny_skia::Pattern::new(
                        source.pixmap.as_ref(),
                        tiny_skia::SpreadMode::Repeat,
                        tiny_skia::FilterQuality::Bilinear,
                        global_alpha,
                        tiny_skia::Transform::identity(),
                    );
                }
            }
        }
        paint
    }

    fn make_linear_gradient(
        &self,
        x0: f32, y0: f32, x1: f32, y1: f32,
        stops: &[(f32, tiny_skia::Color)],
        _global_alpha: f32,
    ) -> Option<tiny_skia::Shader<'static>> {
        make_linear_gradient_free(x0, y0, x1, y1, stops)
    }

    fn make_radial_gradient(
        &self,
        cx: f32, cy: f32,
        fx: f32, fy: f32, r: f32,
        stops: &[(f32, tiny_skia::Color)],
        _global_alpha: f32,
    ) -> Option<tiny_skia::Shader<'static>> {
        make_radial_gradient_free(cx, cy, fx, fy, r, stops)
    }

    /// Apply a resolved gradient paint to a canvas buffer's fill operation.
    /// This is called from the JS runtime when the canvas uses a gradient fill.
    pub fn fill_rect_with_gradient(
        &mut self,
        canvas_id: CanvasId,
        x: f32, y: f32, w: f32, h: f32,
    ) {
        // Get the fill style from the canvas, resolve the gradient, then paint.
        let (style, alpha, blend, transform, _clip) = {
            let canvas = match self.buffers.get(&canvas_id) {
                Some(c) => c,
                None => return,
            };
            (canvas.fill_style.clone(), canvas.global_alpha, canvas.blend_mode, canvas.transform, canvas.clip_path.clone())
        };
        // Use split borrow: borrow self.gradients separately from self.buffers
        let paint = resolve_non_pattern_paint(&self.gradients, &style, alpha, blend);
        let rect = match tiny_skia::Rect::from_xywh(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        if let Some(canvas) = self.buffers.get_mut(&canvas_id) {
            canvas.pixmap.fill_rect(rect, &paint, transform, None);
            canvas.dirty = true;
        }
    }

    /// Fill the current path with resolved gradient paint.
    pub fn fill_path_with_gradient(
        &mut self,
        canvas_id: CanvasId,
    ) {
        let (style, alpha, blend, transform, _clip, path) = {
            let canvas = match self.buffers.get_mut(&canvas_id) {
                Some(c) => c,
                None => return,
            };
            let path = canvas.path_builder.take().and_then(|pb| pb.finish());
            canvas.path_builder = Some(tiny_skia::PathBuilder::new());
            (canvas.fill_style.clone(), canvas.global_alpha, canvas.blend_mode, canvas.transform, canvas.clip_path.clone(), path)
        };
        if let Some(path) = path {
            // Use split borrow: borrow self.gradients separately from self.buffers
            let paint = resolve_non_pattern_paint(&self.gradients, &style, alpha, blend);
            if let Some(canvas) = self.buffers.get_mut(&canvas_id) {
                canvas.pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
                canvas.dirty = true;
            }
        }
    }

    /// Stroke the current path with resolved gradient paint.
    pub fn stroke_path_with_gradient(
        &mut self,
        canvas_id: CanvasId,
    ) {
        let (style, alpha, blend, transform, _clip, line_width, path) = {
            let canvas = match self.buffers.get_mut(&canvas_id) {
                Some(c) => c,
                None => return,
            };
            let path = canvas.path_builder.take().and_then(|pb| pb.finish());
            canvas.path_builder = Some(tiny_skia::PathBuilder::new());
            (canvas.stroke_style.clone(), canvas.global_alpha, canvas.blend_mode, canvas.transform,
             canvas.clip_path.clone(), canvas.line_width, path)
        };
        if let Some(path) = path {
            // Use split borrow: borrow self.gradients separately from self.buffers
            let paint = resolve_non_pattern_paint(&self.gradients, &style, alpha, blend);
            let mut stroke = tiny_skia::Stroke::default();
            stroke.width = line_width;
            if let Some(canvas) = self.buffers.get_mut(&canvas_id) {
                canvas.pixmap.stroke_path(&path, &paint, &stroke, transform, None);
                canvas.dirty = true;
            }
        }
    }

    /// Fill a rect using a pattern (tiling the source canvas).
    pub fn fill_rect_with_pattern(
        &mut self,
        canvas_id: CanvasId,
        x: f32, y: f32, w: f32, h: f32,
    ) {
        // Get pattern source canvas ID and drawing state from the target canvas
        let (pattern_cid, alpha, blend, transform) = {
            let canvas = match self.buffers.get(&canvas_id) {
                Some(c) => c,
                None => return,
            };
            let pid = match &canvas.fill_style {
                PaintStyle::Pattern(pid) => *pid,
                _ => return,
            };
            (pid, canvas.global_alpha, canvas.blend_mode, canvas.transform)
        };

        if pattern_cid == canvas_id { return; }

        // Clone the source pattern pixmap to avoid borrow conflicts
        let (src_w, src_h, src_pixmap) = match self.buffers.get(&pattern_cid) {
            Some(s) if s.width > 0 && s.height > 0 => {
                (s.width as f32, s.height as f32, s.pixmap.clone())
            }
            _ => return,
        };

        if let Some(target) = self.buffers.get_mut(&canvas_id) {
            let paint = tiny_skia::PixmapPaint {
                opacity: alpha,
                blend_mode: blend,
                quality: tiny_skia::FilterQuality::Nearest,
            };

            // Tile the pattern across the target rect
            let x_start = x as i32;
            let y_start = y as i32;
            let x_end = (x + w).ceil() as i32;
            let y_end = (y + h).ceil() as i32;
            let sw = src_w as i32;
            let sh = src_h as i32;
            if sw == 0 || sh == 0 { return; }

            let mut ty = y_start;
            while ty < y_end {
                let mut tx = x_start;
                while tx < x_end {
                    let tile_transform = transform.pre_translate(tx as f32, ty as f32);
                    target.pixmap.draw_pixmap(
                        0, 0,
                        src_pixmap.as_ref(),
                        &paint,
                        tile_transform,
                        None,
                    );
                    tx += sw;
                }
                ty += sh;
            }
            target.dirty = true;
        }
    }

    /// Draw one canvas onto another (for drawImage).
    pub fn draw_canvas_to_canvas(
        &mut self,
        target_id: CanvasId,
        source_id: CanvasId,
        dx: f32, dy: f32, dw: f32, dh: f32,
    ) {
        // We need to borrow source and target; handle same-ID edge case
        if target_id == source_id { return; }

        // Clone the source data to avoid borrow conflicts
        let source_data = match self.buffers.get(&source_id) {
            Some(s) => (s.width, s.height, s.pixmap.clone()),
            None => return,
        };

        if let Some(target) = self.buffers.get_mut(&target_id) {
            let sx = dw / source_data.0 as f32;
            let sy = dh / source_data.1 as f32;

            let paint = tiny_skia::PixmapPaint {
                opacity: target.global_alpha,
                blend_mode: target.blend_mode,
                quality: tiny_skia::FilterQuality::Bilinear,
            };

            let img_transform = target.transform
                .pre_translate(dx, dy)
                .pre_scale(sx, sy);

            target.pixmap.draw_pixmap(
                0, 0,
                source_data.2.as_ref(),
                &paint,
                img_transform,
                None,
            );
            target.dirty = true;
        }
    }
}

/// Free function to build a linear gradient shader (no self borrow needed).
fn make_linear_gradient_free(
    x0: f32, y0: f32, x1: f32, y1: f32,
    stops: &[(f32, tiny_skia::Color)],
) -> Option<tiny_skia::Shader<'static>> {
    if stops.len() < 2 { return None; }
    let ts_stops: Vec<tiny_skia::GradientStop> = stops.iter().map(|(offset, color)| {
        tiny_skia::GradientStop::new(*offset, *color)
    }).collect();
    tiny_skia::LinearGradient::new(
        tiny_skia::Point::from_xy(x0, y0),
        tiny_skia::Point::from_xy(x1, y1),
        ts_stops,
        tiny_skia::SpreadMode::Pad,
        tiny_skia::Transform::identity(),
    )
}

/// Free function to build a radial gradient shader (no self borrow needed).
fn make_radial_gradient_free(
    cx: f32, cy: f32,
    fx: f32, fy: f32, r: f32,
    stops: &[(f32, tiny_skia::Color)],
) -> Option<tiny_skia::Shader<'static>> {
    if stops.len() < 2 { return None; }
    let ts_stops: Vec<tiny_skia::GradientStop> = stops.iter().map(|(offset, color)| {
        tiny_skia::GradientStop::new(*offset, *color)
    }).collect();
    tiny_skia::RadialGradient::new(
        tiny_skia::Point::from_xy(fx, fy),
        tiny_skia::Point::from_xy(cx, cy),
        r,
        ts_stops,
        tiny_skia::SpreadMode::Pad,
        tiny_skia::Transform::identity(),
    )
}

/// Resolve a PaintStyle into a Paint without borrowing the entire CanvasManager.
/// This enables split borrows: borrow `gradients` separately from `buffers`.
/// Only handles Color and Gradient styles (not Pattern, which needs buffer access).
fn resolve_non_pattern_paint(
    gradients: &std::collections::HashMap<u32, GradientDef>,
    style: &PaintStyle,
    global_alpha: f32,
    blend_mode: tiny_skia::BlendMode,
) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.anti_alias = true;
    paint.blend_mode = blend_mode;
    match style {
        PaintStyle::Color(c) => {
            let a = c.alpha() * global_alpha;
            paint.set_color(
                tiny_skia::Color::from_rgba(c.red(), c.green(), c.blue(), a)
                    .unwrap_or(*c)
            );
        }
        PaintStyle::Gradient(gid) => {
            if let Some(def) = gradients.get(gid) {
                match def {
                    GradientDef::Linear { x0, y0, x1, y1, stops } => {
                        if let Some(shader) = make_linear_gradient_free(*x0, *y0, *x1, *y1, stops) {
                            paint.shader = shader;
                        }
                    }
                    GradientDef::Radial { x0, y0, r0: _, x1, y1, r1, stops } => {
                        if let Some(shader) = make_radial_gradient_free(*x0, *y0, *x1, *y1, *r1, stops) {
                            paint.shader = shader;
                        }
                    }
                }
            }
        }
        PaintStyle::Pattern(_) => {
            // Pattern requires buffer borrow; not handled by this free function.
            // Use transparent so the draw is invisible rather than solid black.
            paint.set_color(tiny_skia::Color::TRANSPARENT);
        }
    }
    paint
}

/// Parse a CSS color string like "rgba(255,0,0,0.5)" or "#ff0000" or "red".
pub fn parse_css_color(s: &str) -> Option<tiny_skia::Color> {
    let s = s.trim();

    // rgba(r, g, b, a)
    if s.starts_with("rgba(") && s.ends_with(')') {
        let inner = &s[5..s.len()-1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 4 {
            let r = parts[0].trim().parse::<f32>().ok()?;
            let g = parts[1].trim().parse::<f32>().ok()?;
            let b = parts[2].trim().parse::<f32>().ok()?;
            let a = parts[3].trim().parse::<f32>().ok()?;
            return tiny_skia::Color::from_rgba(r / 255.0, g / 255.0, b / 255.0, a);
        }
    }

    // rgb(r, g, b)
    if s.starts_with("rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len()-1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse::<f32>().ok()?;
            let g = parts[1].trim().parse::<f32>().ok()?;
            let b = parts[2].trim().parse::<f32>().ok()?;
            return tiny_skia::Color::from_rgba(r / 255.0, g / 255.0, b / 255.0, 1.0);
        }
    }

    // #rrggbb or #rgb
    if s.starts_with('#') {
        let hex = &s[1..];
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                return tiny_skia::Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                return tiny_skia::Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0);
            }
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                return tiny_skia::Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
            }
            _ => {}
        }
    }

    // Named colors (subset)
    match s {
        "black" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, 1.0),
        "white" => tiny_skia::Color::from_rgba(1.0, 1.0, 1.0, 1.0),
        "red" => tiny_skia::Color::from_rgba(1.0, 0.0, 0.0, 1.0),
        "green" => tiny_skia::Color::from_rgba(0.0, 0.502, 0.0, 1.0),
        "blue" => tiny_skia::Color::from_rgba(0.0, 0.0, 1.0, 1.0),
        "transparent" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        _ => None,
    }
}
