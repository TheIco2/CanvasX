// canvasx-runtime/src/compiler/css/parsing.rs
//
// Shared CSS value parsing utilities used across all CSS versions.
// Contains parsers for dimensions, colors, gradients, grid tracks,
// calc() expressions, box-shadow, and CSS variable resolution.

use crate::cxrd::style::*;
use crate::cxrd::value::{Color, Dimension};
use std::collections::HashMap;

// ───────────────────────────── Dimension parsing ─────────────────────────────

/// Parse a CSS dimension value.
pub fn parse_dimension(value: &str) -> Dimension {
    let value = value.trim();
    if value == "auto" {
        return Dimension::Auto;
    }

    // Handle calc() expressions.
    if value.starts_with("calc(") {
        if let Some(inner) = value.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
            return parse_calc_dimension(inner);
        }
    }

    if let Some(v) = value.strip_suffix("px") {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Px(n);
        }
    }
    if let Some(v) = value.strip_suffix('%') {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Percent(n);
        }
    }
    if let Some(v) = value.strip_suffix("rem") {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Rem(n);
        }
    }
    if let Some(v) = value.strip_suffix("em") {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Em(n);
        }
    }
    if let Some(v) = value.strip_suffix("vw") {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Vw(n);
        }
    }
    if let Some(v) = value.strip_suffix("vh") {
        if let Ok(n) = v.trim().parse::<f32>() {
            return Dimension::Vh(n);
        }
    }
    // Bare number → px
    if let Ok(n) = value.parse::<f32>() {
        return Dimension::Px(n);
    }
    Dimension::Auto
}

/// Parse a `calc()` expression into a Dimension.
///
/// Handles common patterns:
///   - `calc(100% / N)` → Percent(100/N)
///   - `calc(100% - Npx)` → Percent(100) (approximate — drops px term)
///   - `calc(Npx + Mpx)` → Px(N+M)
///   - `calc(Npx * N)` → Px(result)
///
/// Falls back to evaluating as a pure numeric expression when possible.
pub(crate) fn parse_calc_dimension(expr: &str) -> Dimension {
    let expr = expr.trim();

    // Try to detect the "dominant" unit in the expression.
    if expr.contains('%') {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if let Some(pct_pos) = parts.iter().position(|p| p.ends_with('%')) {
            let pct_val = parts[pct_pos].trim_end_matches('%').parse::<f32>().unwrap_or(100.0);

            if pct_pos + 2 < parts.len() {
                let op = parts[pct_pos + 1];
                let rhs_str = parts[pct_pos + 2].trim_end_matches("px");
                let rhs = eval_calc_expr(rhs_str).unwrap_or(1.0);
                match op {
                    "/" => return Dimension::Percent(pct_val / rhs),
                    "*" => return Dimension::Percent(pct_val * rhs),
                    "+" | "-" => {
                        return Dimension::Percent(pct_val);
                    }
                    _ => {}
                }
            }
            return Dimension::Percent(pct_val);
        }
    }

    // Pure px or unitless arithmetic.
    let cleaned = expr.replace("px", "");
    if let Some(result) = eval_calc_expr(&cleaned) {
        return Dimension::Px(result);
    }

    Dimension::Auto
}

/// Evaluate a simple arithmetic expression (supports +, -, *, /).
/// Handles operator precedence: * and / before + and -.
pub(crate) fn eval_calc_expr(expr: &str) -> Option<f32> {
    let expr = expr.trim();

    let mut tokens: Vec<CalcToken> = Vec::new();
    let mut pos = 0;
    let bytes = expr.as_bytes();

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() { break; }

        if matches!(bytes[pos], b'+' | b'*' | b'/') {
            tokens.push(CalcToken::Op(bytes[pos] as char));
            pos += 1;
            continue;
        }

        if bytes[pos] == b'-' {
            let is_neg = tokens.is_empty() || matches!(tokens.last(), Some(CalcToken::Op(_)));
            if !is_neg {
                tokens.push(CalcToken::Op('-'));
                pos += 1;
                continue;
            }
        }

        let num_start = pos;
        if pos < bytes.len() && bytes[pos] == b'-' { pos += 1; }
        while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
            pos += 1;
        }
        if pos > num_start {
            if let Ok(n) = expr[num_start..pos].parse::<f32>() {
                tokens.push(CalcToken::Num(n));
                continue;
            }
        }

        pos += 1;
    }

    // First pass: * and /
    let mut simplified: Vec<CalcToken> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let CalcToken::Op(op) = &tokens[i] {
            if (*op == '*' || *op == '/') && !simplified.is_empty() {
                if let (Some(CalcToken::Num(lhs)), Some(CalcToken::Num(rhs))) =
                    (simplified.last().cloned(), tokens.get(i + 1))
                {
                    let result = if *op == '*' { lhs * rhs } else if rhs.abs() > f32::EPSILON { lhs / rhs } else { lhs };
                    *simplified.last_mut().unwrap() = CalcToken::Num(result);
                    i += 2;
                    continue;
                }
            }
        }
        simplified.push(tokens[i].clone());
        i += 1;
    }

    // Second pass: + and -
    let mut result = match simplified.first() {
        Some(CalcToken::Num(n)) => *n,
        _ => return None,
    };
    let mut j = 1;
    while j + 1 < simplified.len() {
        if let (CalcToken::Op(op), CalcToken::Num(rhs)) = (&simplified[j], &simplified[j + 1]) {
            match op {
                '+' => result += rhs,
                '-' => result -= rhs,
                _ => {}
            }
            j += 2;
        } else {
            j += 1;
        }
    }

    Some(result)
}

#[derive(Debug, Clone)]
enum CalcToken {
    Num(f32),
    Op(char),
}

// ───────────────────────────── Pixel parsing ─────────────────────────────────

/// Parse a px value.
pub fn parse_px(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.starts_with("calc(") {
        if let Dimension::Px(v) = parse_dimension(value) {
            return Some(v);
        }
        return None;
    }
    if let Some(v) = value.strip_suffix("px") {
        v.trim().parse::<f32>().ok()
    } else {
        value.parse::<f32>().ok()
    }
}

/// Parse `backdrop-filter` blur amount from values like `blur(8px)`.
pub fn parse_backdrop_blur(value: &str) -> Option<f32> {
    let v = value.trim();
    let start = v.find("blur(")?;
    let inner = &v[start + 5..];
    let end = inner.rfind(')')?;
    let expr = inner[..end].trim();
    parse_px(expr)
}

/// Parse transform scale from values like `scale(1.2)`.
pub fn parse_transform_scale(value: &str) -> Option<f32> {
    let v = value.trim();
    let start = v.find("scale(")?;
    let inner = &v[start + 6..];
    let end = inner.find(')')?;
    inner[..end].trim().parse::<f32>().ok()
}

// ───────────────────────────── Color parsing ─────────────────────────────────

/// Parse a CSS color value.
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();

    match value {
        "transparent" => return Some(Color::TRANSPARENT),
        "white" => return Some(Color::WHITE),
        "black" => return Some(Color::BLACK),
        "red" => return Some(Color::new(1.0, 0.0, 0.0, 1.0)),
        "green" => return Some(Color::new(0.0, 0.5, 0.0, 1.0)),
        "blue" => return Some(Color::new(0.0, 0.0, 1.0, 1.0)),
        "yellow" => return Some(Color::new(1.0, 1.0, 0.0, 1.0)),
        "orange" => return Some(Color::new(1.0, 0.647, 0.0, 1.0)),
        "gray" | "grey" => return Some(Color::new(0.5, 0.5, 0.5, 1.0)),
        _ => {}
    }

    // Hex
    if value.starts_with('#') {
        return Color::from_hex(value);
    }

    // rgba()
    if let Some(args) = value.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|s| parse_color_component(s)).collect();
        if nums.len() >= 4 {
            let r = if nums[0] > 1.0 { nums[0] / 255.0 } else { nums[0] };
            let g = if nums[1] > 1.0 { nums[1] / 255.0 } else { nums[1] };
            let b = if nums[2] > 1.0 { nums[2] / 255.0 } else { nums[2] };
            return Some(Color::new(r, g, b, nums[3]));
        }
    }

    // rgb()
    if let Some(args) = value.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|s| parse_color_component(s)).collect();
        if nums.len() >= 3 {
            let r = if nums[0] > 1.0 { nums[0] / 255.0 } else { nums[0] };
            let g = if nums[1] > 1.0 { nums[1] / 255.0 } else { nums[1] };
            let b = if nums[2] > 1.0 { nums[2] / 255.0 } else { nums[2] };
            return Some(Color::new(r, g, b, 1.0));
        }
    }

    // hsla()
    if let Some(args) = value.strip_prefix("hsla(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(',').map(str::trim).collect();
        if parts.len() >= 4 {
            let h = parse_hue_degrees(parts[0])?;
            let s = parse_percentage_unit(parts[1])?;
            let l = parse_percentage_unit(parts[2])?;
            let a = parse_color_component(parts[3])?.clamp(0.0, 1.0);
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(Color::new(r, g, b, a));
        }
    }

    // hsl()
    if let Some(args) = value.strip_prefix("hsl(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let h = parse_hue_degrees(parts[0])?;
            let s = parse_percentage_unit(parts[1])?;
            let l = parse_percentage_unit(parts[2])?;
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(Color::new(r, g, b, 1.0));
        }
    }

    None
}

fn parse_hue_degrees(raw: &str) -> Option<f32> {
    let s = raw.trim().trim_end_matches("deg").trim();
    s.parse::<f32>().ok()
}

fn parse_percentage_unit(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if let Some(v) = s.strip_suffix('%') {
        return v.trim().parse::<f32>().ok().map(|n| (n / 100.0).clamp(0.0, 1.0));
    }
    s.parse::<f32>().ok().map(|n| n.clamp(0.0, 1.0))
}

pub(crate) fn hsl_to_rgb(h_deg: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h_deg.rem_euclid(360.0) / 360.0;
    if s <= f32::EPSILON {
        return (l, l, l);
    }

    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;

    fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
        if t < 1.0 / 2.0 { return q; }
        if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
        p
    }

    (
        hue_to_channel(p, q, h + 1.0 / 3.0),
        hue_to_channel(p, q, h),
        hue_to_channel(p, q, h - 1.0 / 3.0),
    )
}

/// Parse a single color component (number, percentage, or calc()).
pub(crate) fn parse_color_component(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.ends_with('%') {
        return s.trim_end_matches('%').parse::<f32>().ok().map(|v| v / 100.0);
    }
    if let Ok(v) = s.parse::<f32>() {
        return Some(v);
    }
    if let Some(inner) = s.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
        return eval_calc_expr(inner);
    }
    if s.contains('*') || s.contains('/') || (s.contains('+') && !s.starts_with('+')) || (s.contains('-') && !s.starts_with('-') && s.len() > 1) {
        return eval_calc_expr(s);
    }
    None
}

// ───────────────────────────── Gradient parsing ──────────────────────────────

/// Split a string on commas, but respect nested parentheses.
pub fn split_comma_aware(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        if ch == '(' { depth += 1; current.push(ch); }
        else if ch == ')' { depth -= 1; current.push(ch); }
        else if ch == ',' && depth == 0 {
            tokens.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    let t = current.trim().to_string();
    if !t.is_empty() { tokens.push(t); }
    tokens
}

/// Parse a CSS `linear-gradient(angle, stop1, stop2, ...)` value.
pub fn parse_linear_gradient(value: &str) -> Option<Background> {
    let inner = value.strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;
    let parts = split_comma_aware(inner);
    if parts.is_empty() { return None; }

    let mut idx = 0;
    let mut angle_deg: f32 = 180.0;

    let first = parts[0].trim();
    if first.ends_with("deg") {
        if let Ok(a) = first.trim_end_matches("deg").trim().parse::<f32>() {
            angle_deg = a;
            idx = 1;
        }
    } else if first.starts_with("to ") {
        angle_deg = match first {
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
        idx = 1;
    }

    let stop_parts: Vec<&str> = parts[idx..].iter().map(|s| s.as_str()).collect();
    if stop_parts.is_empty() { return None; }

    let mut stops = Vec::new();
    let n = stop_parts.len();
    for (i, part) in stop_parts.iter().enumerate() {
        let part = part.trim();
        let (color_str, position) = if let Some(pct_idx) = part.rfind('%') {
            let before = &part[..pct_idx];
            if let Some(space_idx) = before.rfind(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
                let num_str = &part[space_idx+1..pct_idx];
                let pos = num_str.parse::<f32>().unwrap_or(i as f32 / (n - 1).max(1) as f32 * 100.0) / 100.0;
                (&part[..=space_idx], Some(pos))
            } else {
                (part, None)
            }
        } else {
            let words: Vec<&str> = part.rsplitn(2, char::is_whitespace).collect();
            if words.len() == 2 {
                if let Some(px) = parse_px(words[0]) {
                    (words[1], Some(px / 100.0))
                } else {
                    (part, None)
                }
            } else {
                (part, None)
            }
        };

        let position = position.unwrap_or_else(|| {
            if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 }
        });

        if let Some(color) = parse_color(color_str.trim()) {
            stops.push(GradientStop { color, position });
        }
    }

    if stops.is_empty() { return None; }

    Some(Background::LinearGradient { angle_deg, stops })
}

/// Parse a CSS `radial-gradient(stop1, stop2, ...)` value.
pub fn parse_radial_gradient(value: &str) -> Option<Background> {
    let inner = value.strip_prefix("radial-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;
    let parts = split_comma_aware(inner);
    if parts.is_empty() { return None; }

    let mut stops = Vec::new();
    let n = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let part = part.trim();
        if i == 0 && (part.starts_with("circle") || part.starts_with("ellipse") || part.starts_with("closest") || part.starts_with("farthest")) {
            continue;
        }

        let (color_str, position) = if let Some(pct_idx) = part.rfind('%') {
            let before = &part[..pct_idx];
            if let Some(space_idx) = before.rfind(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
                let num_str = &part[space_idx+1..pct_idx];
                let pos = num_str.parse::<f32>().unwrap_or(i as f32 / (n - 1).max(1) as f32 * 100.0) / 100.0;
                (&part[..=space_idx], Some(pos))
            } else {
                (part, None)
            }
        } else {
            (part, None)
        };

        let position = position.unwrap_or_else(|| {
            if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 }
        });

        if let Some(color) = parse_color(color_str.trim()) {
            stops.push(GradientStop { color, position });
        }
    }

    if stops.is_empty() { return None; }

    Some(Background::RadialGradient { stops })
}

// ───────────────────────────── Box shadow parsing ────────────────────────────

/// Parse a single `box-shadow` value: `offset-x offset-y blur-radius spread-radius color`
pub fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let value = value.trim();
    if value == "none" || value.is_empty() { return None; }

    let (color, remainder) = if let Some(rgba_start) = value.find("rgba(") {
        let end = value[rgba_start..].find(')').map(|e| rgba_start + e + 1)?;
        let c = parse_color(&value[rgba_start..end])?;
        let rest = format!("{} {}", &value[..rgba_start], &value[end..]);
        (c, rest)
    } else if let Some(rgb_start) = value.find("rgb(") {
        let end = value[rgb_start..].find(')').map(|e| rgb_start + e + 1)?;
        let c = parse_color(&value[rgb_start..end])?;
        let rest = format!("{} {}", &value[..rgb_start], &value[end..]);
        (c, rest)
    } else {
        let tokens: Vec<&str> = value.split_whitespace().collect();
        if tokens.len() >= 3 {
            if let Some(c) = tokens.last().and_then(|t| parse_color(t)) {
                let rest = tokens[..tokens.len()-1].join(" ");
                (c, rest)
            } else {
                (Color::new(0.0, 0.0, 0.0, 0.5), value.to_string())
            }
        } else {
            return None;
        }
    };

    let nums: Vec<f32> = remainder.split_whitespace()
        .filter_map(|t| parse_px(t))
        .collect();

    let offset_x = nums.first().copied().unwrap_or(0.0);
    let offset_y = nums.get(1).copied().unwrap_or(0.0);
    let blur_radius = nums.get(2).copied().unwrap_or(0.0);
    let spread_radius = nums.get(3).copied().unwrap_or(0.0);

    Some(BoxShadow { offset_x, offset_y, blur_radius, spread_radius, color, inset: false })
}

// ───────────────────────────── Shorthand parsing ─────────────────────────────

/// Parse a CSS shorthand with 1–4 values (margin, padding, etc.).
pub fn parse_shorthand_4(value: &str) -> (Dimension, Dimension, Dimension, Dimension) {
    let parts = split_css_function_aware(value);
    match parts.len() {
        1 => {
            let v = parse_dimension(&parts[0]);
            (v, v, v, v)
        }
        2 => {
            let tb = parse_dimension(&parts[0]);
            let lr = parse_dimension(&parts[1]);
            (tb, lr, tb, lr)
        }
        3 => {
            let t = parse_dimension(&parts[0]);
            let lr = parse_dimension(&parts[1]);
            let b = parse_dimension(&parts[2]);
            (t, lr, b, lr)
        }
        4 => {
            (parse_dimension(&parts[0]), parse_dimension(&parts[1]),
             parse_dimension(&parts[2]), parse_dimension(&parts[3]))
        }
        _ => (Dimension::Px(0.0), Dimension::Px(0.0), Dimension::Px(0.0), Dimension::Px(0.0)),
    }
}

/// Split a CSS value on whitespace, respecting parenthesized groups like `calc(...)`.
pub fn split_css_function_aware(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in value.chars() {
        if ch == '(' {
            depth += 1;
            current.push(ch);
        } else if ch == ')' {
            depth -= 1;
            current.push(ch);
        } else if ch.is_ascii_whitespace() && depth == 0 {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                tokens.push(trimmed);
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        tokens.push(trimmed);
    }
    tokens
}

// ───────────────────────────── CSS variable resolution ───────────────────────

/// Resolve CSS `var(--name)` references (internal).
pub(crate) fn resolve_var(value: &str, variables: &HashMap<String, String>) -> String {
    resolve_var_pub(value, variables)
}

/// Public version of resolve_var for use by the HTML compiler.
pub fn resolve_var_pub(value: &str, variables: &HashMap<String, String>) -> String {
    let mut result = value.to_string();
    while let Some(start) = result.find("var(") {
        let open = start + 3;
        let Some(close) = find_matching_paren(&result, open) else {
            break;
        };

        let inner = result[open + 1..close].trim();
        let (var_name, default) = split_top_level_comma(inner)
            .map(|(name, def)| (name.trim(), Some(def.trim().to_string())))
            .unwrap_or((inner, None));

        let replacement = variables
            .get(var_name)
            .cloned()
            .or(default)
            .unwrap_or_default();

        result = format!("{}{}{}", &result[..start], replacement, &result[close + 1..]);
    }
    result
}

fn find_matching_paren(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'(' {
        return None;
    }

    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open_idx) {
        match *b {
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_comma(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, b) in s.as_bytes().iter().enumerate() {
        match *b {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                return Some((&s[..i], &s[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

// ───────────────────────────── Grid parsing ──────────────────────────────────

/// Parse a `grid-template-columns` or `grid-template-rows` value into track sizes.
pub fn parse_grid_template(value: &str) -> Vec<GridTrackSize> {
    let value = value.trim();

    // Handle repeat()
    if value.starts_with("repeat(") {
        if let Some(inner) = value.strip_prefix("repeat(").and_then(|s| s.strip_suffix(')')) {
            if let Some((count_str, track_str)) = inner.split_once(',') {
                let count = count_str.trim().parse::<usize>().unwrap_or(1);
                let track = parse_single_grid_track(track_str.trim());
                return vec![track; count];
            }
        }
    }

    let tokens = split_css_function_aware(value);
    tokens.iter().map(|t| parse_single_grid_track(t)).collect()
}

/// Parse a single grid track size value.
pub(crate) fn parse_single_grid_track(value: &str) -> GridTrackSize {
    let value = value.trim();
    if value == "auto" {
        return GridTrackSize::Auto;
    }
    if value == "min-content" {
        return GridTrackSize::MinContent;
    }
    if value == "max-content" {
        return GridTrackSize::MaxContent;
    }
    if let Some(fr_str) = value.strip_suffix("fr") {
        if let Ok(v) = fr_str.trim().parse::<f32>() {
            return GridTrackSize::Fr(v);
        }
    }
    if let Some(pct_str) = value.strip_suffix('%') {
        if let Ok(v) = pct_str.trim().parse::<f32>() {
            return GridTrackSize::Percent(v);
        }
    }
    match parse_dimension(value) {
        Dimension::Px(v) => GridTrackSize::Px(v),
        Dimension::Percent(v) => GridTrackSize::Percent(v),
        _ => GridTrackSize::Auto,
    }
}

/// Parse a grid placement shorthand like "1 / -1", "1 / 3", "span 2", "auto".
pub fn parse_grid_placement(value: &str) -> (i32, i32) {
    let value = value.trim();
    if value == "auto" {
        return (0, 0);
    }
    if let Some((start_str, end_str)) = value.split_once('/') {
        let start = parse_grid_line(start_str.trim());
        let end = parse_grid_line(end_str.trim());
        (start, end)
    } else if value.starts_with("span") {
        let span = value.strip_prefix("span").and_then(|s| s.trim().parse::<i32>().ok()).unwrap_or(1);
        (0, span + 1000) // Encode span as > 1000 so layout can detect it
    } else {
        let line = parse_grid_line(value);
        (line, 0)
    }
}

/// Parse a single grid line number: "1", "-1", "auto", "span 2".
pub fn parse_grid_line(value: &str) -> i32 {
    let value = value.trim();
    if value == "auto" {
        return 0;
    }
    value.parse::<i32>().unwrap_or(0)
}
