// openrender-runtime/src/layout/types.rs
//
// Layout computation types — intermediate results during layout.

use crate::cxrd::value::{Rect, EdgeInsets, Size};

/// Layout constraints passed from parent to child.
#[derive(Debug, Clone, Copy)]
pub struct LayoutConstraints {
    /// Maximum width available (from parent's content area).
    pub max_width: f32,
    /// Maximum height available.
    pub max_height: f32,
    /// Viewport dimensions (for vw/vh resolution).
    pub viewport_width: f32,
    pub viewport_height: f32,
    /// Root font size (for rem resolution).
    pub root_font_size: f32,
}

impl LayoutConstraints {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            max_width: viewport_width,
            max_height: viewport_height,
            viewport_width,
            viewport_height,
            root_font_size: 16.0,
        }
    }

    pub fn with_max(&self, max_width: f32, max_height: f32) -> Self {
        Self {
            max_width,
            max_height,
            ..*self
        }
    }
}

/// Result of laying out a single node.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutBox {
    /// The content area (inside padding and border).
    pub content: Rect,
    /// Resolved padding.
    pub padding: EdgeInsets,
    /// Resolved margin.
    pub margin: EdgeInsets,
    /// Resolved border widths.
    pub border: EdgeInsets,
    /// Total size including margin.
    pub outer_size: Size,
}

impl LayoutBox {
    /// The border box (content + padding + border).
    pub fn border_rect(&self) -> Rect {
        Rect {
            x: self.content.x - self.padding.left - self.border.left,
            y: self.content.y - self.padding.top - self.border.top,
            width: self.content.width + self.padding.horizontal() + self.border.horizontal(),
            height: self.content.height + self.padding.vertical() + self.border.vertical(),
        }
    }

    /// The margin box (content + padding + border + margin).
    pub fn margin_rect(&self) -> Rect {
        let br = self.border_rect();
        Rect {
            x: br.x - self.margin.left,
            y: br.y - self.margin.top,
            width: br.width + self.margin.horizontal(),
            height: br.height + self.margin.vertical(),
        }
    }
}
