// canvasx-runtime/src/compiler/css.rs
//
// CSS subset parser for the CanvasX Runtime.
// Parses a restricted set of CSS properties into ComputedStyle values.
//
// Supported selectors: tag, .class, #id, descendant combinator.
// Supported properties: see the match arms in `apply_property`.

use crate::cxrd::style::*;
use crate::cxrd::value::{Color, Dimension};
use std::collections::HashMap;

/// A parsed CSS rule.
#[derive(Debug, Clone)]
pub struct CssRule {
    /// Selector string.
    pub selector: String,
    /// Parsed complex selector (list of compound selectors for descendant matching).
    pub compound_selectors: Vec<CompoundSelector>,
    /// Property declarations.
    pub declarations: Vec<(String, String)>,
}

/// A compound CSS selector part (matches a single element).
/// Multiple conditions must ALL match the same element.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub is_universal: bool,
}

impl CompoundSelector {
    /// Check if this compound selector matches a node.
    pub fn matches_node(&self, tag: Option<&str>, classes: &[String], html_id: Option<&str>) -> bool {
        if self.is_universal {
            return true;
        }
        if let Some(ref t) = self.tag {
            if tag != Some(t.as_str()) {
                return false;
            }
        }
        for cls in &self.classes {
            if !classes.contains(cls) {
                return false;
            }
        }
        if let Some(ref id) = self.id {
            if html_id != Some(id.as_str()) {
                return false;
            }
        }
        // Must have at least one condition
        self.tag.is_some() || !self.classes.is_empty() || self.id.is_some()
    }
}

/// A part of a CSS selector (kept for backward compatibility).
#[derive(Debug, Clone, PartialEq)]
pub enum SelectorPart {
    Tag(String),
    Class(String),
    Id(String),
    Universal,
}

/// Parse CSS source into a list of rules.
pub fn parse_css(source: &str) -> Vec<CssRule> {
    let mut rules = Vec::new();
    let mut pos = 0;
    // Strip comments
    let source = strip_comments(source);
    let bytes = source.as_bytes();

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Skip @rules (we don't support most of them)
        if bytes[pos] == b'@' {
            // Find the matching { } block or semicolon
            if let Some(block_start) = source[pos..].find('{') {
                let abs_start = pos + block_start;
                if let Some(block_end) = find_matching_brace(&source, abs_start) {
                    // Check for @keyframes — we handle those
                    let at_rule = &source[pos..abs_start].trim();
                    if at_rule.starts_with("@keyframes") {
                        // TODO: Parse keyframes into AnimationDef
                    }
                    pos = block_end + 1;
                    continue;
                }
            }
            // Skip to next semicolon
            if let Some(semi) = source[pos..].find(';') {
                pos += semi + 1;
            } else {
                break;
            }
            continue;
        }

        // Parse selector
        let selector_start = pos;
        while pos < bytes.len() && bytes[pos] != b'{' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let selector = source[selector_start..pos].trim().to_string();
        pos += 1; // skip '{'

        // Parse declarations until '}'
        let decl_start = pos;
        let mut depth = 1;
        while pos < bytes.len() && depth > 0 {
            if bytes[pos] == b'{' { depth += 1; }
            if bytes[pos] == b'}' { depth -= 1; }
            if depth > 0 { pos += 1; }
        }
        let decl_block = &source[decl_start..pos];
        pos += 1; // skip '}'

        let declarations = parse_declarations(decl_block);

        // Handle comma-separated selectors: "html, body" → two rules.
        let selector_group: Vec<&str> = selector.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        for sel in selector_group {
            let compound_selectors = parse_compound_selector(sel);
            rules.push(CssRule {
                selector: sel.to_string(),
                compound_selectors,
                declarations: declarations.clone(),
            });
        }
    }

    rules
}

/// Parse a CSS selector string into a chain of compound selectors.
/// Each compound matches a single element; the chain represents descendant combinators.
///
/// Examples:
///   ".pnl h2"    → [Compound{classes:["pnl"]}, Compound{tag:"h2"}]
///   ".cap.on"    → [Compound{classes:["cap","on"]}]
///   "#mediaPanel h2" → [Compound{id:"mediaPanel"}, Compound{tag:"h2"}]
///   "*"          → [Compound{universal:true}]
fn parse_compound_selector(selector: &str) -> Vec<CompoundSelector> {
    let mut chain = Vec::new();
    for token in selector.split_whitespace() {
        // Skip pseudo-elements: we don't render them, so the entire selector
        // is unmatchable. Return empty chain (compound_selector_matches returns false).
        let token = if token.contains("::") {
            return Vec::new();
        } else if let Some(idx) = token.find(':') {
            // :root, :hover, etc. — keep the part before for matching
            let before = &token[..idx];
            if before.is_empty() {
                // Pure pseudo like ":root"
                chain.push(CompoundSelector {
                    tag: Some(token.to_string()),
                    classes: Vec::new(),
                    id: None,
                    is_universal: false,
                });
                continue;
            }
            before
        } else {
            token
        };

        if token.is_empty() {
            continue;
        }

        if token == "*" {
            chain.push(CompoundSelector {
                tag: None, classes: Vec::new(), id: None, is_universal: true,
            });
            continue;
        }

        // Parse compound: "div.class1.class2#id" etc.
        let mut compound = CompoundSelector {
            tag: None, classes: Vec::new(), id: None, is_universal: false,
        };

        let mut pos = 0;
        let chars: Vec<char> = token.chars().collect();
        while pos < chars.len() {
            match chars[pos] {
                '.' => {
                    pos += 1;
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.classes.push(chars[start..pos].iter().collect());
                    }
                }
                '#' => {
                    pos += 1;
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.id = Some(chars[start..pos].iter().collect());
                    }
                }
                _ => {
                    // Tag name
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.tag = Some(chars[start..pos].iter().collect::<String>().to_lowercase());
                    }
                }
            }
        }

        chain.push(compound);
    }
    chain
}

/// Parse a CSS selector into parts (legacy, kept for backward compat).
#[allow(dead_code)]
fn parse_selector(selector: &str) -> Vec<SelectorPart> {
    let mut parts = Vec::new();
    for token in selector.split_whitespace() {
        if token == "*" {
            parts.push(SelectorPart::Universal);
        } else if let Some(class) = token.strip_prefix('.') {
            parts.push(SelectorPart::Class(class.to_string()));
        } else if let Some(id) = token.strip_prefix('#') {
            parts.push(SelectorPart::Id(id.to_string()));
        } else {
            parts.push(SelectorPart::Tag(token.to_lowercase()));
        }
    }
    parts
}

/// Parse declaration block into (property, value) pairs.
fn parse_declarations(block: &str) -> Vec<(String, String)> {
    let mut decls = Vec::new();
    for decl in block.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some((prop, val)) = decl.split_once(':') {
            decls.push((prop.trim().to_lowercase(), val.trim().to_string()));
        }
    }
    decls
}

/// Apply a CSS property value to a ComputedStyle.
pub fn apply_property(style: &mut ComputedStyle, property: &str, value: &str, variables: &HashMap<String, String>) {
    // Resolve CSS variables.
    let value = resolve_var(value, variables);
    let value = value.trim();

    match property {
        // --- Display ---
        "display" => {
            style.display = match value {
                "flex" => Display::Flex,
                "grid" => Display::Grid,
                "inline-grid" => Display::Grid,
                "block" => Display::Block,
                "inline-block" => Display::InlineBlock,
                "inline" => Display::InlineBlock, // approximate
                "none" => Display::None,
                _ => style.display,
            };
        }

        // --- Position ---
        "position" => {
            style.position = match value {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                _ => style.position,
            };
        }

        // --- Overflow ---
        "overflow" => {
            style.overflow = match value {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => style.overflow,
            };
        }
        "overflow-x" | "overflow-y" => {
            // Map overflow-x/overflow-y to our single overflow property.
            // Scroll beats Hidden: if one axis requests scrolling, honour it.
            let ov = match value {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => style.overflow,
            };
            match ov {
                Overflow::Scroll => style.overflow = Overflow::Scroll,
                Overflow::Hidden => {
                    if !matches!(style.overflow, Overflow::Scroll) {
                        style.overflow = Overflow::Hidden;
                    }
                }
                Overflow::Visible => {}
            }
        }

        // --- Dimensions ---
        "width" => { style.width = parse_dimension(value); }
        "height" => { style.height = parse_dimension(value); }
        "min-width" => { style.min_width = parse_dimension(value); }
        "min-height" => { style.min_height = parse_dimension(value); }
        "max-width" => { style.max_width = parse_dimension(value); }
        "max-height" => { style.max_height = parse_dimension(value); }

        // --- Margin ---
        "margin" => {
            let parts = parse_shorthand_4(value);
            style.margin.top = parts.0;
            style.margin.right = parts.1;
            style.margin.bottom = parts.2;
            style.margin.left = parts.3;
        }
        "margin-top" => { style.margin.top = parse_dimension(value); }
        "margin-right" => { style.margin.right = parse_dimension(value); }
        "margin-bottom" => { style.margin.bottom = parse_dimension(value); }
        "margin-left" => { style.margin.left = parse_dimension(value); }

        // --- Padding ---
        "padding" => {
            let parts = parse_shorthand_4(value);
            style.padding.top = parts.0;
            style.padding.right = parts.1;
            style.padding.bottom = parts.2;
            style.padding.left = parts.3;
        }
        "padding-top" => { style.padding.top = parse_dimension(value); }
        "padding-right" => { style.padding.right = parse_dimension(value); }
        "padding-bottom" => { style.padding.bottom = parse_dimension(value); }
        "padding-left" => { style.padding.left = parse_dimension(value); }

        // --- Flex ---
        "flex-direction" => {
            style.flex_direction = match value {
                "row" => FlexDirection::Row,
                "row-reverse" => FlexDirection::RowReverse,
                "column" => FlexDirection::Column,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => style.flex_direction,
            };
        }
        "flex-wrap" => {
            style.flex_wrap = match value {
                "nowrap" => FlexWrap::NoWrap,
                "wrap" => FlexWrap::Wrap,
                "wrap-reverse" => FlexWrap::WrapReverse,
                _ => style.flex_wrap,
            };
        }
        "justify-content" => {
            style.justify_content = match value {
                "flex-start" | "start" => JustifyContent::FlexStart,
                "flex-end" | "end" => JustifyContent::FlexEnd,
                "center" => JustifyContent::Center,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                "space-evenly" => JustifyContent::SpaceEvenly,
                _ => style.justify_content,
            };
        }
        "align-items" => {
            style.align_items = match value {
                "flex-start" | "start" => AlignItems::FlexStart,
                "flex-end" | "end" => AlignItems::FlexEnd,
                "center" => AlignItems::Center,
                "stretch" => AlignItems::Stretch,
                "baseline" => AlignItems::Baseline,
                _ => style.align_items,
            };
        }
        "align-self" => {
            style.align_self = match value {
                "auto" => AlignSelf::Auto,
                "flex-start" | "start" => AlignSelf::FlexStart,
                "flex-end" | "end" => AlignSelf::FlexEnd,
                "center" => AlignSelf::Center,
                "stretch" => AlignSelf::Stretch,
                _ => style.align_self,
            };
        }
        "flex-grow" => {
            if let Ok(v) = value.parse::<f32>() {
                style.flex_grow = v;
            }
        }
        "flex-shrink" => {
            if let Ok(v) = value.parse::<f32>() {
                style.flex_shrink = v;
            }
        }
        "flex-basis" => { style.flex_basis = parse_dimension(value); }

        // `flex` shorthand:  flex: <grow> [<shrink>] [<basis>]
        // Common patterns: `flex: 1`, `flex: 0 1 auto`, `flex: none`, `flex: auto`.
        "flex" => {
            match value {
                "none" => {
                    style.flex_grow = 0.0;
                    style.flex_shrink = 0.0;
                    style.flex_basis = Dimension::Auto;
                }
                "auto" => {
                    style.flex_grow = 1.0;
                    style.flex_shrink = 1.0;
                    style.flex_basis = Dimension::Auto;
                }
                _ => {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if let Some(g) = parts.first().and_then(|v| v.parse::<f32>().ok()) {
                        style.flex_grow = g;
                        // When flex shorthand has a unitless number, spec says
                        // flex-basis defaults to 0% (not auto).
                        style.flex_basis = Dimension::Percent(0.0);
                    }
                    if let Some(s) = parts.get(1).and_then(|v| v.parse::<f32>().ok()) {
                        style.flex_shrink = s;
                    }
                    if let Some(b) = parts.get(2) {
                        style.flex_basis = parse_dimension(b);
                    }
                }
            }
        }

        "gap" => {
            if let Some(v) = parse_px(value) {
                style.gap = v;
            }
        }

        // --- Grid ---
        "grid-template-columns" => {
            style.grid_template_columns = parse_grid_template(value);
        }
        "grid-template-rows" => {
            style.grid_template_rows = parse_grid_template(value);
        }
        "grid-column" => {
            // e.g. "1 / -1", "1 / 3", "span 2", "auto"
            let (start, end) = parse_grid_placement(value);
            style.grid_column_start = start;
            style.grid_column_end = end;
        }
        "grid-column-start" => {
            style.grid_column_start = value.trim().parse::<i32>().unwrap_or(0);
        }
        "grid-column-end" => {
            style.grid_column_end = parse_grid_line(value.trim());
        }
        "grid-row" => {
            let (start, end) = parse_grid_placement(value);
            style.grid_row_start = start;
            style.grid_row_end = end;
        }
        "grid-row-start" => {
            style.grid_row_start = value.trim().parse::<i32>().unwrap_or(0);
        }
        "grid-row-end" => {
            style.grid_row_end = parse_grid_line(value.trim());
        }

        // --- Position offsets ---
        "inset" => {
            // Shorthand: sets top, right, bottom, left simultaneously.
            let parts: Vec<&str> = value.split_whitespace().collect();
            match parts.len() {
                1 => {
                    let v = parse_dimension(parts[0]);
                    style.top = v; style.right = v; style.bottom = v; style.left = v;
                }
                2 => {
                    let tb = parse_dimension(parts[0]);
                    let lr = parse_dimension(parts[1]);
                    style.top = tb; style.bottom = tb; style.right = lr; style.left = lr;
                }
                4 => {
                    style.top = parse_dimension(parts[0]);
                    style.right = parse_dimension(parts[1]);
                    style.bottom = parse_dimension(parts[2]);
                    style.left = parse_dimension(parts[3]);
                }
                _ => {
                    let v = parse_dimension(value);
                    style.top = v; style.right = v; style.bottom = v; style.left = v;
                }
            }
        }
        "top" => { style.top = parse_dimension(value); }
        "right" => { style.right = parse_dimension(value); }
        "bottom" => { style.bottom = parse_dimension(value); }
        "left" => { style.left = parse_dimension(value); }

        // --- Background ---
        "background-color" | "background" => {
            // Try linear-gradient() first
            if let Some(grad) = parse_linear_gradient(value) {
                style.background = grad;
            } else if let Some(rad) = parse_radial_gradient(value) {
                style.background = rad;
            } else if let Some(color) = parse_color(value) {
                style.background = Background::Solid(color);
            }
        }

        // --- Border ---
        "border" => {
            // Shorthand: 1px solid #color  or  1px solid rgba(...)
            // First extract width from the beginning.
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_width = crate::cxrd::value::EdgeInsets::uniform(width);
            }
            // Extract color: find the color portion (could be rgba(...) spanning multiple whitespace-split parts).
            let color_start = value.find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.border_color = c;
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.border_color = color;
            }
        }
        "border-color" => {
            if let Some(c) = parse_color(value) {
                style.border_color = c;
            }
        }
        "border-width" => {
            if let Some(w) = parse_px(value) {
                style.border_width = crate::cxrd::value::EdgeInsets::uniform(w);
            }
        }
        "border-radius" => {
            // Shorthand: uniform or per-corner
            let parts = split_css_function_aware(value);
            match parts.len() {
                1 => {
                    if let Some(v) = parse_px(&parts[0]) {
                        style.border_radius = crate::cxrd::value::CornerRadii::uniform(v);
                    }
                }
                4 => {
                    let tl = parse_px(&parts[0]).unwrap_or(0.0);
                    let tr = parse_px(&parts[1]).unwrap_or(0.0);
                    let br = parse_px(&parts[2]).unwrap_or(0.0);
                    let bl = parse_px(&parts[3]).unwrap_or(0.0);
                    style.border_radius = crate::cxrd::value::CornerRadii { top_left: tl, top_right: tr, bottom_right: br, bottom_left: bl };
                }
                _ => {}
            }
        }

        // --- Typography ---
        "color" => {
            if let Some(c) = parse_color(value) {
                style.color = c;
            }
        }
        "font-family" => {
            // Extract the first font family from comma-separated list.
            let first = value.split(',').next().unwrap_or(value);
            let family = first.trim().trim_matches(|c: char| c == '"' || c == '\'');
            style.font_family = family.to_string();
        }
        "font-size" => {
            if let Some(v) = parse_px(value) {
                style.font_size = v;
            }
        }
        "font-weight" => {
            let w = match value {
                "normal" => 400,
                "bold" => 700,
                "lighter" => 300,
                "bolder" => 600,
                _ => value.parse::<u16>().unwrap_or(400),
            };
            style.font_weight = FontWeight(w);
        }
        "line-height" => {
            if let Ok(v) = value.parse::<f32>() {
                style.line_height = v;
            } else if let Some(v) = parse_px(value) {
                style.line_height = v / style.font_size.max(1.0);
            }
        }
        "text-align" => {
            style.text_align = match value {
                "left" => TextAlign::Left,
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                _ => style.text_align,
            };
        }
        "letter-spacing" => {
            if let Some(em_str) = value.strip_suffix("em") {
                // em-based letter-spacing: resolve relative to font-size.
                if let Ok(v) = em_str.trim().parse::<f32>() {
                    style.letter_spacing = v * style.font_size;
                }
            } else if let Some(v) = parse_px(value) {
                style.letter_spacing = v;
            }
        }
        "text-transform" => {
            style.text_transform = match value {
                "uppercase" => TextTransform::Uppercase,
                "lowercase" => TextTransform::Lowercase,
                "capitalize" => TextTransform::Capitalize,
                "none" => TextTransform::None,
                _ => style.text_transform,
            };
        }

        // --- Visual ---
        "opacity" => {
            if let Ok(v) = value.parse::<f32>() {
                style.opacity = v.clamp(0.0, 1.0);
            }
        }
        "backdrop-filter" => {
            if let Some(blur_px) = parse_backdrop_blur(value) {
                style.backdrop_blur = blur_px.max(0.0);
            }
        }
        "transform" => {
            if let Some(scale) = parse_transform_scale(value) {
                style.transform_scale = scale.max(0.01);
            }
        }
        "z-index" => {
            if let Ok(v) = value.parse::<i32>() {
                style.z_index = v;
            }
        }

        // --- Box shadow ---
        "box-shadow" => {
            if value == "none" {
                style.box_shadow.clear();
            } else if let Some(shadow) = parse_box_shadow(value) {
                style.box_shadow = vec![shadow];
            }
        }

        // --- Transition ---
        "transition" => {
            // TODO: parse transition shorthand into TransitionDef
        }

        _ => {
            // Unsupported property — silently ignore.
            log::debug!("Unsupported CSS property: {}", property);
        }
    }
}

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
fn parse_calc_dimension(expr: &str) -> Dimension {
    let expr = expr.trim();

    // Try to detect the "dominant" unit in the expression.
    // Common pattern: "100% / 1" or "100% / var" (already resolved).
    if expr.contains('%') {
        // Extract the percentage value and any arithmetic after it.
        // Pattern: `N% OP M`
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if let Some(pct_pos) = parts.iter().position(|p| p.ends_with('%')) {
            let pct_val = parts[pct_pos].trim_end_matches('%').parse::<f32>().unwrap_or(100.0);

            // Check for operator + operand after the percentage.
            if pct_pos + 2 < parts.len() {
                let op = parts[pct_pos + 1];
                let rhs_str = parts[pct_pos + 2].trim_end_matches("px");
                let rhs = eval_calc_expr(rhs_str).unwrap_or(1.0);
                match op {
                    "/" => return Dimension::Percent(pct_val / rhs),
                    "*" => return Dimension::Percent(pct_val * rhs),
                    "+" | "-" => {
                        // Mixed units — can't perfectly represent, use percentage.
                        return Dimension::Percent(pct_val);
                    }
                    _ => {}
                }
            }
            return Dimension::Percent(pct_val);
        }
    }

    // Pure px or unitless arithmetic: "10px + 20px", "300 - 50", etc.
    // Strip all "px" suffixes and evaluate as arithmetic.
    let cleaned = expr.replace("px", "");
    if let Some(result) = eval_calc_expr(&cleaned) {
        return Dimension::Px(result);
    }

    Dimension::Auto
}

/// Evaluate a simple arithmetic expression (supports +, -, *, /).
/// Handles operator precedence: * and / before + and -.
fn eval_calc_expr(expr: &str) -> Option<f32> {
    let expr = expr.trim();

    // Tokenize into numbers and operators.
    let mut tokens: Vec<CalcToken> = Vec::new();
    let mut pos = 0;
    let bytes = expr.as_bytes();

    while pos < bytes.len() {
        // Skip whitespace.
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() { break; }

        // Operator?
        if matches!(bytes[pos], b'+' | b'*' | b'/') {
            tokens.push(CalcToken::Op(bytes[pos] as char));
            pos += 1;
            continue;
        }

        // Minus: could be operator or negative sign.
        if bytes[pos] == b'-' {
            // It's a negative sign if: first token, or previous token is an operator.
            let is_neg = tokens.is_empty() || matches!(tokens.last(), Some(CalcToken::Op(_)));
            if !is_neg {
                tokens.push(CalcToken::Op('-'));
                pos += 1;
                continue;
            }
        }

        // Number (possibly negative).
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

        // Unknown character — skip.
        pos += 1;
    }

    // Evaluate with precedence: first pass handles * and /.
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

    // Second pass: + and -.
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

/// Parse a px value.
fn parse_px(value: &str) -> Option<f32> {
    let value = value.trim();
    // Handle calc() expressions.
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

/// Parse `backdrop-filter` blur amount from values like `blur(8px)` or
/// `blur(calc(var(--panel-blur) * 1px))` (after var() resolution).
fn parse_backdrop_blur(value: &str) -> Option<f32> {
    let v = value.trim();
    let start = v.find("blur(")?;
    let inner = &v[start + 5..];

    // Use the last ')' so nested functions like blur(calc(...)) work.
    let end = inner.rfind(')')?;
    let expr = inner[..end].trim();
    parse_px(expr)
}

/// Parse transform scale from values like `scale(1.2)`.
fn parse_transform_scale(value: &str) -> Option<f32> {
    let v = value.trim();
    let start = v.find("scale(")?;
    let inner = &v[start + 6..];
    let end = inner.find(')')?;
    inner[..end].trim().parse::<f32>().ok()
}

/// Split a string on commas, but respect nested parentheses.
fn split_comma_aware(s: &str) -> Vec<String> {
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
fn parse_linear_gradient(value: &str) -> Option<Background> {
    let inner = value.strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;
    let parts = split_comma_aware(inner);
    if parts.is_empty() { return None; }

    let mut idx = 0;
    let mut angle_deg: f32 = 180.0; // default: top-to-bottom

    // Try to parse the first part as an angle or direction
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
        // A stop can be "color position%" or just "color"
        // Try to find a percentage or px at the end
        let (color_str, position) = if let Some(pct_idx) = part.rfind('%') {
            // Find the start of the percentage number
            let before = &part[..pct_idx];
            if let Some(space_idx) = before.rfind(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
                let num_str = &part[space_idx+1..pct_idx];
                let pos = num_str.parse::<f32>().unwrap_or(i as f32 / (n - 1).max(1) as f32 * 100.0) / 100.0;
                (&part[..=space_idx], Some(pos))
            } else {
                (part, None)
            }
        } else {
            // Check for px position at end
            let words: Vec<&str> = part.rsplitn(2, char::is_whitespace).collect();
            if words.len() == 2 {
                if let Some(px) = parse_px(words[0]) {
                    // px positions need parent width context — approximate as percentage
                    (words[1], Some(px / 100.0)) // rough approximation
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
fn parse_radial_gradient(value: &str) -> Option<Background> {
    let inner = value.strip_prefix("radial-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;
    let parts = split_comma_aware(inner);
    if parts.is_empty() { return None; }

    let mut stops = Vec::new();
    let n = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let part = part.trim();
        // Skip shape/extent keywords at the front (e.g. "circle at center")
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

/// Parse a single `box-shadow` value: `offset-x offset-y blur-radius spread-radius color`
fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let value = value.trim();
    if value == "none" || value.is_empty() { return None; }

    // Extract color portion: find rgba(...) or rgb(...) or #hex or named color
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
        // Try the last token as a hex or named color
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

    // Parse numeric values from remainder
    let nums: Vec<f32> = remainder.split_whitespace()
        .filter_map(|t| parse_px(t))
        .collect();

    let offset_x = nums.first().copied().unwrap_or(0.0);
    let offset_y = nums.get(1).copied().unwrap_or(0.0);
    let blur_radius = nums.get(2).copied().unwrap_or(0.0);
    let spread_radius = nums.get(3).copied().unwrap_or(0.0);

    Some(BoxShadow { offset_x, offset_y, blur_radius, spread_radius, color, inset: false })
}

/// Parse a CSS color value.
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();

    // Named colors
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

    // rgb() / rgba()
    if let Some(args) = value.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|s| parse_color_component(s)).collect();
        if nums.len() >= 4 {
            let r = if nums[0] > 1.0 { nums[0] / 255.0 } else { nums[0] };
            let g = if nums[1] > 1.0 { nums[1] / 255.0 } else { nums[1] };
            let b = if nums[2] > 1.0 { nums[2] / 255.0 } else { nums[2] };
            return Some(Color::new(r, g, b, nums[3]));
        }
    }

    if let Some(args) = value.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|s| parse_color_component(s)).collect();
        if nums.len() >= 3 {
            let r = if nums[0] > 1.0 { nums[0] / 255.0 } else { nums[0] };
            let g = if nums[1] > 1.0 { nums[1] / 255.0 } else { nums[1] };
            let b = if nums[2] > 1.0 { nums[2] / 255.0 } else { nums[2] };
            return Some(Color::new(r, g, b, 1.0));
        }
    }

    // hsl() / hsla()
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

fn hsl_to_rgb(h_deg: f32, s: f32, l: f32) -> (f32, f32, f32) {
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

/// Parse a single color component which may be a plain number, percentage,
/// or a `calc()` expression like `calc(50 * 3)`.
fn parse_color_component(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.ends_with('%') {
        return s.trim_end_matches('%').parse::<f32>().ok().map(|v| v / 100.0);
    }
    // Try plain number first.
    if let Ok(v) = s.parse::<f32>() {
        return Some(v);
    }
    // Try calc() expression.
    if let Some(inner) = s.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
        return eval_calc_expr(inner);
    }
    // Try evaluating as arithmetic even without calc() wrapper
    // (handles things like `50 * 3` that result from var() expansion).
    if s.contains('*') || s.contains('/') || (s.contains('+') && !s.starts_with('+')) || (s.contains('-') && !s.starts_with('-') && s.len() > 1) {
        return eval_calc_expr(s);
    }
    None
}

/// Parse a CSS shorthand with 1–4 values (margin, padding, etc.).
fn parse_shorthand_4(value: &str) -> (Dimension, Dimension, Dimension, Dimension) {
    // Use function-aware split to respect calc() parentheses.
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

/// Resolve CSS `var(--name)` references.
fn resolve_var(value: &str, variables: &HashMap<String, String>) -> String {
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

/// Parse a `grid-template-columns` or `grid-template-rows` value into track sizes.
///
/// Handles: `300px 1fr 300px`, `auto 1fr auto`,
///          `calc(300 * 1px) 1fr calc(300 * 1px)`, `1fr 1fr`, `repeat(3, 1fr)`.
fn parse_grid_template(value: &str) -> Vec<GridTrackSize> {
    let value = value.trim();

    // Handle repeat() — simple case: repeat(N, track)
    if value.starts_with("repeat(") {
        if let Some(inner) = value.strip_prefix("repeat(").and_then(|s| s.strip_suffix(')')) {
            if let Some((count_str, track_str)) = inner.split_once(',') {
                let count = count_str.trim().parse::<usize>().unwrap_or(1);
                let track = parse_single_grid_track(track_str.trim());
                return vec![track; count];
            }
        }
    }

    // Split on whitespace but respect calc() parentheses.
    let tokens = split_css_function_aware(value);
    tokens.iter().map(|t| parse_single_grid_track(t)).collect()
}

/// Parse a single grid track size value.
fn parse_single_grid_track(value: &str) -> GridTrackSize {
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
    // Try as a dimension (handles calc(), px, etc.)
    match parse_dimension(value) {
        Dimension::Px(v) => GridTrackSize::Px(v),
        Dimension::Percent(v) => GridTrackSize::Percent(v),
        _ => GridTrackSize::Auto,
    }
}

/// Split a CSS value on whitespace, respecting parenthesized groups like `calc(...)`.
fn split_css_function_aware(value: &str) -> Vec<String> {
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

/// Parse a grid placement shorthand like "1 / -1", "1 / 3", "span 2", "auto".
fn parse_grid_placement(value: &str) -> (i32, i32) {
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
fn parse_grid_line(value: &str) -> i32 {
    let value = value.trim();
    if value == "auto" {
        return 0;
    }
    value.parse::<i32>().unwrap_or(0)
}

/// Strip CSS comments.
fn strip_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut in_comment = false;
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if !in_comment && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_comment = true;
            i += 2;
        } else if in_comment && i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            in_comment = false;
            i += 2;
        } else if !in_comment {
            result.push(bytes[i] as char);
            i += 1;
        } else {
            i += 1;
        }
    }

    result
}

/// Find the matching closing brace for an opening brace at `start`.
fn find_matching_brace(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'{' { depth += 1; }
        if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
