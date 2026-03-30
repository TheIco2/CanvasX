// openrender-runtime/src/cxrd/value.rs
//
// Fundamental value types used across the CXRD format.

use serde::{Serialize, Deserialize};

/// A CSS-like dimension value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Dimension {
    /// Absolute pixels.
    Px(f32),
    /// Percentage of parent.
    Percent(f32),
    /// Relative to root font size.
    Rem(f32),
    /// Relative to element font size.
    Em(f32),
    /// Percentage of viewport width.
    Vw(f32),
    /// Percentage of viewport height.
    Vh(f32),
    /// Auto sizing.
    Auto,
}

impl Default for Dimension {
    fn default() -> Self {
        Dimension::Auto
    }
}

impl Dimension {
    /// Resolve this dimension to an absolute pixel value.
    pub fn resolve(&self, parent_size: f32, viewport_w: f32, viewport_h: f32, font_size: f32, root_font_size: f32) -> f32 {
        match *self {
            Dimension::Px(v) => v,
            Dimension::Percent(v) => parent_size * (v / 100.0),
            Dimension::Rem(v) => v * root_font_size,
            Dimension::Em(v) => v * font_size,
            Dimension::Vw(v) => viewport_w * (v / 100.0),
            Dimension::Vh(v) => viewport_h * (v / 100.0),
            Dimension::Auto => 0.0, // Caller must handle Auto specially
        }
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Dimension::Auto)
    }
}

/// RGBA color (0.0–1.0 per channel).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
                Some(Color { r, g, b, a: 1.0 })
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()? as f32 / 255.0;
                Some(Color { r, g, b, a })
            }
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()? as f32 / 255.0;
                Some(Color { r, g, b, a: 1.0 })
            }
            _ => None,
        }
    }

    pub fn to_array(&self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Linear interpolation between two colors.
    pub fn lerp(&self, other: &Color, t: f32) -> Color {
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::TRANSPARENT
    }
}

/// A rectangular region in pixel-space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width
            && py >= self.y && py <= self.y + self.height
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    pub fn to_array(&self) -> [f32; 4] {
        [self.x, self.y, self.width, self.height]
    }
}

/// Edge insets (padding, margin, border-width).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgeInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeInsets {
    pub fn uniform(v: f32) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Corner radii.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadii {
    pub fn uniform(v: f32) -> Self {
        Self { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }

    pub fn to_array(&self) -> [f32; 4] {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
    }
}

/// 2D size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
