// prism-runtime/src/compiler/css/parsing.rs
//
// Shared CSS value parsers — dimensions, colors, gradients, shadows, etc.
// Used by the property application logic in css.rs and by html.rs / v8_runtime.rs
// through the re-exports in mod.rs.

use crate::prd::style::*;
use crate::prd::value::{Color, Dimension};
use std::collections::HashMap;

// ═════════════════════════════════════════════════════════════════════════════
//  CSS VARIABLE RESOLUTION
// ═════════════════════════════════════════════════════════════════════════════

/// Resolve CSS custom property references: `var(--name)` → value.
/// Handles nested `var()` and fallback values `var(--name, fallback)`.
pub fn resolve_var(value: &str, variables: &HashMap<String, String>) -> String {
    let mut result = value.to_string();
    let mut iterations = 0;
    while result.contains("var(") && iterations < 20 {
        iterations += 1;
        if let Some(start) = result.find("var(") {
            let rest = &result[start + 4..];
            // Find the matching closing paren, respecting nesting.
            let mut depth = 1;
            let mut end = 0;
            for (i, ch) in rest.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner = &rest[..end];
            let (var_name, fallback) = if let Some(comma) = inner.find(',') {
                (inner[..comma].trim(), Some(inner[comma + 1..].trim()))
            } else {
                (inner.trim(), None)
            };
            let resolved = variables
                .get(var_name)
                .cloned()
                .or_else(|| fallback.map(|f| f.to_string()))
                .unwrap_or_default();
            result = format!("{}{}{}", &result[..start], resolved, &rest[end + 1..]);
        }
    }
    result
}

/// Public alias for `resolve_var` — re-exported by the CSS module for use in
/// html.rs and v8_runtime.rs.
pub fn resolve_var_pub(value: &str, variables: &HashMap<String, String>) -> String {
    resolve_var(value, variables)
}

// ═════════════════════════════════════════════════════════════════════════════
//  COLOR PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a CSS color value into a `Color`.
///
/// Supports:
///   - `transparent`
///   - Named colors (all 148 CSS Level 4 named colors)
///   - `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`
///   - `rgb(r, g, b)` / `rgb(r g b)`
///   - `rgba(r, g, b, a)` / `rgb(r g b / a)`
///   - `hsl(h, s%, l%)` / `hsl(h s% l%)`
///   - `hsla(h, s%, l%, a)` / `hsl(h s% l% / a)`
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    // Transparent
    if value.eq_ignore_ascii_case("transparent") {
        return Some(Color::TRANSPARENT);
    }

    // currentColor / inherit — not resolved here (caller decides)
    if value.eq_ignore_ascii_case("currentcolor") || value.eq_ignore_ascii_case("inherit") {
        return None;
    }

    // Hex colors
    if value.starts_with('#') {
        return parse_hex_color(value);
    }

    // rgb() / rgba()
    if value.starts_with("rgb") {
        return parse_rgb_color(value);
    }

    // hsl() / hsla()
    if value.starts_with("hsl") {
        return parse_hsl_color(value);
    }

    // Named colors
    named_color(value)
}

/// Parse `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`.
fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.trim_start_matches('#');
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(if hex.len() <= 4 { 1 } else { 2 })
        .filter_map(|i| {
            if hex.len() <= 4 {
                // Short form: double each digit
                let ch = &hex[i..i + 1];
                u8::from_str_radix(&format!("{}{}", ch, ch), 16).ok()
            } else {
                if i + 2 <= hex.len() {
                    u8::from_str_radix(&hex[i..i + 2], 16).ok()
                } else {
                    None
                }
            }
        })
        .collect();

    match bytes.len() {
        3 => Some(Color::new(
            bytes[0] as f32 / 255.0,
            bytes[1] as f32 / 255.0,
            bytes[2] as f32 / 255.0,
            1.0,
        )),
        4 => Some(Color::new(
            bytes[0] as f32 / 255.0,
            bytes[1] as f32 / 255.0,
            bytes[2] as f32 / 255.0,
            bytes[3] as f32 / 255.0,
        )),
        _ => None,
    }
}

/// Parse `rgb(...)` or `rgba(...)`.
fn parse_rgb_color(value: &str) -> Option<Color> {
    let inner = extract_function_args(value)?;
    let parts = split_color_args(&inner);

    if parts.len() < 3 {
        return None;
    }

    let r = parse_color_channel(&parts[0], 255.0)?;
    let g = parse_color_channel(&parts[1], 255.0)?;
    let b = parse_color_channel(&parts[2], 255.0)?;
    let a = if parts.len() >= 4 {
        parse_alpha_value(&parts[3])?
    } else {
        1.0
    };

    Some(Color::new(r, g, b, a))
}

/// Parse `hsl(...)` or `hsla(...)`.
fn parse_hsl_color(value: &str) -> Option<Color> {
    let inner = extract_function_args(value)?;
    let parts = split_color_args(&inner);

    if parts.len() < 3 {
        return None;
    }

    let h: f32 = parts[0]
        .trim()
        .trim_end_matches("deg")
        .trim_end_matches("turn")
        .parse()
        .ok()?;
    let h = if parts[0].contains("turn") { h * 360.0 } else { h };
    let s: f32 = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
    let l: f32 = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
    let a = if parts.len() >= 4 {
        parse_alpha_value(&parts[3])?
    } else {
        1.0
    };

    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(Color::new(r, g, b, a))
}

/// Extract the arguments from a CSS function: `fn(...)` → `...`.
fn extract_function_args(value: &str) -> Option<String> {
    let open = value.find('(')?;
    let close = value.rfind(')')?;
    if close > open + 1 {
        Some(value[open + 1..close].to_string())
    } else {
        None
    }
}

/// Split color function arguments, handling both comma-separated and space/slash
/// modern syntax: `r, g, b, a` or `r g b / a`.
fn split_color_args(inner: &str) -> Vec<String> {
    if inner.contains(',') {
        inner.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        // Modern syntax: `r g b` or `r g b / a`
        let parts: Vec<&str> = inner.split_whitespace().collect();
        let mut result = Vec::new();
        let mut saw_slash = false;
        for part in parts {
            if part == "/" {
                saw_slash = true;
                continue;
            }
            result.push(part.to_string());
        }
        // If there was a slash, the last part is alpha; it's already in the vec.
        let _ = saw_slash; // The alpha is just the last element after the slash
        result
    }
}

/// Parse a color channel value: number (0–255) or percentage (0%–100%).
fn parse_color_channel(s: &str, max: f32) -> Option<f32> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        Some((v / 100.0).clamp(0.0, 1.0))
    } else {
        let v: f32 = s.parse().ok()?;
        Some((v / max).clamp(0.0, 1.0))
    }
}

/// Parse an alpha value: number (0.0–1.0) or percentage (0%–100%).
fn parse_alpha_value(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        Some((v / 100.0).clamp(0.0, 1.0))
    } else {
        let v: f32 = s.parse().ok()?;
        Some(v.clamp(0.0, 1.0))
    }
}

/// Convert HSL to RGB (all values 0.0–1.0 except h which is 0–360).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);
    (r, g, b)
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// Lookup a CSS named color. Returns `Some(Color)` or `None`.
fn named_color(name: &str) -> Option<Color> {
    let (r, g, b) = match name.to_ascii_lowercase().as_str() {
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" | "grey" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 111, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    };
    Some(Color::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        1.0,
    ))
}

// ═════════════════════════════════════════════════════════════════════════════
//  DIMENSION PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a CSS dimension value into a `Dimension`.
///
/// Supports: `auto`, `px`, `%`, `em`, `rem`, `vh`, `vw`, `vmin`, `vmax`,
/// `ch`, `ex`, `fr`, `calc(...)`, bare numbers, `min-content`, `max-content`,
/// `fit-content`, `0`.
pub fn parse_dimension(value: &str) -> Dimension {
    let value = value.trim();
    match value {
        "auto" | "none" => Dimension::Auto,
        "0" => Dimension::Px(0.0),
        "min-content" | "max-content" | "fit-content" => Dimension::Auto,
        _ => {
            // Percentage
            if let Some(pct) = value.strip_suffix('%') {
                if let Ok(v) = pct.trim().parse::<f32>() {
                    return Dimension::Percent(v);
                }
            }
            // Pixel
            if let Some(px) = value.strip_suffix("px") {
                if let Ok(v) = px.trim().parse::<f32>() {
                    return Dimension::Px(v);
                }
            }
            // Em → approximate to px (assume 16px base)
            if let Some(em) = value.strip_suffix("em") {
                let em = em.trim_end_matches('r'); // rem → em
                if let Ok(v) = em.trim().parse::<f32>() {
                    return Dimension::Px(v * 16.0);
                }
            }
            // Viewport units — resolved against the actual viewport at
            // layout time (not the parent box).
            if let Some(vh) = value.strip_suffix("vh") {
                if let Ok(v) = vh.trim().parse::<f32>() {
                    return Dimension::Vh(v);
                }
            }
            if let Some(vw) = value.strip_suffix("vw") {
                if let Ok(v) = vw.trim().parse::<f32>() {
                    return Dimension::Vw(v);
                }
            }
            // vmin/vmax aren't first-class in the Dimension enum yet —
            // approximate with the smaller/larger of the two viewport
            // axes by mapping to Vw (close enough for typical 16:9).
            if let Some(vmin) = value.strip_suffix("vmin") {
                if let Ok(v) = vmin.trim().parse::<f32>() {
                    return Dimension::Vh(v);
                }
            }
            if let Some(vmax) = value.strip_suffix("vmax") {
                if let Ok(v) = vmax.trim().parse::<f32>() {
                    return Dimension::Vw(v);
                }
            }
            // Bare number → px
            if let Ok(v) = value.parse::<f32>() {
                return Dimension::Px(v);
            }
            // calc() — extract the first numeric value as an approximation
            if value.starts_with("calc(") {
                if let Some(inner) = extract_function_args(value) {
                    // Try to extract a simple value from the calc expression
                    for token in inner.split(|c: char| c == '+' || c == '-' || c == '*' || c == '/') {
                        let t = token.trim();
                        if !t.is_empty() {
                            let dim = parse_dimension(t);
                            if !matches!(dim, Dimension::Auto) {
                                return dim;
                            }
                        }
                    }
                }
            }
            Dimension::Auto
        }
    }
}

/// Parse a pixel value from a CSS length string. Returns `None` for
/// non-numeric values like `auto`.
pub fn parse_px(value: &str) -> Option<f32> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse::<f32>().ok();
    }
    if let Some(em) = value.strip_suffix("em") {
        let em = em.trim_end_matches('r');
        return em.trim().parse::<f32>().ok().map(|v| v * 16.0);
    }
    if let Some(pt) = value.strip_suffix("pt") {
        return pt.trim().parse::<f32>().ok().map(|v| v * 1.333);
    }
    // Keyword widths
    match value {
        "thin" => return Some(1.0),
        "medium" => return Some(3.0),
        "thick" => return Some(5.0),
        _ => {}
    }
    // Bare number
    value.parse::<f32>().ok()
}

// ═════════════════════════════════════════════════════════════════════════════
//  SHORTHAND PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a 1–4 value shorthand (margin, padding, border-width, etc.)
/// into `(top, right, bottom, left)` following CSS shorthand rules.
pub fn parse_shorthand_4(value: &str) -> (Dimension, Dimension, Dimension, Dimension) {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.len() {
        1 => {
            let v = parse_dimension(parts[0]);
            (v, v, v, v)
        }
        2 => {
            let tb = parse_dimension(parts[0]);
            let lr = parse_dimension(parts[1]);
            (tb, lr, tb, lr)
        }
        3 => {
            let t = parse_dimension(parts[0]);
            let lr = parse_dimension(parts[1]);
            let b = parse_dimension(parts[2]);
            (t, lr, b, lr)
        }
        4 => (
            parse_dimension(parts[0]),
            parse_dimension(parts[1]),
            parse_dimension(parts[2]),
            parse_dimension(parts[3]),
        ),
        _ => {
            let v = parse_dimension(value);
            (v, v, v, v)
        }
    }
}

/// Split a CSS value string into tokens, respecting parenthesized functions.
/// E.g. `"10px rgba(0,0,0,0.5) 20px"` → `["10px", "rgba(0,0,0,0.5)", "20px"]`.
pub fn split_css_function_aware(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in value.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ' ' | '\t' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    tokens.push(trimmed);
                }
                current.clear();
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    tokens.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        tokens.push(trimmed);
    }
    tokens
}

// ═════════════════════════════════════════════════════════════════════════════
//  GRADIENT PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a `linear-gradient(...)` value into a `Background::LinearGradient`.
pub fn parse_linear_gradient(value: &str) -> Option<Background> {
    let value = value.trim();
    if !value.starts_with("linear-gradient(") {
        return None;
    }
    let inner = extract_function_args(value)?;
    let parts = split_gradient_args(&inner);

    let mut angle_deg: f32 = 180.0; // default: top to bottom
    let mut color_parts = &parts[..];

    // First part might be an angle or direction keyword.
    if let Some(first) = parts.first() {
        let first_trimmed = first.trim();
        if first_trimmed.starts_with("to ") {
            angle_deg = match first_trimmed {
                "to top" => 0.0,
                "to right" => 90.0,
                "to bottom" => 180.0,
                "to left" => 270.0,
                "to top right" | "to right top" => 45.0,
                "to bottom right" | "to right bottom" => 135.0,
                "to bottom left" | "to left bottom" => 225.0,
                "to top left" | "to left top" => 315.0,
                _ => 180.0,
            };
            color_parts = &parts[1..];
        } else if let Some(angle) = parse_angle(first_trimmed) {
            angle_deg = angle;
            color_parts = &parts[1..];
        }
    }

    let stops = parse_gradient_stops(color_parts)?;
    Some(Background::LinearGradient { angle_deg, stops })
}

/// Parse a `radial-gradient(...)` value into a `Background::RadialGradient`.
pub fn parse_radial_gradient(value: &str) -> Option<Background> {
    let value = value.trim();
    if !value.starts_with("radial-gradient(") {
        return None;
    }
    let inner = extract_function_args(value)?;
    let parts = split_gradient_args(&inner);

    // Skip shape/extent keywords, just parse color stops.
    let mut color_start = 0;
    for (i, part) in parts.iter().enumerate() {
        let p = part.trim().to_ascii_lowercase();
        if p.contains("circle")
            || p.contains("ellipse")
            || p.contains("closest")
            || p.contains("farthest")
            || p.starts_with("at ")
        {
            color_start = i + 1;
        } else {
            break;
        }
    }

    let stops = parse_gradient_stops(&parts[color_start..])?;
    Some(Background::RadialGradient { stops })
}

/// Split gradient arguments by commas, respecting nested functions.
fn split_gradient_args(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        parts.push(remaining);
    }
    parts
}

/// Parse gradient color stops from a list of comma-separated parts.
fn parse_gradient_stops(parts: &[String]) -> Option<Vec<GradientStop>> {
    if parts.is_empty() {
        return None;
    }
    let mut stops = Vec::new();
    let count = parts.len();

    for (i, part) in parts.iter().enumerate() {
        let tokens: Vec<&str> = part.trim().splitn(2, ' ').collect();
        let color_str = tokens[0];
        let position = if tokens.len() > 1 {
            parse_stop_position(tokens[1])
        } else {
            // Default evenly spaced positions.
            if count == 1 {
                0.5
            } else {
                i as f32 / (count - 1) as f32
            }
        };

        if let Some(color) = parse_color(color_str) {
            stops.push(GradientStop { color, position });
        }
    }

    if stops.len() >= 2 {
        Some(stops)
    } else {
        None
    }
}

/// Parse a gradient stop position: `50%` → 0.5, `100px` → approximate.
fn parse_stop_position(s: &str) -> f32 {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0
    } else if let Some(px) = parse_px(s) {
        // Approximate: treat pixels as fraction of a 1000px gradient.
        (px / 1000.0).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Parse a CSS angle value: `90deg`, `0.25turn`, `100grad`, `1.57rad`.
pub fn parse_angle(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(deg) = s.strip_suffix("deg") {
        return deg.trim().parse::<f32>().ok();
    }
    if let Some(turn) = s.strip_suffix("turn") {
        return turn.trim().parse::<f32>().ok().map(|v| v * 360.0);
    }
    if let Some(grad) = s.strip_suffix("grad") {
        return grad.trim().parse::<f32>().ok().map(|v| v * 0.9);
    }
    if let Some(rad) = s.strip_suffix("rad") {
        return rad.trim().parse::<f32>().ok().map(|v| v.to_degrees());
    }
    // Bare number assumed degrees
    s.parse::<f32>().ok()
}

// ═════════════════════════════════════════════════════════════════════════════
//  BOX SHADOW PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a CSS `box-shadow` value.
///
/// Syntax: `[inset] <offset-x> <offset-y> [<blur-radius>] [<spread-radius>] <color>`
pub fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let value = value.trim();
    if value == "none" {
        return None;
    }

    let inset = value.contains("inset");
    let value = value.replace("inset", "");
    let tokens = split_css_function_aware(&value);

    let mut numbers = Vec::new();
    let mut color = Color::new(0.0, 0.0, 0.0, 0.25); // default shadow color

    for token in &tokens {
        if let Some(px) = parse_px(token) {
            numbers.push(px);
        } else if let Some(c) = parse_color(token) {
            color = c;
        }
    }

    let offset_x = *numbers.first().unwrap_or(&0.0);
    let offset_y = *numbers.get(1).unwrap_or(&0.0);
    let blur_radius = *numbers.get(2).unwrap_or(&0.0);
    let spread_radius = *numbers.get(3).unwrap_or(&0.0);

    Some(BoxShadow {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color,
        inset,
    })
}

// ═════════════════════════════════════════════════════════════════════════════
//  FILTER / TRANSFORM PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse `backdrop-filter: blur(Xpx)` → blur radius in px.
pub fn parse_backdrop_blur(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(start) = value.find("blur(") {
        let rest = &value[start + 5..];
        if let Some(end) = rest.find(')') {
            return parse_px(&rest[..end]);
        }
    }
    None
}

/// Parse `transform: scale(X)` → scale factor.
/// Also handles `scaleX(X)`, `scaleY(X)`, `scale(X, Y)`.
pub fn parse_transform_scale(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(start) = value.find("scale(") {
        let rest = &value[start + 6..];
        if let Some(end) = rest.find(')') {
            let inner = &rest[..end];
            // scale(x, y) — return x
            let x_str = inner.split(',').next()?.trim();
            return x_str.parse::<f32>().ok();
        }
    }
    if let Some(start) = value.find("scaleX(") {
        let rest = &value[start + 7..];
        if let Some(end) = rest.find(')') {
            return rest[..end].trim().parse::<f32>().ok();
        }
    }
    if let Some(start) = value.find("scaleY(") {
        let rest = &value[start + 7..];
        if let Some(end) = rest.find(')') {
            return rest[..end].trim().parse::<f32>().ok();
        }
    }
    None
}

// ═════════════════════════════════════════════════════════════════════════════
//  GRID PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a `grid-template-columns` / `grid-template-rows` value.
pub fn parse_grid_template(value: &str) -> Vec<GridTrackSize> {
    let value = value.trim();
    if value == "none" || value == "auto" {
        return Vec::new();
    }

    let mut tracks = Vec::new();

    // Handle repeat(N, track)
    if value.starts_with("repeat(") {
        if let Some(inner) = extract_function_args(value) {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() == 2 {
                let count: usize = parts[0].trim().parse().unwrap_or(1);
                let track = parse_single_track(parts[1].trim());
                for _ in 0..count {
                    tracks.push(track.clone());
                }
            }
        }
        return tracks;
    }

    for token in split_css_function_aware(value) {
        // Handle repeat() inside a multi-track definition
        if token.starts_with("repeat(") {
            if let Some(inner) = extract_function_args(&token) {
                let parts: Vec<&str> = inner.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let count: usize = parts[0].trim().parse().unwrap_or(1);
                    let track = parse_single_track(parts[1].trim());
                    for _ in 0..count {
                        tracks.push(track.clone());
                    }
                }
            }
        } else {
            tracks.push(parse_single_track(&token));
        }
    }
    tracks
}

/// Parse a single grid track size token.
fn parse_single_track(s: &str) -> GridTrackSize {
    let s = s.trim();
    if s == "auto" {
        return GridTrackSize::Auto;
    }
    if s == "min-content" {
        return GridTrackSize::MinContent;
    }
    if s == "max-content" {
        return GridTrackSize::MaxContent;
    }
    if let Some(fr) = s.strip_suffix("fr") {
        if let Ok(v) = fr.trim().parse::<f32>() {
            return GridTrackSize::Fr(v);
        }
    }
    if let Some(pct) = s.strip_suffix('%') {
        if let Ok(v) = pct.trim().parse::<f32>() {
            return GridTrackSize::Percent(v);
        }
    }
    if let Some(px) = parse_px(s) {
        return GridTrackSize::Px(px);
    }
    GridTrackSize::Auto
}

/// Parse `grid-column` / `grid-row` shorthand: `start / end`.
pub fn parse_grid_placement(value: &str) -> (i32, i32) {
    if let Some((start_str, end_str)) = value.split_once('/') {
        let start = parse_grid_line(start_str.trim());
        let end = parse_grid_line(end_str.trim());
        (start, end)
    } else {
        let start = parse_grid_line(value.trim());
        (start, 0)
    }
}

/// Parse a grid line value: number, `span N`, `auto`, or `-1`.
pub fn parse_grid_line(value: &str) -> i32 {
    let value = value.trim();
    if value == "auto" {
        return 0;
    }
    if let Some(span_str) = value.strip_prefix("span") {
        let n: i32 = span_str.trim().parse().unwrap_or(1);
        return -n; // negative = span
    }
    value.parse::<i32>().unwrap_or(0)
}

// ═════════════════════════════════════════════════════════════════════════════
//  TRANSITION PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a CSS `transition` shorthand into a list of `TransitionDef`.
///
/// Syntax: `<property> <duration> [<timing-function>] [<delay>]`
/// Multiple transitions separated by commas.
pub fn parse_transition(value: &str) -> Vec<TransitionDef> {
    let mut defs = Vec::new();
    let parts = split_gradient_args(value); // reuse comma-aware splitter

    for part in &parts {
        let tokens: Vec<&str> = part.trim().split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        let property = tokens.first().map(|s| s.to_string()).unwrap_or_default();
        let duration_ms = tokens.get(1).and_then(|s| parse_time_ms(s)).unwrap_or(0.0);
        let mut easing = EasingFunction::Ease;
        let mut delay_ms: f32 = 0.0;

        // Remaining tokens: timing function and/or delay
        for token in tokens.iter().skip(2) {
            if let Some(e) = parse_easing(token) {
                easing = e;
            } else if let Some(t) = parse_time_ms(token) {
                delay_ms = t;
            }
        }

        if !property.is_empty() && property != "none" {
            defs.push(TransitionDef {
                property,
                duration_ms,
                delay_ms,
                easing,
            });
        }
    }
    defs
}

/// Parse a CSS time value to milliseconds: `200ms`, `0.2s`, `1s`.
pub fn parse_time_ms(value: &str) -> Option<f32> {
    let value = value.trim();
    if let Some(ms) = value.strip_suffix("ms") {
        return ms.trim().parse::<f32>().ok();
    }
    if let Some(s) = value.strip_suffix('s') {
        return s.trim().parse::<f32>().ok().map(|v| v * 1000.0);
    }
    None
}

/// Parse a CSS easing function name into an `EasingFunction`.
pub fn parse_easing(value: &str) -> Option<EasingFunction> {
    let value = value.trim();
    match value {
        "linear" => Some(EasingFunction::Linear),
        "ease" => Some(EasingFunction::Ease),
        "ease-in" => Some(EasingFunction::EaseIn),
        "ease-out" => Some(EasingFunction::EaseOut),
        "ease-in-out" => Some(EasingFunction::EaseInOut),
        _ if value.starts_with("cubic-bezier(") => {
            let inner = extract_function_args(value)?;
            let nums: Vec<f32> = inner.split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            if nums.len() == 4 {
                Some(EasingFunction::CubicBezier(nums[0], nums[1], nums[2], nums[3]))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  BORDER SHORTHAND PARSING
// ═════════════════════════════════════════════════════════════════════════════

/// Parse a border shorthand value: `<width> <style> <color>`.
/// Returns `(width, style, color)` — any component may be None.
pub fn parse_border_shorthand(value: &str) -> (Option<f32>, Option<BorderStyle>, Option<Color>) {
    let tokens = split_css_function_aware(value);
    let mut width = None;
    let mut style = None;
    let mut color = None;

    for token in &tokens {
        // Try border style keyword first
        if let Some(s) = parse_border_style_keyword(token) {
            style = Some(s);
        } else if let Some(c) = parse_color(token) {
            color = Some(c);
        } else if let Some(w) = parse_px(token) {
            width = Some(w);
        }
    }

    (width, style, color)
}

/// Parse a border-style keyword.
pub fn parse_border_style_keyword(value: &str) -> Option<BorderStyle> {
    match value.trim() {
        "none" => Some(BorderStyle::None),
        "solid" => Some(BorderStyle::Solid),
        "dashed" => Some(BorderStyle::Dashed),
        "dotted" => Some(BorderStyle::Dotted),
        "double" => Some(BorderStyle::Double),
        "groove" => Some(BorderStyle::Groove),
        "ridge" => Some(BorderStyle::Ridge),
        "inset" => Some(BorderStyle::Inset),
        "outset" => Some(BorderStyle::Outset),
        "hidden" => Some(BorderStyle::Hidden),
        _ => None,
    }
}

/// Parse a border-style keyword, returning a default of Solid for unknowns.
pub fn parse_border_style(value: &str) -> BorderStyle {
    parse_border_style_keyword(value).unwrap_or(BorderStyle::Solid)
}

/// Parse an overflow value, returning the most restrictive of two axes.
pub fn most_restrictive_overflow(a: Overflow, b: Overflow) -> Overflow {
    match (a, b) {
        (Overflow::Scroll, _) | (_, Overflow::Scroll) => Overflow::Scroll,
        (Overflow::Hidden, _) | (_, Overflow::Hidden) => Overflow::Hidden,
        _ => Overflow::Visible,
    }
}

/// Parse a CSS blend mode keyword.
pub fn parse_blend_mode(value: &str) -> BlendMode {
    match value.trim() {
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// Parse a background-clip/background-origin keyword.
pub fn parse_background_box(value: &str) -> BackgroundBox {
    match value.trim() {
        "padding-box" => BackgroundBox::PaddingBox,
        "content-box" => BackgroundBox::ContentBox,
        _ => BackgroundBox::BorderBox,
    }
}

/// Parse a full `transform` value into a list of CssTransform functions.
pub fn parse_transforms(value: &str) -> Vec<CssTransform> {
    let mut transforms = Vec::new();
    let mut remaining = value.trim();

    while !remaining.is_empty() {
        if let Some(start) = remaining.find('(') {
            let func_name = remaining[..start].trim();
            let after_paren = &remaining[start + 1..];
            // Find matching close paren
            let mut depth = 1;
            let mut end = 0;
            for (i, ch) in after_paren.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let args = &after_paren[..end];
            remaining = after_paren[end + 1..].trim();

            match func_name {
                "translate" | "translateX" | "translateY" => {
                    let parts: Vec<&str> = args.split(',').collect();
                    let tx = if func_name == "translateY" { 0.0 } else { parse_px(parts[0].trim()).unwrap_or(0.0) };
                    let ty = if func_name == "translateX" {
                        0.0
                    } else if func_name == "translateY" {
                        parse_px(parts[0].trim()).unwrap_or(0.0)
                    } else {
                        parts.get(1).and_then(|v| parse_px(v.trim())).unwrap_or(0.0)
                    };
                    transforms.push(CssTransform::Translate(tx, ty));
                }
                "scale" | "scaleX" | "scaleY" => {
                    let parts: Vec<&str> = args.split(',').collect();
                    let sx = if func_name == "scaleY" { 1.0 } else { parts[0].trim().parse::<f32>().unwrap_or(1.0) };
                    let sy = if func_name == "scaleX" {
                        1.0
                    } else if func_name == "scaleY" {
                        parts[0].trim().parse::<f32>().unwrap_or(1.0)
                    } else {
                        parts.get(1).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(sx)
                    };
                    transforms.push(CssTransform::Scale(sx, sy));
                }
                "rotate" => {
                    if let Some(deg) = parse_angle(args.trim()) {
                        transforms.push(CssTransform::Rotate(deg));
                    }
                }
                "skewX" | "skew" => {
                    if let Some(deg) = parse_angle(args.split(',').next().unwrap_or("").trim()) {
                        transforms.push(CssTransform::SkewX(deg));
                    }
                }
                "skewY" => {
                    if let Some(deg) = parse_angle(args.trim()) {
                        transforms.push(CssTransform::SkewY(deg));
                    }
                }
                _ => {} // Unrecognized transform function
            }
        } else {
            break;
        }
    }
    transforms
}

/// Parse a CSS `filter` value into a list of CssFilter functions.
pub fn parse_filters(value: &str) -> Vec<CssFilter> {
    let mut filters = Vec::new();
    let mut remaining = value.trim();

    while !remaining.is_empty() {
        if let Some(start) = remaining.find('(') {
            let func_name = remaining[..start].trim();
            let after_paren = &remaining[start + 1..];
            let mut depth = 1;
            let mut end = 0;
            for (i, ch) in after_paren.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let arg = after_paren[..end].trim();
            remaining = after_paren[end + 1..].trim();

            let parse_percent_or_number = |s: &str| -> f32 {
                if let Some(p) = s.strip_suffix('%') {
                    p.parse::<f32>().unwrap_or(100.0) / 100.0
                } else {
                    s.parse::<f32>().unwrap_or(1.0)
                }
            };

            match func_name {
                "blur" => {
                    if let Some(v) = parse_px(arg) {
                        filters.push(CssFilter::Blur(v));
                    }
                }
                "brightness" => filters.push(CssFilter::Brightness(parse_percent_or_number(arg))),
                "contrast" => filters.push(CssFilter::Contrast(parse_percent_or_number(arg))),
                "grayscale" => filters.push(CssFilter::Grayscale(parse_percent_or_number(arg))),
                "hue-rotate" => {
                    if let Some(deg) = parse_angle(arg) {
                        filters.push(CssFilter::HueRotate(deg));
                    }
                }
                "invert" => filters.push(CssFilter::Invert(parse_percent_or_number(arg))),
                "opacity" => filters.push(CssFilter::Opacity(parse_percent_or_number(arg))),
                "saturate" => filters.push(CssFilter::Saturate(parse_percent_or_number(arg))),
                "sepia" => filters.push(CssFilter::Sepia(parse_percent_or_number(arg))),
                _ => {}
            }
        } else {
            break;
        }
    }
    filters
}

/// Parse a CSS `font` shorthand value.
pub fn parse_font_shorthand(value: &str, style: &mut ComputedStyle) {
    // CSS font shorthand: [style] [variant] [weight] [stretch] size[/line-height] family[, family]*
    // We parse from right to left: family is everything after the size/line-height token.
    let tokens: Vec<&str> = value.split_whitespace().collect();
    if tokens.is_empty() {
        return;
    }

    // Find the size token — it's the first token that looks like a dimension
    let mut size_idx = None;
    for (i, t) in tokens.iter().enumerate() {
        let base = t.split('/').next().unwrap_or(t);
        if parse_px(base).is_some() || base.ends_with("em") || base.ends_with("rem") || base.ends_with('%') {
            size_idx = Some(i);
            break;
        }
    }

    if let Some(si) = size_idx {
        // Everything before size_idx is style/variant/weight/stretch
        for t in &tokens[..si] {
            match *t {
                "italic" => style.font_style = FontStyle::Italic,
                "oblique" => style.font_style = FontStyle::Oblique,
                "bold" => style.font_weight = FontWeight(700),
                "normal" => {} // default
                "lighter" => style.font_weight = FontWeight(300),
                "bolder" => style.font_weight = FontWeight(600),
                "small-caps" => style.font_variant = Some("small-caps".to_string()),
                _ => {
                    if let Ok(w) = t.parse::<u16>() {
                        style.font_weight = FontWeight(w);
                    }
                }
            }
        }

        // Parse size/line-height
        let size_token = tokens[si];
        if let Some(slash) = size_token.find('/') {
            let size_str = &size_token[..slash];
            let lh_str = &size_token[slash + 1..];
            if let Some(s) = parse_px(size_str) {
                style.font_size = s;
            }
            if let Ok(lh) = lh_str.parse::<f32>() {
                style.line_height = lh;
            } else if let Some(lh) = parse_px(lh_str) {
                style.line_height = lh / style.font_size.max(1.0);
            }
        } else if let Some(s) = parse_px(size_token) {
            style.font_size = s;
        }

        // Everything after size is the font family
        if si + 1 < tokens.len() {
            let family_str = tokens[si + 1..].join(" ");
            let first = family_str.split(',').next().unwrap_or(&family_str);
            let family = first.trim().trim_matches(|c: char| c == '"' || c == '\'');
            style.font_family = family.to_string();
        }
    } else {
        // No size found — treat entire value as family
        let first = value.split(',').next().unwrap_or(value);
        let family = first.trim().trim_matches(|c: char| c == '"' || c == '\'');
        style.font_family = family.to_string();
    }
}

/// Parse a CSS `animation` shorthand into an AnimationDef.
pub fn parse_animation_shorthand(value: &str) -> Option<AnimationDef> {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    if tokens.is_empty() || value == "none" {
        return None;
    }

    let mut name = String::new();
    let mut duration_ms = 0.0;
    let mut delay_ms = 0.0;
    let mut easing = EasingFunction::Ease;
    let mut iteration_count = AnimationIterationCount::Number(1.0);
    let mut direction = AnimationDirection::Normal;
    let mut fill_mode = AnimationFillMode::None;
    let mut play_state = AnimationPlayState::Running;
    let mut found_duration = false;

    for token in &tokens {
        if let Some(t) = parse_time_ms(token) {
            if !found_duration {
                duration_ms = t;
                found_duration = true;
            } else {
                delay_ms = t;
            }
        } else if let Some(e) = parse_easing(token) {
            easing = e;
        } else {
            match *token {
                "infinite" => iteration_count = AnimationIterationCount::Infinite,
                "reverse" => direction = AnimationDirection::Reverse,
                "alternate" => direction = AnimationDirection::Alternate,
                "alternate-reverse" => direction = AnimationDirection::AlternateReverse,
                "forwards" => fill_mode = AnimationFillMode::Forwards,
                "backwards" => fill_mode = AnimationFillMode::Backwards,
                "both" => fill_mode = AnimationFillMode::Both,
                "paused" => play_state = AnimationPlayState::Paused,
                "running" => play_state = AnimationPlayState::Running,
                "normal" => {} // default direction
                "none" => {}
                _ => {
                    if name.is_empty() {
                        name = token.to_string();
                    }
                }
            }
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(AnimationDef {
        name,
        duration_ms,
        delay_ms,
        easing,
        iteration_count,
        direction,
        fill_mode,
        play_state,
    })
}

/// Create a default AnimationDef with the given name.
pub fn default_animation_def(name: &str) -> AnimationDef {
    AnimationDef {
        name: name.to_string(),
        duration_ms: 0.0,
        delay_ms: 0.0,
        easing: EasingFunction::Ease,
        iteration_count: AnimationIterationCount::Number(1.0),
        direction: AnimationDirection::Normal,
        fill_mode: AnimationFillMode::None,
        play_state: AnimationPlayState::Running,
    }
}

/// Parse an easing function, wrapping the inner parse_easing.
pub fn parse_easing_function(value: &str) -> EasingFunction {
    parse_easing(value).unwrap_or(EasingFunction::Ease)
}

