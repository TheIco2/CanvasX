// prism-runtime/src/scripting/canvas2d.rs
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
    line_cap: tiny_skia::LineCap,
    line_join: tiny_skia::LineJoin,
    miter_limit: f32,
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
    pub text_align: String,
    pub text_baseline: String,
    clip_path: Option<tiny_skia::Path>,
    line_cap: tiny_skia::LineCap,
    line_join: tiny_skia::LineJoin,
    miter_limit: f32,

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
            line_cap: tiny_skia::LineCap::Butt,
            line_join: tiny_skia::LineJoin::Miter,
            miter_limit: 10.0,
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

    pub fn arc(&mut self, x: f32, y: f32, radius: f32, start_angle: f32, end_angle: f32, anticlockwise: bool) {
        if let Some(ref mut pb) = self.path_builder {
            // Determine sweep direction
            let (sa, ea) = if anticlockwise {
                if end_angle < start_angle {
                    (start_angle, end_angle)
                } else {
                    (start_angle, end_angle - std::f32::consts::TAU)
                }
            } else {
                if end_angle > start_angle {
                    (start_angle, end_angle)
                } else {
                    (start_angle, end_angle + std::f32::consts::TAU)
                }
            };
            // Approximate arc with line segments (tiny-skia doesn't have native arc)
            let steps = ((ea - sa).abs() / (std::f32::consts::PI / 16.0)).ceil() as usize;
            let steps = steps.max(4).min(128);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = sa + t * (ea - sa);
                let px = x + radius * angle.cos();
                let py = y + radius * angle.sin();
                pb.line_to(px, py);
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
            line_cap: self.line_cap,
            line_join: self.line_join,
            miter_limit: self.miter_limit,
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
            self.line_cap = saved.line_cap;
            self.line_join = saved.line_join;
            self.miter_limit = saved.miter_limit;
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.transform = self.transform.pre_translate(x, y);
    }

    pub fn rotate(&mut self, angle: f32) {
        self.transform = self.transform.pre_concat(
            tiny_skia::Transform::from_rotate(angle)
        );
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.transform = self.transform.pre_scale(sx, sy);
    }

    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.transform = tiny_skia::Transform::from_row(a, b, c, d, e, f);
    }

    pub fn reset_transform(&mut self) {
        self.transform = tiny_skia::Transform::identity();
    }

    pub fn clip_current_path(&mut self) {
        if let Some(ref pb) = self.path_builder {
            if let Some(path) = pb.clone().finish() {
                self.clip_path = Some(path);
            }
        }
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

    // ─── fillText ────────────────────────────────────────────────

    /// Rasterize text onto this canvas pixmap using cosmic-text.
    /// `font_system` and `swash_cache` are borrowed from `CanvasManager`.
    pub fn fill_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        font_system: &mut glyphon::FontSystem,
        swash_cache: &mut glyphon::cosmic_text::SwashCache,
    ) {
        use glyphon::cosmic_text::{Attrs as CAttrs, Buffer as CBuffer, Metrics as CMetrics, Shaping as CShaping, SwashContent};

        if text.is_empty() { return; }

        let font_size = self.font_size;
        let line_height = font_size * 1.2;
        let metrics = CMetrics::new(font_size, line_height);

        let mut buffer = CBuffer::new(font_system, metrics);

        let family_str = self.font_family.clone();
        let family = if family_str.is_empty() || family_str == "sans-serif" {
            glyphon::cosmic_text::Family::SansSerif
        } else {
            glyphon::cosmic_text::Family::Name(&family_str)
        };
        let attrs = CAttrs::new().family(family);

        // Set a wide bound so text doesn't wrap
        buffer.set_size(font_system, Some(self.width as f32 * 4.0), Some(line_height * 2.0));
        buffer.set_text(font_system, text, &attrs, CShaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        // Determine fill color
        let (fr, fg, fb, fa) = match &self.fill_style {
            PaintStyle::Color(c) => (c.red(), c.green(), c.blue(), c.alpha() * self.global_alpha),
            _ => (0.0, 0.0, 0.0, self.global_alpha),
        };

        // Compute text width for alignment (measured from layout runs)
        let text_width: f32 = buffer.layout_runs().map(|run| {
            run.glyphs.last().map_or(0.0, |g| g.x + g.w)
        }).fold(0.0_f32, f32::max);

        let align_offset = match self.text_align.as_str() {
            "center" => -text_width / 2.0,
            "right" | "end" => -text_width,
            _ => 0.0,
        };

        // Canvas 2D baseline: y is the alphabetic baseline.
        // cosmic-text line_y is from the top of the line.
        // Approximate ascent as ~80% of font_size for Latin fonts.
        let baseline_offset = -font_size * 0.8;
        let offset_y = match self.text_baseline.as_str() {
            "top" | "hanging" => 0.0,
            "middle" => -font_size * 0.4,
            "bottom" | "ideographic" => -font_size,
            _ => baseline_offset, // "alphabetic" (default)
        };

        let ox = x + align_offset;
        let oy = y + offset_y;

        let w = self.width;
        let h = self.height;
        // Use the transform to map glyph positions
        let tf = self.transform;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((ox, oy), 1.0);
                if let Some(image) = swash_cache.get_image(font_system, physical.cache_key) {
                    let gx = physical.x + image.placement.left;
                    let gy = physical.y - image.placement.top + run.line_y as i32;
                    let gw = image.placement.width as i32;
                    let gh = image.placement.height as i32;

                    match image.content {
                        SwashContent::Mask => {
                            for py in 0..gh {
                                for px in 0..gw {
                                    // Apply transform to the glyph pixel position
                                    let sx = (gx + px) as f32 + 0.5;
                                    let sy = (gy + py) as f32 + 0.5;
                                    let dx = (tf.sx * sx + tf.kx * sy + tf.tx) as i32;
                                    let dy = (tf.ky * sx + tf.sy * sy + tf.ty) as i32;
                                    if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 { continue; }
                                    let alpha = image.data[(py * gw + px) as usize] as f32 / 255.0 * fa;
                                    if alpha < 1.0 / 255.0 { continue; }
                                    let idx = ((dy as u32 * w + dx as u32) * 4) as usize;
                                    let pixels = self.pixmap.data_mut();
                                    if idx + 3 >= pixels.len() { continue; }
                                    // Premultiplied alpha compositing (src over dst)
                                    let dst_r = pixels[idx] as f32 / 255.0;
                                    let dst_g = pixels[idx + 1] as f32 / 255.0;
                                    let dst_b = pixels[idx + 2] as f32 / 255.0;
                                    let dst_a = pixels[idx + 3] as f32 / 255.0;
                                    let out_a = alpha + dst_a * (1.0 - alpha);
                                    if out_a > 0.001 {
                                        pixels[idx]     = ((fr * alpha + dst_r * dst_a * (1.0 - alpha)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 1] = ((fg * alpha + dst_g * dst_a * (1.0 - alpha)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 2] = ((fb * alpha + dst_b * dst_a * (1.0 - alpha)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 3] = (out_a * 255.0).min(255.0) as u8;
                                    }
                                }
                            }
                        }
                        SwashContent::Color => {
                            for py in 0..gh {
                                for px in 0..gw {
                                    let sx = (gx + px) as f32 + 0.5;
                                    let sy = (gy + py) as f32 + 0.5;
                                    let dx = (tf.sx * sx + tf.kx * sy + tf.tx) as i32;
                                    let dy = (tf.ky * sx + tf.sy * sy + tf.ty) as i32;
                                    if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 { continue; }
                                    let si = ((py * gw + px) * 4) as usize;
                                    if si + 3 >= image.data.len() { continue; }
                                    let sr = image.data[si] as f32 / 255.0;
                                    let sg = image.data[si + 1] as f32 / 255.0;
                                    let sb = image.data[si + 2] as f32 / 255.0;
                                    let sa = image.data[si + 3] as f32 / 255.0 * fa;
                                    if sa < 1.0 / 255.0 { continue; }
                                    let idx = ((dy as u32 * w + dx as u32) * 4) as usize;
                                    let pixels = self.pixmap.data_mut();
                                    if idx + 3 >= pixels.len() { continue; }
                                    let dst_r = pixels[idx] as f32 / 255.0;
                                    let dst_g = pixels[idx + 1] as f32 / 255.0;
                                    let dst_b = pixels[idx + 2] as f32 / 255.0;
                                    let dst_a = pixels[idx + 3] as f32 / 255.0;
                                    let out_a = sa + dst_a * (1.0 - sa);
                                    if out_a > 0.001 {
                                        pixels[idx]     = ((sr * sa + dst_r * dst_a * (1.0 - sa)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 1] = ((sg * sa + dst_g * dst_a * (1.0 - sa)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 2] = ((sb * sa + dst_b * dst_a * (1.0 - sa)) / out_a * 255.0).min(255.0) as u8;
                                        pixels[idx + 3] = (out_a * 255.0).min(255.0) as u8;
                                    }
                                }
                            }
                        }
                        _ => {} // SubpixelMask — skip
                    }
                }
            }
        }
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
        stroke.line_cap = self.line_cap;
        stroke.line_join = self.line_join;
        stroke.miter_limit = self.miter_limit;
        stroke
    }

    pub fn set_line_cap(&mut self, cap: &str) {
        self.line_cap = match cap {
            "round" => tiny_skia::LineCap::Round,
            "square" => tiny_skia::LineCap::Square,
            _ => tiny_skia::LineCap::Butt,
        };
    }

    pub fn set_line_join(&mut self, join: &str) {
        self.line_join = match join {
            "round" => tiny_skia::LineJoin::Round,
            "bevel" => tiny_skia::LineJoin::Bevel,
            _ => tiny_skia::LineJoin::Miter,
        };
    }

    pub fn set_miter_limit(&mut self, limit: f32) {
        self.miter_limit = limit;
    }
}

/// Manages all canvas buffers and gradient definitions.
pub struct CanvasManager {
    pub buffers: HashMap<CanvasId, CanvasBuffer>,
    pub gradients: HashMap<GradientId, GradientDef>,
    next_canvas_id: CanvasId,
    next_gradient_id: GradientId,
    /// Number of currently dirty canvas buffers (avoids O(n) scan).
    pub dirty_count: u32,
    /// Font system for Canvas 2D text rendering (shared across all canvases).
    pub font_system: glyphon::FontSystem,
    /// Glyph rasterization cache for Canvas 2D text.
    pub swash_cache: glyphon::cosmic_text::SwashCache,
}

impl CanvasManager {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            gradients: HashMap::new(),
            next_canvas_id: 1000, // Start high to avoid conflict with node IDs
            next_gradient_id: 1,
            dirty_count: 0,
            font_system: glyphon::FontSystem::new(),
            swash_cache: glyphon::cosmic_text::SwashCache::new(),
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

    pub fn get_image_data(&self, canvas_id: CanvasId, x: i32, y: i32, w: i32, h: i32) -> Option<(u32, u32, Vec<u8>)> {
        let canvas = self.buffers.get(&canvas_id)?;
        let width = w.max(0) as u32;
        let height = h.max(0) as u32;
        if width == 0 || height == 0 {
            return Some((0, 0, Vec::new()));
        }

        let mut out = vec![0u8; (width * height * 4) as usize];
        let src = canvas.pixels();
        let cw = canvas.width as i32;
        let ch = canvas.height as i32;

        for ry in 0..height as i32 {
            for rx in 0..width as i32 {
                let sx = x + rx;
                let sy = y + ry;
                let dst_idx = ((ry as u32 * width + rx as u32) * 4) as usize;
                if sx < 0 || sy < 0 || sx >= cw || sy >= ch {
                    continue;
                }
                let src_idx = ((sy as u32 * canvas.width + sx as u32) * 4) as usize;
                if src_idx + 3 < src.len() {
                    out[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
                }
            }
        }

        Some((width, height, out))
    }

    pub fn put_image_data(&mut self, canvas_id: CanvasId, x: i32, y: i32, w: i32, h: i32, data: &[u8]) {
        let Some(canvas) = self.buffers.get_mut(&canvas_id) else { return; };
        let width = w.max(0) as u32;
        let height = h.max(0) as u32;
        if width == 0 || height == 0 {
            return;
        }

        let expected = (width * height * 4) as usize;
        if data.len() < expected {
            return;
        }

        let cw = canvas.width as i32;
        let ch = canvas.height as i32;
        let dst = canvas.pixmap.data_mut();
        for ry in 0..height as i32 {
            for rx in 0..width as i32 {
                let dx = x + rx;
                let dy = y + ry;
                if dx < 0 || dy < 0 || dx >= cw || dy >= ch {
                    continue;
                }
                let src_idx = ((ry as u32 * width + rx as u32) * 4) as usize;
                let dst_idx = ((dy as u32 * canvas.width + dx as u32) * 4) as usize;
                if dst_idx + 3 < dst.len() {
                    dst[dst_idx..dst_idx + 4].copy_from_slice(&data[src_idx..src_idx + 4]);
                }
            }
        }
        canvas.dirty = true;
    }

    pub fn clip_current_path(&mut self, canvas_id: CanvasId) {
        if let Some(canvas) = self.buffers.get_mut(&canvas_id) {
            canvas.clip_current_path();
        }
    }

    /// Clear all gradient definitions and reset the ID counter.
    /// Call once per animation frame before new gradients are created
    /// to prevent unbounded growth when wallpapers re-create gradients
    /// every frame (e.g. `createRadialGradient()` in a rAF loop).
    pub fn gc_gradients(&mut self) {
        self.gradients.clear();
        self.next_gradient_id = 1;
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
        r,
        tiny_skia::Point::from_xy(cx, cy),
        0.0,
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

    // hsla(h, s%, l%, a) or hsl(h, s%, l%)
    if s.starts_with("hsla(") && s.ends_with(')') {
        let inner = &s[5..s.len()-1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 4 {
            let h = parts[0].trim().parse::<f32>().ok()?;
            let sp = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let a = parts[3].trim().parse::<f32>().ok()?;
            let (r, g, b) = hsl_to_rgb(h, sp, l);
            return tiny_skia::Color::from_rgba(r, g, b, a);
        }
    }
    if s.starts_with("hsl(") && s.ends_with(')') {
        let inner = &s[4..s.len()-1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let h = parts[0].trim().parse::<f32>().ok()?;
            let sp = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
            let (r, g, b) = hsl_to_rgb(h, sp, l);
            return tiny_skia::Color::from_rgba(r, g, b, 1.0);
        }
    }

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

    // Named colors (extended subset)
    match s {
        "black" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, 1.0),
        "white" => tiny_skia::Color::from_rgba(1.0, 1.0, 1.0, 1.0),
        "red" => tiny_skia::Color::from_rgba(1.0, 0.0, 0.0, 1.0),
        "green" => tiny_skia::Color::from_rgba(0.0, 0.502, 0.0, 1.0),
        "blue" => tiny_skia::Color::from_rgba(0.0, 0.0, 1.0, 1.0),
        "transparent" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        "yellow" => tiny_skia::Color::from_rgba(1.0, 1.0, 0.0, 1.0),
        "cyan" | "aqua" => tiny_skia::Color::from_rgba(0.0, 1.0, 1.0, 1.0),
        "magenta" | "fuchsia" => tiny_skia::Color::from_rgba(1.0, 0.0, 1.0, 1.0),
        "orange" => tiny_skia::Color::from_rgba(1.0, 0.647, 0.0, 1.0),
        "purple" => tiny_skia::Color::from_rgba(0.502, 0.0, 0.502, 1.0),
        "pink" => tiny_skia::Color::from_rgba(1.0, 0.753, 0.796, 1.0),
        "gray" | "grey" => tiny_skia::Color::from_rgba(0.502, 0.502, 0.502, 1.0),
        "silver" => tiny_skia::Color::from_rgba(0.753, 0.753, 0.753, 1.0),
        "maroon" => tiny_skia::Color::from_rgba(0.502, 0.0, 0.0, 1.0),
        "olive" => tiny_skia::Color::from_rgba(0.502, 0.502, 0.0, 1.0),
        "lime" => tiny_skia::Color::from_rgba(0.0, 1.0, 0.0, 1.0),
        "teal" => tiny_skia::Color::from_rgba(0.0, 0.502, 0.502, 1.0),
        "navy" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.502, 1.0),
        "coral" => tiny_skia::Color::from_rgba(1.0, 0.498, 0.314, 1.0),
        "salmon" => tiny_skia::Color::from_rgba(0.980, 0.502, 0.447, 1.0),
        "gold" => tiny_skia::Color::from_rgba(1.0, 0.843, 0.0, 1.0),
        "indigo" => tiny_skia::Color::from_rgba(0.294, 0.0, 0.510, 1.0),
        "violet" => tiny_skia::Color::from_rgba(0.933, 0.510, 0.933, 1.0),
        "brown" => tiny_skia::Color::from_rgba(0.647, 0.165, 0.165, 1.0),
        "tan" => tiny_skia::Color::from_rgba(0.824, 0.706, 0.549, 1.0),
        "beige" => tiny_skia::Color::from_rgba(0.961, 0.961, 0.863, 1.0),
        "ivory" => tiny_skia::Color::from_rgba(1.0, 1.0, 0.941, 1.0),
        "khaki" => tiny_skia::Color::from_rgba(0.941, 0.902, 0.549, 1.0),
        "crimson" => tiny_skia::Color::from_rgba(0.863, 0.078, 0.235, 1.0),
        "tomato" => tiny_skia::Color::from_rgba(1.0, 0.388, 0.278, 1.0),
        "turquoise" => tiny_skia::Color::from_rgba(0.251, 0.878, 0.816, 1.0),
        "skyblue" => tiny_skia::Color::from_rgba(0.529, 0.808, 0.922, 1.0),
        "steelblue" => tiny_skia::Color::from_rgba(0.275, 0.510, 0.706, 1.0),
        "slategray" | "slategrey" => tiny_skia::Color::from_rgba(0.439, 0.502, 0.565, 1.0),
        "darkgray" | "darkgrey" => tiny_skia::Color::from_rgba(0.663, 0.663, 0.663, 1.0),
        "lightgray" | "lightgrey" => tiny_skia::Color::from_rgba(0.827, 0.827, 0.827, 1.0),
        "dimgray" | "dimgrey" => tiny_skia::Color::from_rgba(0.412, 0.412, 0.412, 1.0),
        "whitesmoke" => tiny_skia::Color::from_rgba(0.961, 0.961, 0.961, 1.0),
        "snow" => tiny_skia::Color::from_rgba(1.0, 0.980, 0.980, 1.0),
        "linen" => tiny_skia::Color::from_rgba(0.980, 0.941, 0.902, 1.0),
        "dodgerblue" => tiny_skia::Color::from_rgba(0.118, 0.565, 1.0, 1.0),
        "royalblue" => tiny_skia::Color::from_rgba(0.255, 0.412, 0.882, 1.0),
        "cornflowerblue" => tiny_skia::Color::from_rgba(0.392, 0.584, 0.929, 1.0),
        "midnightblue" => tiny_skia::Color::from_rgba(0.098, 0.098, 0.439, 1.0),
        "darkblue" => tiny_skia::Color::from_rgba(0.0, 0.0, 0.545, 1.0),
        "darkred" => tiny_skia::Color::from_rgba(0.545, 0.0, 0.0, 1.0),
        "darkgreen" => tiny_skia::Color::from_rgba(0.0, 0.392, 0.0, 1.0),
        "darkcyan" => tiny_skia::Color::from_rgba(0.0, 0.545, 0.545, 1.0),
        "darkmagenta" => tiny_skia::Color::from_rgba(0.545, 0.0, 0.545, 1.0),
        "darkorange" => tiny_skia::Color::from_rgba(1.0, 0.549, 0.0, 1.0),
        "darkviolet" => tiny_skia::Color::from_rgba(0.580, 0.0, 0.827, 1.0),
        "deeppink" => tiny_skia::Color::from_rgba(1.0, 0.078, 0.576, 1.0),
        "deepskyblue" => tiny_skia::Color::from_rgba(0.0, 0.749, 1.0, 1.0),
        "firebrick" => tiny_skia::Color::from_rgba(0.698, 0.133, 0.133, 1.0),
        "forestgreen" => tiny_skia::Color::from_rgba(0.133, 0.545, 0.133, 1.0),
        "limegreen" => tiny_skia::Color::from_rgba(0.196, 0.804, 0.196, 1.0),
        "seagreen" => tiny_skia::Color::from_rgba(0.180, 0.545, 0.341, 1.0),
        "springgreen" => tiny_skia::Color::from_rgba(0.0, 1.0, 0.498, 1.0),
        "yellowgreen" => tiny_skia::Color::from_rgba(0.604, 0.804, 0.196, 1.0),
        "chartreuse" => tiny_skia::Color::from_rgba(0.498, 1.0, 0.0, 1.0),
        "hotpink" => tiny_skia::Color::from_rgba(1.0, 0.412, 0.706, 1.0),
        "lightblue" => tiny_skia::Color::from_rgba(0.678, 0.847, 0.902, 1.0),
        "lightgreen" => tiny_skia::Color::from_rgba(0.565, 0.933, 0.565, 1.0),
        "lightyellow" => tiny_skia::Color::from_rgba(1.0, 1.0, 0.878, 1.0),
        "lightcoral" => tiny_skia::Color::from_rgba(0.941, 0.502, 0.502, 1.0),
        "lightsalmon" => tiny_skia::Color::from_rgba(1.0, 0.627, 0.478, 1.0),
        "lightpink" => tiny_skia::Color::from_rgba(1.0, 0.714, 0.757, 1.0),
        "lightcyan" => tiny_skia::Color::from_rgba(0.878, 1.0, 1.0, 1.0),
        "lavender" => tiny_skia::Color::from_rgba(0.902, 0.902, 0.980, 1.0),
        "plum" => tiny_skia::Color::from_rgba(0.867, 0.627, 0.867, 1.0),
        "orchid" => tiny_skia::Color::from_rgba(0.855, 0.439, 0.839, 1.0),
        "peru" => tiny_skia::Color::from_rgba(0.804, 0.522, 0.247, 1.0),
        "sienna" => tiny_skia::Color::from_rgba(0.627, 0.322, 0.176, 1.0),
        "chocolate" => tiny_skia::Color::from_rgba(0.824, 0.412, 0.118, 1.0),
        "sandybrown" => tiny_skia::Color::from_rgba(0.957, 0.643, 0.376, 1.0),
        "wheat" => tiny_skia::Color::from_rgba(0.961, 0.871, 0.702, 1.0),
        "moccasin" => tiny_skia::Color::from_rgba(1.0, 0.894, 0.710, 1.0),
        "papayawhip" => tiny_skia::Color::from_rgba(1.0, 0.937, 0.835, 1.0),
        "peachpuff" => tiny_skia::Color::from_rgba(1.0, 0.855, 0.725, 1.0),
        "mintcream" => tiny_skia::Color::from_rgba(0.961, 1.0, 0.980, 1.0),
        "honeydew" => tiny_skia::Color::from_rgba(0.941, 1.0, 0.941, 1.0),
        "ghostwhite" => tiny_skia::Color::from_rgba(0.973, 0.973, 1.0, 1.0),
        "aliceblue" => tiny_skia::Color::from_rgba(0.941, 0.973, 1.0, 1.0),
        "azure" => tiny_skia::Color::from_rgba(0.941, 1.0, 1.0, 1.0),
        "seashell" => tiny_skia::Color::from_rgba(1.0, 0.961, 0.933, 1.0),
        "oldlace" => tiny_skia::Color::from_rgba(0.992, 0.961, 0.902, 1.0),
        "floralwhite" => tiny_skia::Color::from_rgba(1.0, 0.980, 0.941, 1.0),
        "antiquewhite" => tiny_skia::Color::from_rgba(0.980, 0.922, 0.843, 1.0),
        "blanchedalmond" => tiny_skia::Color::from_rgba(1.0, 0.922, 0.804, 1.0),
        "bisque" => tiny_skia::Color::from_rgba(1.0, 0.894, 0.769, 1.0),
        "navajowhite" => tiny_skia::Color::from_rgba(1.0, 0.871, 0.678, 1.0),
        "cornsilk" => tiny_skia::Color::from_rgba(1.0, 0.973, 0.863, 1.0),
        "lemonchiffon" => tiny_skia::Color::from_rgba(1.0, 0.980, 0.804, 1.0),
        "mistyrose" => tiny_skia::Color::from_rgba(1.0, 0.894, 0.882, 1.0),
        _ => None,
    }
}

/// Convert HSL to RGB. H is in degrees [0, 360), S and L are [0, 1].
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    ((r1 + m).clamp(0.0, 1.0), (g1 + m).clamp(0.0, 1.0), (b1 + m).clamp(0.0, 1.0))
}

