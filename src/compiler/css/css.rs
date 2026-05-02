// prism-runtime/src/compiler/css/css.rs
//
// CSS property database + property-application engine.
// All value parsing lives in `super::parsing`.

use crate::prd::style::*;
use crate::prd::value::Dimension;
use std::collections::HashMap;
use super::parsing::*;

// ═════════════════════════════════════════════════════════════════════════════
//  CSS PROPERTY DATABASE
// ═════════════════════════════════════════════════════════════════════════════

pub static CSS_PROPERTIES: &[&str] = &[
    // ── Layout ───────────────────────────────────────────────────────────
    "display", "position", "top", "right", "bottom", "left", "inset",
    "float", "clear", "z-index",

    // ── Box model ────────────────────────────────────────────────────────
    "width", "height", "min-width", "min-height", "max-width", "max-height",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "box-sizing",

    // ── Flexbox ──────────────────────────────────────────────────────────
    "flex", "flex-grow", "flex-shrink", "flex-basis",
    "flex-direction", "flex-wrap", "flex-flow",
    "justify-content", "align-items", "align-content", "align-self",
    "order", "gap", "row-gap", "column-gap",

    // ── Grid ─────────────────────────────────────────────────────────────
    "grid", "grid-template", "grid-template-columns", "grid-template-rows",
    "grid-template-areas", "grid-auto-flow", "grid-auto-columns", "grid-auto-rows",
    "grid-column", "grid-column-start", "grid-column-end",
    "grid-row", "grid-row-start", "grid-row-end",
    "grid-area", "grid-gap",

    // ── Typography ───────────────────────────────────────────────────────
    "font", "font-family", "font-size", "font-weight", "font-style",
    "font-variant", "font-stretch", "line-height",
    "letter-spacing", "word-spacing",
    "text-align", "text-decoration", "text-decoration-line",
    "text-decoration-color", "text-decoration-style", "text-decoration-thickness",
    "text-transform", "text-overflow", "white-space",
    "word-break", "word-wrap", "overflow-wrap",

    // ── Background ───────────────────────────────────────────────────────
    "background", "background-color", "background-image",
    "background-position", "background-size",
    "background-repeat", "background-attachment",
    "background-clip", "background-origin", "background-blend-mode",

    // ── Border ───────────────────────────────────────────────────────────
    "border", "border-width", "border-style", "border-color",
    "border-top", "border-right", "border-bottom", "border-left",
    "border-top-width", "border-right-width", "border-bottom-width", "border-left-width",
    "border-top-color", "border-right-color", "border-bottom-color", "border-left-color",
    "border-top-style", "border-right-style", "border-bottom-style", "border-left-style",
    "border-radius",
    "border-top-left-radius", "border-top-right-radius",
    "border-bottom-right-radius", "border-bottom-left-radius",

    // ── Effects ──────────────────────────────────────────────────────────
    "box-shadow", "opacity", "filter", "backdrop-filter",

    // ── Transform ────────────────────────────────────────────────────────
    "transform", "transform-origin", "transform-style",
    "perspective", "perspective-origin", "backface-visibility",
    "rotate", "scale", "translate",

    // ── Animation ────────────────────────────────────────────────────────
    "animation", "animation-name", "animation-duration",
    "animation-delay", "animation-iteration-count",
    "animation-direction", "animation-fill-mode",
    "animation-play-state", "animation-timing-function",

    // ── Transition ───────────────────────────────────────────────────────
    "transition", "transition-property", "transition-duration",
    "transition-delay", "transition-timing-function",

    // ── Interaction ──────────────────────────────────────────────────────
    "cursor", "pointer-events", "user-select",

    // ── Overflow ─────────────────────────────────────────────────────────
    "overflow", "overflow-x", "overflow-y",

    // ── Positioning / sizing ─────────────────────────────────────────────
    "aspect-ratio", "object-fit", "object-position",
    "vertical-align",

    // ── Outline ──────────────────────────────────────────────────────────
    "outline", "outline-color", "outline-width", "outline-offset", "outline-style",

    // ── Place shorthands ─────────────────────────────────────────────────
    "place-items", "place-content", "place-self",
    "justify-items", "justify-self",

    // ── Logical properties ───────────────────────────────────────────────
    "margin-block", "margin-block-start", "margin-block-end",
    "margin-inline", "margin-inline-start", "margin-inline-end",
    "padding-block", "padding-block-start", "padding-block-end",
    "padding-inline", "padding-inline-start", "padding-inline-end",
    "inset-block", "inset-block-start", "inset-block-end",
    "inset-inline", "inset-inline-start", "inset-inline-end",
    "block-size", "inline-size",
    "min-block-size", "min-inline-size", "max-block-size", "max-inline-size",

    // ── Logical border ───────────────────────────────────────────────────
    "border-block", "border-block-start", "border-block-end",
    "border-inline", "border-inline-start", "border-inline-end",
    "border-start-start-radius", "border-start-end-radius",
    "border-end-end-radius", "border-end-start-radius",

    // ── Scroll ───────────────────────────────────────────────────────────
    "scroll-behavior", "scroll-snap-type", "scroll-snap-align",

    // ── Containment ──────────────────────────────────────────────────────
    "contain", "content-visibility", "will-change",

    // ── Color ────────────────────────────────────────────────────────────
    "color", "accent-color", "caret-color", "color-scheme",

    // ── Visibility ───────────────────────────────────────────────────────
    "visibility", "isolation", "mix-blend-mode",

    // ── SVG ──────────────────────────────────────────────────────────────
    "fill", "stroke", "stroke-width",

    // ── CSS-wide ─────────────────────────────────────────────────────────
    "all",
];

pub static CSS_PSEUDO_CLASSES: &[&str] = &[
    // Interactive
    "hover", "active", "focus", "focus-visible", "focus-within",

    // Structural
    "first-child", "last-child", "only-child",
    "first-of-type", "last-of-type", "only-of-type",
    "nth-child", "nth-last-child", "nth-of-type", "nth-last-of-type",
    "empty", "root", "scope",

    // Logical / functional
    "is", "not", "where", "has", "matches",

    // Forms
    "checked", "default", "defined", "disabled", "enabled",
    "required", "optional",
    "valid", "invalid", "user-valid", "user-invalid",
    "in-range", "out-of-range",
    "read-only", "read-write", "placeholder-shown", "autofill",
    "indeterminate",

    // Links
    "link", "visited", "any-link", "local-link",

    // UI / media state
    "fullscreen", "modal", "picture-in-picture",
    "playing", "paused", "seeking", "stalled", "buffering",
    "muted", "volume-locked",
    "open", "popover-open",

    // Target
    "target", "target-current", "target-before", "target-after",

    // Language / direction
    "lang", "dir",

    // Host / custom elements
    "host", "host-context", "state", "has-slotted",

    // View transition
    "active-view-transition", "active-view-transition-type",

    // Time-based
    "current", "past", "future",

    // Page
    "first", "left", "right", "blank",

    // Heading
    "heading",

    // Interest
    "interest-source", "interest-target",
];

pub static CSS_PSEUDO_ELEMENTS: &[&str] = &[
    "before", "after",
    "first-letter", "first-line",
    "selection",
    "placeholder",
    "marker",
    "backdrop",
    "cue",
    "file-selector-button",
    "details-content",
    "grammar-error", "spelling-error",
    "target-text",
    "highlight",
    "part", "slotted",
    "column",
    "checkmark", "picker", "picker-icon",
    "scroll-button", "scroll-marker", "scroll-marker-group",
    "view-transition", "view-transition-group",
    "view-transition-image-pair", "view-transition-new", "view-transition-old",
];

pub static CSS_AT_RULES: &[&str] = &[
    "charset", "import", "namespace",
    "media", "supports", "layer",
    "container", "scope",
    "keyframes",
    "font-face", "font-feature-values", "font-palette-values",
    "counter-style", "property", "color-profile",
    "page",
    "starting-style",
    "view-transition", "position-try",
    "function", "custom-media", "document",
];

// ═════════════════════════════════════════════════════════════════════════════
//  CSS ENGINE — `Css` struct + implementation
// ═════════════════════════════════════════════════════════════════════════════

pub struct Css;

impl Css {
    // ── Database accessors ───────────────────────────────────────────────

    pub fn properties() -> &'static [&'static str] {
        CSS_PROPERTIES
    }

    pub fn pseudo_classes() -> &'static [&'static str] {
        CSS_PSEUDO_CLASSES
    }

    pub fn pseudo_elements() -> &'static [&'static str] {
        CSS_PSEUDO_ELEMENTS
    }

    pub fn at_rules() -> &'static [&'static str] {
        CSS_AT_RULES
    }

    // ── Pseudo-class helpers ─────────────────────────────────────────────

    /// Check whether `name` is a recognized CSS pseudo-class.
    pub fn is_pseudo_class(name: &str) -> bool {
        let base = name.split('(').next().unwrap_or(name);
        matches!(
            base,
            "active" | "hover" | "focus" | "focus-visible" | "focus-within"
            | "first-child" | "last-child" | "only-child"
            | "first-of-type" | "last-of-type" | "only-of-type"
            | "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type"
            | "empty" | "root" | "scope"
            | "checked" | "default" | "defined" | "disabled" | "enabled"
            | "in-range" | "out-of-range" | "indeterminate"
            | "invalid" | "valid" | "user-invalid" | "user-valid"
            | "optional" | "required" | "read-only" | "read-write"
            | "placeholder-shown" | "autofill"
            | "link" | "visited" | "any-link" | "local-link"
            | "fullscreen" | "modal" | "picture-in-picture"
            | "playing" | "paused" | "seeking" | "stalled" | "buffering"
            | "muted" | "volume-locked"
            | "open" | "popover-open"
            | "is" | "not" | "has" | "where" | "matches"
            | "dir" | "lang"
            | "target" | "target-current" | "target-before" | "target-after"
            | "first" | "left" | "right" | "blank"
            | "heading"
            | "host" | "host-context" | "state" | "has-slotted"
            | "interest-source" | "interest-target"
            | "active-view-transition" | "active-view-transition-type"
            | "current" | "past" | "future"
        )
    }

    /// Check whether `name` is a recognized CSS pseudo-element.
    pub fn is_pseudo_element(name: &str) -> bool {
        let base = name.trim_start_matches(':').split('(').next().unwrap_or(name);
        matches!(
            base,
            "after" | "before"
            | "first-letter" | "first-line"
            | "selection" | "placeholder" | "marker"
            | "backdrop" | "cue"
            | "file-selector-button" | "details-content"
            | "grammar-error" | "spelling-error" | "target-text"
            | "highlight" | "part" | "slotted" | "column"
            | "checkmark" | "picker" | "picker-icon"
            | "scroll-button" | "scroll-marker" | "scroll-marker-group"
            | "view-transition" | "view-transition-group"
            | "view-transition-image-pair" | "view-transition-new" | "view-transition-old"
        )
    }

    /// Check whether `name` is a recognized CSS at-rule.
    pub fn is_at_rule(name: &str) -> bool {
        let base = name.trim_start_matches('@');
        matches!(
            base,
            "charset" | "import" | "namespace"
            | "media" | "supports" | "layer"
            | "keyframes" | "font-face" | "font-feature-values" | "font-palette-values"
            | "counter-style" | "page" | "property" | "color-profile"
            | "container" | "scope"
            | "starting-style" | "view-transition" | "position-try"
            | "function" | "custom-media" | "document"
        )
    }

    /// Classify a pseudo-class for runtime behavior dispatch.
    pub fn pseudo_class_category(name: &str) -> PseudoClassCategory {
        let base = name.split('(').next().unwrap_or(name);
        match base {
            "active" | "hover" | "focus" | "focus-visible" | "focus-within" => {
                PseudoClassCategory::Interactive
            }
            "first-child" | "last-child" | "only-child"
            | "first-of-type" | "last-of-type" | "only-of-type"
            | "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type"
            | "empty" | "root" | "scope" => PseudoClassCategory::Structural,
            "checked" | "default" | "defined" | "disabled" | "enabled"
            | "in-range" | "out-of-range" | "indeterminate"
            | "invalid" | "valid" | "user-invalid" | "user-valid"
            | "optional" | "required" | "read-only" | "read-write"
            | "placeholder-shown" | "autofill" | "blank" => PseudoClassCategory::FormState,
            "link" | "visited" | "any-link" | "local-link" => PseudoClassCategory::LinkState,
            "fullscreen" | "modal" | "picture-in-picture"
            | "playing" | "paused" | "seeking" | "stalled" | "buffering"
            | "muted" | "volume-locked" | "open" | "popover-open"
            | "active-view-transition" | "active-view-transition-type"
            | "current" | "past" | "future" => PseudoClassCategory::MediaState,
            "is" | "not" | "has" | "where" | "matches" => PseudoClassCategory::Functional,
            "host" | "host-context" | "state" | "has-slotted" | "heading"
            | "dir" | "lang" | "target" | "target-current" | "target-before"
            | "target-after" => PseudoClassCategory::ElementState,
            _ => PseudoClassCategory::Unknown,
        }
    }

    // ── Property application ─────────────────────────────────────────────

    /// Apply a CSS property to a `ComputedStyle`. Returns `true` if recognized.
    pub fn apply_property(
        style: &mut ComputedStyle,
        property: &str,
        value: &str,
        variables: &HashMap<String, String>,
    ) -> bool {
        let value = resolve_var(value, variables);
        let value = value.trim();

    match property {
        // ─────────────────────────── Display ──────────────────────────────
        "display" => {
            style.display = match value {
                "flex" => Display::Flex,
                "inline-flex" => Display::InlineFlex,
                "grid" => Display::Grid,
                "inline-grid" => Display::InlineGrid,
                "block" => Display::Block,
                "inline" => Display::Inline,
                "inline-block" => Display::InlineBlock,
                "none" => Display::None,
                _ => style.display,
            };
        }
        // ─────────────────────────── Position ─────────────────────────────
        "position" => {
            style.position = match value {
                "static" => Position::Static,
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" => Position::Sticky,
                _ => style.position,
            };
        }
        // ─────────────────────────── Overflow ─────────────────────────────
        "overflow" => {
            let ov = match value {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => style.overflow,
            };
            style.overflow = ov;
            style.overflow_x = ov;
            style.overflow_y = ov;
        }
        "overflow-x" => {
            style.overflow_x = match value {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => style.overflow_x,
            };
            // Update combined overflow to most restrictive
            style.overflow = most_restrictive_overflow(style.overflow_x, style.overflow_y);
        }
        "overflow-y" => {
            style.overflow_y = match value {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => style.overflow_y,
            };
            style.overflow = most_restrictive_overflow(style.overflow_x, style.overflow_y);
        }
        // ─────────────────────────── Dimensions ───────────────────────────
        "width" => {
            style.width = parse_dimension(value);
        }
        "height" => {
            style.height = parse_dimension(value);
        }
        "min-width" => {
            style.min_width = parse_dimension(value);
        }
        "min-height" => {
            style.min_height = parse_dimension(value);
        }
        "max-width" => {
            style.max_width = parse_dimension(value);
        }
        "max-height" => {
            style.max_height = parse_dimension(value);
        }
        // ─────────────────────────── Margin ───────────────────────────────
        "margin" => {
            let parts = parse_shorthand_4(value);
            style.margin.top = parts.0;
            style.margin.right = parts.1;
            style.margin.bottom = parts.2;
            style.margin.left = parts.3;
        }
        "margin-top" => {
            style.margin.top = parse_dimension(value);
        }
        "margin-right" => {
            style.margin.right = parse_dimension(value);
        }
        "margin-bottom" => {
            style.margin.bottom = parse_dimension(value);
        }
        "margin-left" => {
            style.margin.left = parse_dimension(value);
        }

        // ─────────────────────────── Padding ──────────────────────────────
        "padding" => {
            let parts = parse_shorthand_4(value);
            style.padding.top = parts.0;
            style.padding.right = parts.1;
            style.padding.bottom = parts.2;
            style.padding.left = parts.3;
        }
        "padding-top" => {
            style.padding.top = parse_dimension(value);
        }
        "padding-right" => {
            style.padding.right = parse_dimension(value);
        }
        "padding-bottom" => {
            style.padding.bottom = parse_dimension(value);
        }
        "padding-left" => {
            style.padding.left = parse_dimension(value);
        }
        // ─────────────────────────── Flexbox ──────────────────────────────
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
        "flex-flow" => {
            // Shorthand: <flex-direction> <flex-wrap>
            let parts: Vec<&str> = value.split_whitespace().collect();
            for part in &parts {
                match *part {
                    "row" => style.flex_direction = FlexDirection::Row,
                    "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                    "column" => style.flex_direction = FlexDirection::Column,
                    "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                    "nowrap" => style.flex_wrap = FlexWrap::NoWrap,
                    "wrap" => style.flex_wrap = FlexWrap::Wrap,
                    "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                    _ => {}
                }
            }
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
        "align-content" => {
            style.align_content = match value {
                "flex-start" | "start" => AlignContent::FlexStart,
                "flex-end" | "end" => AlignContent::FlexEnd,
                "center" => AlignContent::Center,
                "stretch" => AlignContent::Stretch,
                "space-between" => AlignContent::SpaceBetween,
                "space-around" => AlignContent::SpaceAround,
                "space-evenly" => AlignContent::SpaceEvenly,
                _ => style.align_content,
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
        "flex-basis" => {
            style.flex_basis = parse_dimension(value);
        }
        "flex" => match value {
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
                    style.flex_basis = Dimension::Percent(0.0);
                }
                if let Some(s) = parts.get(1).and_then(|v| v.parse::<f32>().ok()) {
                    style.flex_shrink = s;
                }
                if let Some(b) = parts.get(2) {
                    style.flex_basis = parse_dimension(b);
                }
            }
        },
        "gap" => {
            if let Some(v) = parse_px(value) {
                style.gap = v;
            }
        }
        "row-gap" => {
            if let Some(v) = parse_px(value) {
                style.row_gap = v;
            }
        }
        "column-gap" => {
            if let Some(v) = parse_px(value) {
                style.column_gap = v;
            }
        }
        "order" => {
            if let Ok(v) = value.parse::<i32>() {
                style.order = v;
            }
        }
        // ─────────────────────────── Grid ─────────────────────────────────
        "grid-template-columns" => {
            style.grid_template_columns = parse_grid_template(value);
        }
        "grid-template-rows" => {
            style.grid_template_rows = parse_grid_template(value);
        }
        "grid-column" => {
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
        // ─────────────────────────── Position offsets ─────────────────────
        "inset" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            match parts.len() {
                1 => {
                    let v = parse_dimension(parts[0]);
                    style.top = v;
                    style.right = v;
                    style.bottom = v;
                    style.left = v;
                }
                2 => {
                    let tb = parse_dimension(parts[0]);
                    let lr = parse_dimension(parts[1]);
                    style.top = tb;
                    style.bottom = tb;
                    style.right = lr;
                    style.left = lr;
                }
                4 => {
                    style.top = parse_dimension(parts[0]);
                    style.right = parse_dimension(parts[1]);
                    style.bottom = parse_dimension(parts[2]);
                    style.left = parse_dimension(parts[3]);
                }
                _ => {
                    let v = parse_dimension(value);
                    style.top = v;
                    style.right = v;
                    style.bottom = v;
                    style.left = v;
                }
            }
        }
        "top" => {
            style.top = parse_dimension(value);
        }
        "right" => {
            style.right = parse_dimension(value);
        }
        "bottom" => {
            style.bottom = parse_dimension(value);
        }
        "left" => {
            style.left = parse_dimension(value);
        }
        // ─────────────────────────── Background ───────────────────────────
        "background-color" | "background" => {
            if let Some(grad) = parse_linear_gradient(value) {
                style.background = grad;
            } else if let Some(rad) = parse_radial_gradient(value) {
                style.background = rad;
            } else if let Some(color) = parse_color(value) {
                style.background = Background::Solid(color);
            }
        }
        "background-image" => {
            if let Some(start) = value.find("url(") {
                let rest = &value[start + 4..];
                let url = rest.trim_start_matches(|c: char| c == '\'' || c == '"');
                let end = url
                    .find(|c: char| c == '\'' || c == '"' || c == ')')
                    .unwrap_or(url.len());
                style.background_image = Some(url[..end].to_string());
            }
        }
        "background-size" => {
            style.background_size = match value {
                "cover" => BackgroundSize::Cover,
                "contain" => BackgroundSize::Contain,
                "auto" => BackgroundSize::Auto,
                _ => {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() == 2 {
                        let w = parse_px(parts[0]).unwrap_or(0.0);
                        let h = parse_px(parts[1]).unwrap_or(0.0);
                        BackgroundSize::Explicit(w, h)
                    } else if let Some(w) = parse_px(value) {
                        BackgroundSize::Explicit(w, 0.0)
                    } else {
                        style.background_size
                    }
                }
            };
        }
        "background-position" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let parse_pos = |s: &str| -> BackgroundPosition {
                if s == "center" {
                    BackgroundPosition::Center
                } else if s == "left" || s == "top" {
                    BackgroundPosition::Percent(0.0)
                } else if s == "right" || s == "bottom" {
                    BackgroundPosition::Percent(100.0)
                } else if let Some(p) = s.strip_suffix('%') {
                    BackgroundPosition::Percent(p.parse().unwrap_or(0.0))
                } else if let Some(v) = parse_px(s) {
                    BackgroundPosition::Px(v)
                } else {
                    BackgroundPosition::Percent(0.0)
                }
            };
            match parts.len() {
                1 => {
                    let p = parse_pos(parts[0]);
                    style.background_position = (p, BackgroundPosition::Center);
                }
                2 => {
                    style.background_position = (parse_pos(parts[0]), parse_pos(parts[1]));
                }
                _ => {}
            }
        }
        "background-repeat" => {
            style.background_repeat = match value {
                "repeat" => BackgroundRepeat::Repeat,
                "no-repeat" => BackgroundRepeat::NoRepeat,
                "repeat-x" => BackgroundRepeat::RepeatX,
                "repeat-y" => BackgroundRepeat::RepeatY,
                _ => style.background_repeat,
            };
        }
        // ─────────────────────────── Border (shorthand) ──────────────────
        "border" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_width = crate::prd::value::EdgeInsets::uniform(width);
            }
            let color_start = value
                .find("rgba(")
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
                style.border_width = crate::prd::value::EdgeInsets::uniform(w);
            }
        }
        "border-radius" => {
            let parts = split_css_function_aware(value);
            match parts.len() {
                1 => {
                    if let Some(v) = parse_px(&parts[0]) {
                        style.border_radius = crate::prd::value::CornerRadii::uniform(v);
                    }
                }
                2 => {
                    let tl_br = parse_px(&parts[0]).unwrap_or(0.0);
                    let tr_bl = parse_px(&parts[1]).unwrap_or(0.0);
                    style.border_radius = crate::prd::value::CornerRadii {
                        top_left: tl_br,
                        top_right: tr_bl,
                        bottom_right: tl_br,
                        bottom_left: tr_bl,
                    };
                }
                3 => {
                    let tl = parse_px(&parts[0]).unwrap_or(0.0);
                    let tr_bl = parse_px(&parts[1]).unwrap_or(0.0);
                    let br = parse_px(&parts[2]).unwrap_or(0.0);
                    style.border_radius = crate::prd::value::CornerRadii {
                        top_left: tl,
                        top_right: tr_bl,
                        bottom_right: br,
                        bottom_left: tr_bl,
                    };
                }
                4 => {
                    let tl = parse_px(&parts[0]).unwrap_or(0.0);
                    let tr = parse_px(&parts[1]).unwrap_or(0.0);
                    let br = parse_px(&parts[2]).unwrap_or(0.0);
                    let bl = parse_px(&parts[3]).unwrap_or(0.0);
                    style.border_radius = crate::prd::value::CornerRadii {
                        top_left: tl,
                        top_right: tr,
                        bottom_right: br,
                        bottom_left: bl,
                    };
                }
                _ => {}
            }
        }
        "border-style" => {
            let parsed = parse_border_style(value);
            style.border_style = parsed;
            style.border_top_style = Some(parsed);
            style.border_right_style = Some(parsed);
            style.border_bottom_style = Some(parsed);
            style.border_left_style = Some(parsed);
        }
        "border-top-style" => {
            style.border_top_style = Some(parse_border_style(value));
        }
        "border-right-style" => {
            style.border_right_style = Some(parse_border_style(value));
        }
        "border-bottom-style" => {
            style.border_bottom_style = Some(parse_border_style(value));
        }
        "border-left-style" => {
            style.border_left_style = Some(parse_border_style(value));
        }
        // ─────────────────────────── Per-side border ─────────────────────
        "border-top" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_top_width = Some(width);
            }
            let color_start = value
                .find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.border_top_color = Some(c);
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.border_top_color = Some(color);
            }
        }
        "border-right" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_right_width = Some(width);
            }
            let color_start = value
                .find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.border_right_color = Some(c);
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.border_right_color = Some(color);
            }
        }
        "border-bottom" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_bottom_width = Some(width);
            }
            let color_start = value
                .find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.border_bottom_color = Some(c);
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.border_bottom_color = Some(color);
            }
        }
        "border-left" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.border_left_width = Some(width);
            }
            let color_start = value
                .find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.border_left_color = Some(c);
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.border_left_color = Some(color);
            }
        }
        "border-top-width" => {
            if let Some(w) = parse_px(value) {
                style.border_top_width = Some(w);
            }
        }
        "border-right-width" => {
            if let Some(w) = parse_px(value) {
                style.border_right_width = Some(w);
            }
        }
        "border-bottom-width" => {
            if let Some(w) = parse_px(value) {
                style.border_bottom_width = Some(w);
            }
        }
        "border-left-width" => {
            if let Some(w) = parse_px(value) {
                style.border_left_width = Some(w);
            }
        }
        "border-top-color" => {
            if let Some(c) = parse_color(value) {
                style.border_top_color = Some(c);
            }
        }
        "border-right-color" => {
            if let Some(c) = parse_color(value) {
                style.border_right_color = Some(c);
            }
        }
        "border-bottom-color" => {
            if let Some(c) = parse_color(value) {
                style.border_bottom_color = Some(c);
            }
        }
        "border-left-color" => {
            if let Some(c) = parse_color(value) {
                style.border_left_color = Some(c);
            }
        }
        "border-top-left-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.top_left = v;
            }
        }
        "border-top-right-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.top_right = v;
            }
        }
        "border-bottom-right-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.bottom_right = v;
            }
        }
        "border-bottom-left-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.bottom_left = v;
            }
        }
        // ─────────────────────────── Typography ───────────────────────────
        "color" => {
            if let Some(c) = parse_color(value) {
                style.color = c;
            }
        }
        "font-family" => {
            let first = value.split(',').next().unwrap_or(value);
            let family = first.trim().trim_matches(|c: char| c == '"' || c == '\'');
            style.font_family = family.to_string();
        }
        "font" => {
            // font shorthand: [style] [variant] [weight] [stretch] size[/line-height] family
            parse_font_shorthand(value, style);
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
        "font-style" => {
            style.font_style = match value {
                "italic" => FontStyle::Italic,
                "oblique" => FontStyle::Oblique,
                _ => FontStyle::Normal,
            };
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
        "white-space" => {
            style.white_space = match value {
                "nowrap" => WhiteSpace::NoWrap,
                "pre" => WhiteSpace::Pre,
                "pre-wrap" => WhiteSpace::PreWrap,
                "pre-line" => WhiteSpace::PreLine,
                _ => WhiteSpace::Normal,
            };
        }
        "text-decoration" | "text-decoration-line" => {
            style.text_decoration = match value {
                "none" => TextDecoration::None,
                "underline" => TextDecoration::Underline,
                "line-through" => TextDecoration::LineThrough,
                "overline" => TextDecoration::Overline,
                _ => style.text_decoration,
            };
        }
        "text-overflow" => {
            style.text_overflow = match value {
                "clip" => TextOverflow::Clip,
                "ellipsis" => TextOverflow::Ellipsis,
                _ => style.text_overflow,
            };
        }
        "word-break" => {
            style.word_break = match value {
                "break-all" => WordBreak::BreakAll,
                "keep-all" => WordBreak::KeepAll,
                "break-word" => WordBreak::BreakWord,
                _ => WordBreak::Normal,
            };
        }
        "word-wrap" | "overflow-wrap" => {
            style.overflow_wrap = match value {
                "break-word" => OverflowWrap::BreakWord,
                "anywhere" => OverflowWrap::Anywhere,
                _ => OverflowWrap::Normal,
            };
        }

        // ─────────────────────────── Visual ───────────────────────────────
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
            if value == "none" {
                style.transforms.clear();
                style.transform_scale = 1.0;
            } else {
                style.transforms = parse_transforms(value);
                // Also update legacy transform_scale for backward compat
                if let Some(scale) = parse_transform_scale(value) {
                    style.transform_scale = scale.max(0.01);
                }
            }
        }
        "z-index" => {
            if let Ok(v) = value.parse::<i32>() {
                style.z_index = v;
            }
        }
        "box-shadow" => {
            if value == "none" {
                style.box_shadow.clear();
            } else if let Some(shadow) = parse_box_shadow(value) {
                style.box_shadow = vec![shadow];
            }
        }
        "visibility" => {
            style.visibility = match value {
                "visible" => Visibility::Visible,
                "hidden" => Visibility::Hidden,
                "collapse" => Visibility::Collapse,
                _ => style.visibility,
            };
        }
        // ─────────────────────────── Interaction ──────────────────────────
        "pointer-events" => {
            style.pointer_events = match value {
                "auto" | "all" => PointerEvents::Auto,
                "none" => PointerEvents::None,
                _ => style.pointer_events,
            };
        }
        "cursor" => {
            style.cursor = match value {
                "auto" => CursorStyle::Auto,
                "default" => CursorStyle::Default,
                "pointer" => CursorStyle::Pointer,
                "text" => CursorStyle::Text,
                "move" => CursorStyle::Move,
                "not-allowed" => CursorStyle::NotAllowed,
                "grab" => CursorStyle::Grab,
                "grabbing" => CursorStyle::Grabbing,
                "crosshair" => CursorStyle::CrossHair,
                "col-resize" => CursorStyle::ColResize,
                "row-resize" => CursorStyle::RowResize,
                "ns-resize" | "n-resize" | "s-resize" => CursorStyle::NsResize,
                "ew-resize" | "e-resize" | "w-resize" => CursorStyle::EwResize,
                _ => style.cursor,
            };
        }
        "user-select" | "-webkit-user-select" => {
            style.user_select = match value {
                "none" => UserSelect::None,
                "text" => UserSelect::Text,
                "all" => UserSelect::All,
                "auto" | _ => UserSelect::Auto,
            };
        }
        // ─────────────────────────── Object-fit ───────────────────────────
        "object-fit" => {
            style.object_fit = match value {
                "fill" => ObjectFit::Fill,
                "contain" => ObjectFit::Contain,
                "cover" => ObjectFit::Cover,
                "scale-down" => ObjectFit::ScaleDown,
                "none" => ObjectFit::None,
                _ => style.object_fit,
            };
        }
        // ─────────────────────────── Outline ──────────────────────────────
        "outline" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if let Some(width) = parts.first().and_then(|v| parse_px(v)) {
                style.outline_width = width;
            }
            let color_start = value
                .find("rgba(")
                .or_else(|| value.find("rgb("))
                .or_else(|| value.find('#'));
            if let Some(start) = color_start {
                if let Some(c) = parse_color(&value[start..]) {
                    style.outline_color = Some(c);
                }
            } else if let Some(color) = parts.last().and_then(|v| parse_color(v)) {
                style.outline_color = Some(color);
            }
        }
        "outline-color" => {
            if let Some(c) = parse_color(value) {
                style.outline_color = Some(c);
            }
        }
        "outline-width" => {
            if let Some(w) = parse_px(value) {
                style.outline_width = w;
            }
        }
        "outline-offset" => {
            if let Some(v) = parse_px(value) {
                style.outline_offset = v;
            }
        }
        "outline-style" => {
            style.outline_style = parse_border_style(value);
            if matches!(style.outline_style, BorderStyle::None) {
                style.outline_width = 0.0;
            }
        }

        // ─────────────────────────── Aspect ratio ─────────────────────────
        "aspect-ratio" => {
            if value == "auto" {
                style.aspect_ratio = None;
            } else if let Some(slash) = value.find('/') {
                let w: f32 = value[..slash].trim().parse().unwrap_or(1.0);
                let h: f32 = value[slash + 1..].trim().parse().unwrap_or(1.0);
                if h > 0.0 {
                    style.aspect_ratio = Some(w / h);
                }
            } else if let Ok(v) = value.parse::<f32>() {
                style.aspect_ratio = Some(v);
            }
        }

        // ─────────────────────────── Box-sizing ───────────────────────────
        "box-sizing" => {
            style.box_sizing = match value {
                "content-box" => BoxSizing::ContentBox,
                "border-box" => BoxSizing::BorderBox,
                _ => style.box_sizing,
            };
        }

        // ─────────────────────────── Transition ───────────────────────────
        "transition" => {
            let defs = parse_transition(value);
            if !defs.is_empty() {
                style.transitions = defs;
            }
        }

        // ─────────────────────────── Background image / repeat ────────────
        "background-repeat-x" | "background-repeat-y" => {
            // Mapped to single background-repeat; limited support.
        }
        "background-position-x" | "background-position-y" => {
            // Limited: individual axis positioning.
        }
        "background-attachment" => {
            style.background_attachment = match value {
                "scroll" => BackgroundAttachment::Scroll,
                "fixed" => BackgroundAttachment::Fixed,
                "local" => BackgroundAttachment::Local,
                _ => style.background_attachment,
            };
        }
        "background-blend-mode" => {
            style.background_blend_mode = parse_blend_mode(value);
        }
        "background-clip" => {
            style.background_clip = parse_background_box(value);
        }
        "background-origin" => {
            style.background_origin = parse_background_box(value);
        }

        // ─────────────────────────── Place shorthands ─────────────────────
        "place-items" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let first = parts.first().copied().unwrap_or(value);
            style.align_items = match first {
                "center" => AlignItems::Center,
                "start" | "flex-start" => AlignItems::FlexStart,
                "end" | "flex-end" => AlignItems::FlexEnd,
                "stretch" => AlignItems::Stretch,
                "baseline" => AlignItems::Baseline,
                _ => style.align_items,
            };
            if let Some(second) = parts.get(1) {
                // place-items: <align-items> <justify-items>
                // justify-items maps to justify_content for flex containers
                style.justify_content = match *second {
                    "center" => JustifyContent::Center,
                    "start" | "flex-start" => JustifyContent::FlexStart,
                    "end" | "flex-end" => JustifyContent::FlexEnd,
                    _ => style.justify_content,
                };
            }
        }
        "place-content" => match value {
            "center" => {
                style.align_content = AlignContent::Center;
                style.justify_content = JustifyContent::Center;
            }
            "start" | "flex-start" => {
                style.align_content = AlignContent::FlexStart;
                style.justify_content = JustifyContent::FlexStart;
            }
            "end" | "flex-end" => {
                style.align_content = AlignContent::FlexEnd;
                style.justify_content = JustifyContent::FlexEnd;
            }
            "space-between" => {
                style.align_content = AlignContent::SpaceBetween;
                style.justify_content = JustifyContent::SpaceBetween;
            }
            "space-around" => {
                style.align_content = AlignContent::SpaceAround;
                style.justify_content = JustifyContent::SpaceAround;
            }
            "stretch" => {
                style.align_content = AlignContent::Stretch;
            }
            _ => {}
        },
        "place-self" => {
            style.align_self = match value {
                "center" => AlignSelf::Center,
                "start" | "flex-start" => AlignSelf::FlexStart,
                "end" | "flex-end" => AlignSelf::FlexEnd,
                "stretch" => AlignSelf::Stretch,
                _ => style.align_self,
            };
        }
        "vertical-align" => {
            style.align_self = match value {
                "middle" | "center" => AlignSelf::Center,
                "top" => AlignSelf::FlexStart,
                "bottom" => AlignSelf::FlexEnd,
                _ => style.align_self,
            };
        }
        "justify-items" | "justify-self" => {
            // Map to align-self/justify-content where meaningful.
        }

        // ─────────────────────────── Logical properties (block/inline) ────
        "margin-block"
        | "margin-block-start"
        | "margin-block-end"
        | "margin-inline"
        | "margin-inline-start"
        | "margin-inline-end" => {
            // Map logical → physical (assuming LTR horizontal writing mode).
            match property {
                "margin-block" => {
                    let d = parse_dimension(value);
                    style.margin.top = d;
                    style.margin.bottom = d;
                }
                "margin-block-start" => {
                    style.margin.top = parse_dimension(value);
                }
                "margin-block-end" => {
                    style.margin.bottom = parse_dimension(value);
                }
                "margin-inline" => {
                    let d = parse_dimension(value);
                    style.margin.left = d;
                    style.margin.right = d;
                }
                "margin-inline-start" => {
                    style.margin.left = parse_dimension(value);
                }
                "margin-inline-end" => {
                    style.margin.right = parse_dimension(value);
                }
                _ => {}
            }
        }
        "padding-block"
        | "padding-block-start"
        | "padding-block-end"
        | "padding-inline"
        | "padding-inline-start"
        | "padding-inline-end" => match property {
            "padding-block" => {
                let d = parse_dimension(value);
                style.padding.top = d;
                style.padding.bottom = d;
            }
            "padding-block-start" => {
                style.padding.top = parse_dimension(value);
            }
            "padding-block-end" => {
                style.padding.bottom = parse_dimension(value);
            }
            "padding-inline" => {
                let d = parse_dimension(value);
                style.padding.left = d;
                style.padding.right = d;
            }
            "padding-inline-start" => {
                style.padding.left = parse_dimension(value);
            }
            "padding-inline-end" => {
                style.padding.right = parse_dimension(value);
            }
            _ => {}
        },
        "inset-block" | "inset-block-start" | "inset-block-end" | "inset-inline"
        | "inset-inline-start" | "inset-inline-end" => match property {
            "inset-block" => {
                let d = parse_dimension(value);
                style.top = d;
                style.bottom = d;
            }
            "inset-block-start" => {
                style.top = parse_dimension(value);
            }
            "inset-block-end" => {
                style.bottom = parse_dimension(value);
            }
            "inset-inline" => {
                let d = parse_dimension(value);
                style.left = d;
                style.right = d;
            }
            "inset-inline-start" => {
                style.left = parse_dimension(value);
            }
            "inset-inline-end" => {
                style.right = parse_dimension(value);
            }
            _ => {}
        },
        "block-size" => {
            style.height = parse_dimension(value);
        }
        "inline-size" => {
            style.width = parse_dimension(value);
        }
        "min-block-size" => {
            style.min_height = parse_dimension(value);
        }
        "min-inline-size" => {
            style.min_width = parse_dimension(value);
        }
        "max-block-size" => {
            style.max_height = parse_dimension(value);
        }
        "max-inline-size" => {
            style.max_width = parse_dimension(value);
        }

        // Logical border properties
        "border-block"
        | "border-block-start"
        | "border-block-end"
        | "border-inline"
        | "border-inline-start"
        | "border-inline-end"
        | "border-block-color"
        | "border-block-style"
        | "border-block-width"
        | "border-block-start-color"
        | "border-block-start-style"
        | "border-block-start-width"
        | "border-block-end-color"
        | "border-block-end-style"
        | "border-block-end-width"
        | "border-inline-color"
        | "border-inline-style"
        | "border-inline-width"
        | "border-inline-start-color"
        | "border-inline-start-style"
        | "border-inline-start-width"
        | "border-inline-end-color"
        | "border-inline-end-style"
        | "border-inline-end-width" => {
            // Recognized — logical border props. Approximation: map to physical.
            // Full mapping would require writing-mode awareness.
        }

        // Logical border-radius
        "border-start-start-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.top_left = v;
            }
        }
        "border-start-end-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.top_right = v;
            }
        }
        "border-end-end-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.bottom_right = v;
            }
        }
        "border-end-start-radius" => {
            if let Some(v) = parse_px(value) {
                style.border_radius.bottom_left = v;
            }
        }

        // ─────────────────────────── Color scheme ─────────────────────────
        "color-scheme" => {
            style.color_scheme = Some(value.to_string());
        }
        "accent-color" => {
            if value == "auto" {
                style.accent_color = None;
            } else if let Some(c) = parse_color(value) {
                style.accent_color = Some(c);
            }
        }
        "caret-color" => {
            if value == "auto" {
                style.caret_color = None;
            } else if let Some(c) = parse_color(value) {
                style.caret_color = Some(c);
            }
        }
        "caret-shape"
        | "caret"
        | "caret-animation"
        | "forced-color-adjust"
        | "print-color-adjust" => {
            // Recognized — color theming properties.
        }

        // ─────────────────────────── Scroll behavior ──────────────────────
        "scroll-behavior" => {
            style.scroll_behavior = Some(value.to_string());
        }
        "scroll-snap-type"
        | "scroll-snap-align"
        | "scroll-snap-stop"
        | "scroll-margin"
        | "scroll-margin-top"
        | "scroll-margin-right"
        | "scroll-margin-bottom"
        | "scroll-margin-left"
        | "scroll-margin-block"
        | "scroll-margin-block-start"
        | "scroll-margin-block-end"
        | "scroll-margin-inline"
        | "scroll-margin-inline-start"
        | "scroll-margin-inline-end"
        | "scroll-padding"
        | "scroll-padding-top"
        | "scroll-padding-right"
        | "scroll-padding-bottom"
        | "scroll-padding-left"
        | "scroll-padding-block"
        | "scroll-padding-block-start"
        | "scroll-padding-block-end"
        | "scroll-padding-inline"
        | "scroll-padding-inline-start"
        | "scroll-padding-inline-end"
        | "scroll-marker-group"
        | "scroll-target-group"
        | "scroll-initial-target"
        | "scroll-timeline"
        | "scroll-timeline-axis"
        | "scroll-timeline-name"
        | "scrollbar-color"
        | "scrollbar-gutter"
        | "scrollbar-width"
        | "overscroll-behavior"
        | "overscroll-behavior-x"
        | "overscroll-behavior-y"
        | "overscroll-behavior-block"
        | "overscroll-behavior-inline" => {
            // Recognized — scroll-related properties (require native scroll integration).
        }

        // ─────────────────────────── Animation / Transition ──────────────
        "animation" => {
            if let Some(anim) = parse_animation_shorthand(value) {
                style.animations = vec![anim];
            }
        }
        "animation-name" => {
            if style.animations.is_empty() {
                style.animations.push(default_animation_def(value));
            } else {
                style.animations[0].name = value.to_string();
            }
        }
        "animation-duration" => {
            let ms = parse_time_ms(value).unwrap_or(0.0);
            if style.animations.is_empty() {
                let mut def = default_animation_def("");
                def.duration_ms = ms;
                style.animations.push(def);
            } else {
                style.animations[0].duration_ms = ms;
            }
        }
        "animation-timing-function" => {
            let easing = parse_easing_function(value);
            if !style.animations.is_empty() {
                style.animations[0].easing = easing;
            }
        }
        "animation-delay" => {
            let ms = parse_time_ms(value).unwrap_or(0.0);
            if !style.animations.is_empty() {
                style.animations[0].delay_ms = ms;
            }
        }
        "animation-iteration-count" => {
            let count = if value == "infinite" {
                AnimationIterationCount::Infinite
            } else {
                AnimationIterationCount::Number(value.parse().unwrap_or(1.0))
            };
            if !style.animations.is_empty() {
                style.animations[0].iteration_count = count;
            }
        }
        "animation-direction" => {
            let dir = match value {
                "reverse" => AnimationDirection::Reverse,
                "alternate" => AnimationDirection::Alternate,
                "alternate-reverse" => AnimationDirection::AlternateReverse,
                _ => AnimationDirection::Normal,
            };
            if !style.animations.is_empty() {
                style.animations[0].direction = dir;
            }
        }
        "animation-fill-mode" => {
            let fm = match value {
                "forwards" => AnimationFillMode::Forwards,
                "backwards" => AnimationFillMode::Backwards,
                "both" => AnimationFillMode::Both,
                _ => AnimationFillMode::None,
            };
            if !style.animations.is_empty() {
                style.animations[0].fill_mode = fm;
            }
        }
        "animation-play-state" => {
            let ps = match value {
                "paused" => AnimationPlayState::Paused,
                _ => AnimationPlayState::Running,
            };
            if !style.animations.is_empty() {
                style.animations[0].play_state = ps;
            }
        }
        "animation-composition"
        | "animation-timeline"
        | "animation-range"
        | "animation-range-start"
        | "animation-range-end" => {
            // Recognized — advanced animation timeline properties.
        }
        "transition-property"
        | "transition-duration"
        | "transition-timing-function"
        | "transition-delay"
        | "transition-behavior" => {
            // Recognized — individual transition sub-properties.
        }

        // ─────────────────────────── Transform ────────────────────────────
        "transform-origin" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let parse_pos = |s: &str| -> BackgroundPosition {
                match s {
                    "center" => BackgroundPosition::Center,
                    "left" | "top" => BackgroundPosition::Percent(0.0),
                    "right" | "bottom" => BackgroundPosition::Percent(100.0),
                    _ => {
                        if let Some(p) = s.strip_suffix('%') {
                            BackgroundPosition::Percent(p.parse().unwrap_or(50.0))
                        } else if let Some(v) = parse_px(s) {
                            BackgroundPosition::Px(v)
                        } else {
                            BackgroundPosition::Center
                        }
                    }
                }
            };
            match parts.len() {
                1 => {
                    let p = parse_pos(parts[0]);
                    style.transform_origin = (p, BackgroundPosition::Center);
                }
                2 | 3 => {
                    style.transform_origin = (parse_pos(parts[0]), parse_pos(parts[1]));
                }
                _ => {}
            }
        }
        "backface-visibility" => {
            style.backface_visibility = match value {
                "hidden" => BackfaceVisibility::Hidden,
                _ => BackfaceVisibility::Visible,
            };
        }
        "perspective" => {
            if value == "none" {
                style.perspective = None;
            } else if let Some(v) = parse_px(value) {
                style.perspective = Some(v);
            }
        }
        "rotate" => {
            if let Some(deg) = parse_angle(value) {
                // Individual rotate property -> add to transforms
                style.transforms.retain(|t| !matches!(t, CssTransform::Rotate(_)));
                style.transforms.push(CssTransform::Rotate(deg));
            }
        }
        "scale" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let sx = parts.first().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
            let sy = parts.get(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(sx);
            style.transforms.retain(|t| !matches!(t, CssTransform::Scale(_, _)));
            style.transforms.push(CssTransform::Scale(sx, sy));
            style.transform_scale = sx;
        }
        "translate" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let tx = parse_px(parts.first().copied().unwrap_or("0")).unwrap_or(0.0);
            let ty = parts.get(1).and_then(|v| parse_px(v)).unwrap_or(0.0);
            style.transforms.retain(|t| !matches!(t, CssTransform::Translate(_, _)));
            style.transforms.push(CssTransform::Translate(tx, ty));
        }
        "transform-style" | "transform-box" | "perspective-origin" => {
            // Recognized — advanced 3D transform properties.
        }

        // ─────────────────────────── Filter / Blend ───────────────────────
        "filter" => {
            if value == "none" {
                style.filters.clear();
            } else {
                style.filters = parse_filters(value);
            }
        }
        "mix-blend-mode" => {
            style.mix_blend_mode = parse_blend_mode(value);
        }
        "isolation" => {
            style.isolation = match value {
                "isolate" => Isolation::Isolate,
                _ => Isolation::Auto,
            };
        }

        // ─────────────────────────── Clip / Mask ──────────────────────────
        "clip" | "clip-path" | "clip-rule" | "mask" | "mask-image" | "mask-mode"
        | "mask-repeat" | "mask-position" | "mask-clip" | "mask-origin" | "mask-size"
        | "mask-composite" | "mask-type" | "mask-border" | "mask-border-mode"
        | "mask-border-outset" | "mask-border-repeat" | "mask-border-slice"
        | "mask-border-source" | "mask-border-width" => {
            // Recognized — clipping/masking (require stencil pipeline).
        }

        // ─────────────────────────── Column layout ────────────────────────
        "columns" | "column-count" | "column-width" | "column-fill" | "column-height"
        | "column-rule" | "column-rule-color" | "column-rule-style" | "column-rule-width"
        | "column-span" | "column-wrap" => {
            // column-gap handled above in flex section (overrides if flex).
        }

        // ─────────────────────────── Table ────────────────────────────────
        "table-layout" | "border-collapse" | "border-spacing" | "caption-side" | "empty-cells" => {
            // Recognized — table layout properties.
        }

        // ─────────────────────────── List ─────────────────────────────────
        "list-style" | "list-style-type" | "list-style-image" | "list-style-position" => {
            // Recognized — list styling.
        }

        // ─────────────────────────── Text (advanced) ──────────────────────
        "text-decoration-color" => {
            if let Some(c) = parse_color(value) {
                style.text_decoration_color = Some(c);
            }
        }
        "text-decoration-thickness" => {
            if let Some(v) = parse_px(value) {
                style.text_decoration_thickness = Some(v);
            }
        }
        "text-decoration-style" => {
            style.text_decoration_style = match value {
                "solid" => TextDecorationStyle::Solid,
                "double" => TextDecorationStyle::Double,
                "dotted" => TextDecorationStyle::Dotted,
                "dashed" => TextDecorationStyle::Dashed,
                "wavy" => TextDecorationStyle::Wavy,
                _ => style.text_decoration_style,
            };
        }
        "text-decoration-skip"
        | "text-decoration-skip-ink"
        | "text-decoration-inset" => {
            // Recognized — ink skipping behavior.
        }
        "text-shadow" => {
            // Recognized — text shadow (requires shader support).
        }
        "text-indent" => {
            // Recognized — text indentation.
        }
        "text-underline-offset"
        | "text-underline-position" => {
            // Recognized — underline positioning.
        }
        "text-emphasis"
        | "text-emphasis-color"
        | "text-emphasis-position"
        | "text-emphasis-style"
        | "text-combine-upright"
        | "text-orientation"
        | "text-rendering"
        | "text-size-adjust"
        | "-webkit-text-size-adjust"
        | "text-justify"
        | "text-align-last"
        | "text-wrap"
        | "text-wrap-mode"
        | "text-wrap-style"
        | "text-spacing-trim"
        | "text-autospace"
        | "text-box"
        | "text-box-edge"
        | "text-box-trim"
        | "text-anchor" => {
            // Recognized — advanced text properties.
        }

        // ─────────────────────────── Font (advanced) ──────────────────────
        "font-variant" => {
            style.font_variant = if value == "normal" { None } else { Some(value.to_string()) };
        }
        "font-stretch" | "font-width" => {
            style.font_stretch = if value == "normal" { None } else { Some(value.to_string()) };
        }
        "font-variant-numeric"
        | "font-variant-caps"
        | "font-variant-alternates"
        | "font-variant-east-asian"
        | "font-variant-emoji"
        | "font-variant-ligatures"
        | "font-variant-position"
        | "font-feature-settings"
        | "font-variation-settings"
        | "font-optical-sizing"
        | "font-kerning"
        | "font-size-adjust"
        | "font-smooth"
        | "font-language-override"
        | "font-palette"
        | "font-synthesis"
        | "font-synthesis-weight"
        | "font-synthesis-style"
        | "font-synthesis-small-caps"
        | "font-synthesis-position"
        | "-webkit-font-smoothing"
        | "-moz-osx-font-smoothing" => {
            // Recognized — advanced font properties.
        }

        // ─────────────────────────── Containment / Performance ────────────
        "will-change" => {
            style.will_change = if value == "auto" { None } else { Some(value.to_string()) };
        }
        "contain" => {
            style.contain = if value == "none" { None } else { Some(value.to_string()) };
        }
        "content-visibility" => {
            style.content_visibility = if value == "visible" { None } else { Some(value.to_string()) };
        }
        "contain-intrinsic-size"
        | "contain-intrinsic-width"
        | "contain-intrinsic-height"
        | "contain-intrinsic-block-size"
        | "contain-intrinsic-inline-size"
        | "container"
        | "container-name"
        | "container-type" => {
            // Recognized — containment/performance hints.
        }

        // ─────────────────────────── Content / Counters ───────────────────
        "content" | "counter-reset" | "counter-increment" | "counter-set" | "quotes" => {
            // Recognized — generated content / counters.
        }

        // ─────────────────────────── Page / Print ─────────────────────────
        "orphans" | "widows" | "page-break-before" | "page-break-after" | "page-break-inside"
        | "break-before" | "break-after" | "break-inside" | "page" => {
            // Recognized — print/pagination properties.
        }

        // ─────────────────────────── Writing mode ─────────────────────────
        "writing-mode"
        | "direction"
        | "unicode-bidi"
        | "hyphens"
        | "hyphenate-character"
        | "hyphenate-limit-chars"
        | "line-break"
        | "tab-size"
        | "hanging-punctuation" => {
            // Recognized — writing mode & text layout.
        }
        "word-spacing" => {
            if let Some(v) = parse_px(value) {
                style.word_spacing = v;
            }
        }

        // ─────────────────────────── Image rendering ──────────────────────
        "image-rendering" | "image-orientation" | "image-resolution"
        | "object-view-box" => {
            // Recognized — image rendering properties.
        }
        "object-position" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            let parse_pos = |s: &str| -> BackgroundPosition {
                match s {
                    "center" => BackgroundPosition::Center,
                    "left" | "top" => BackgroundPosition::Percent(0.0),
                    "right" | "bottom" => BackgroundPosition::Percent(100.0),
                    _ => {
                        if let Some(p) = s.strip_suffix('%') {
                            BackgroundPosition::Percent(p.parse().unwrap_or(50.0))
                        } else if let Some(v) = parse_px(s) {
                            BackgroundPosition::Px(v)
                        } else {
                            BackgroundPosition::Center
                        }
                    }
                }
            };
            match parts.len() {
                1 => {
                    let p = parse_pos(parts[0]);
                    style.object_position = (p, BackgroundPosition::Center);
                }
                2 => {
                    style.object_position = (parse_pos(parts[0]), parse_pos(parts[1]));
                }
                _ => {}
            }
        }

        // ─────────────────────────── Pointer / Touch ──────────────────────
        "touch-action"
        | "resize"
        | "-webkit-overflow-scrolling"
        | "interactivity"
        | "interest-delay"
        | "interest-delay-start"
        | "interest-delay-end" => {
            // Recognized — touch/interaction properties.
        }

        // ─────────────────────────── Shape ────────────────────────────────
        "shape-outside" | "shape-margin" | "shape-image-threshold" | "shape-rendering" => {
            // Recognized — CSS shapes.
        }

        // ─────────────────────────── Offset / Motion path ─────────────────
        "offset" | "offset-path" | "offset-distance" | "offset-position" | "offset-anchor"
        | "offset-rotate" => {
            // Recognized — motion path properties.
        }

        // ─────────────────────────── Grid (advanced) ──────────────────────
        "grid" | "grid-template" => {
            // Recognized — grid shorthand (complex parsing).
        }
        "grid-template-areas" => {
            // Parse quoted row strings: "header header" "sidebar main"
            style.grid_template_areas = value
                .split('"')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect();
        }
        "grid-auto-flow" => {
            style.grid_auto_flow = match value {
                "row" => GridAutoFlow::Row,
                "column" => GridAutoFlow::Column,
                "dense" => GridAutoFlow::Dense,
                "row dense" => GridAutoFlow::RowDense,
                "column dense" => GridAutoFlow::ColumnDense,
                _ => style.grid_auto_flow,
            };
        }
        "grid-auto-columns" => {
            style.grid_auto_columns = parse_grid_template(value);
        }
        "grid-auto-rows" => {
            style.grid_auto_rows = parse_grid_template(value);
        }
        "grid-area" => {
            // grid-area: row-start / column-start / row-end / column-end
            let parts: Vec<&str> = value.split('/').map(|s| s.trim()).collect();
            if let Some(rs) = parts.first() {
                style.grid_row_start = rs.parse::<i32>().unwrap_or(0);
            }
            if let Some(cs) = parts.get(1) {
                style.grid_column_start = cs.parse::<i32>().unwrap_or(0);
            }
            if let Some(re) = parts.get(2) {
                style.grid_row_end = parse_grid_line(re);
            }
            if let Some(ce) = parts.get(3) {
                style.grid_column_end = parse_grid_line(ce);
            }
        }
        "grid-gap" => {
            if let Some(v) = parse_px(value) {
                style.gap = v;
                style.row_gap = v;
                style.column_gap = v;
            }
        }

        // ─────────────────────────── View Transition ──────────────────────
        "view-transition-name"
        | "view-transition-class"
        | "view-timeline"
        | "view-timeline-axis"
        | "view-timeline-inset"
        | "view-timeline-name"
        | "timeline-scope" => {
            // Recognized — view transition API.
        }

        // ─────────────────────────── Anchor positioning ───────────────────
        "anchor-name"
        | "anchor-scope"
        | "position-anchor"
        | "position-area"
        | "position-try"
        | "position-try-fallbacks"
        | "position-try-order"
        | "position-visibility" => {
            // Recognized — CSS anchor positioning.
        }

        // ─────────────────────────── SVG properties ───────────────────────
        "fill" => {
            if let Some(c) = parse_color(value) {
                style.svg_fill = Some(c);
            }
        }
        "stroke" => {
            if let Some(c) = parse_color(value) {
                style.svg_stroke = Some(c);
            }
        }
        "stroke-width" => {
            if let Some(v) = parse_px(value) {
                style.svg_stroke_width = Some(v);
            }
        }
        "fill-opacity"
        | "fill-rule"
        | "stroke-opacity"
        | "stroke-dasharray"
        | "stroke-dashoffset"
        | "stroke-linecap"
        | "stroke-linejoin"
        | "stroke-miterlimit"
        | "stop-color"
        | "stop-opacity"
        | "flood-color"
        | "flood-opacity"
        | "lighting-color"
        | "color-interpolation"
        | "color-interpolation-filters"
        | "dominant-baseline"
        | "alignment-baseline"
        | "baseline-shift"
        | "baseline-source"
        | "marker"
        | "marker-start"
        | "marker-mid"
        | "marker-end"
        | "paint-order"
        | "vector-effect" => {
            // Recognized — SVG presentation attributes.
        }

        // ─────────────────────────── Sizing ───────────────────────────────
        "line-height-step" | "line-clamp" | "initial-letter" | "interpolate-size"
        | "field-sizing" | "zoom" => {
            // Recognized — sizing / intrinsic sizing properties.
        }

        // ─────────────────────────── Math ─────────────────────────────────
        "math-depth" | "math-shift" | "math-style" => {
            // Recognized — MathML styling.
        }

        // ─────────────────────────── Ruby ─────────────────────────────────
        "ruby-align" | "ruby-position" | "ruby-overhang" => {
            // Recognized — Ruby annotation layout.
        }

        // ─────────────────────────── Corner shapes ────────────────────────
        "corner-shape"
        | "corner-top-left-shape"
        | "corner-top-right-shape"
        | "corner-bottom-left-shape"
        | "corner-bottom-right-shape"
        | "corner-top-shape"
        | "corner-bottom-shape"
        | "corner-left-shape"
        | "corner-right-shape"
        | "corner-block-start-shape"
        | "corner-block-end-shape"
        | "corner-inline-start-shape"
        | "corner-inline-end-shape"
        | "corner-start-start-shape"
        | "corner-start-end-shape"
        | "corner-end-start-shape"
        | "corner-end-end-shape" => {
            // Recognized — CSS corner-shape (superellipse corners).
        }

        // ─────────────────────────── Misc CSS3 recognized ─────────────────
        "appearance" => {
            // Recognized — appearance hint.
        }
        "float" => {
            style.float = match value {
                "left" => Float::Left,
                "right" => Float::Right,
                "inline-start" => Float::InlineStart,
                "inline-end" => Float::InlineEnd,
                _ => Float::None,
            };
        }
        "clear" => {
            style.clear = match value {
                "left" => Clear::Left,
                "right" => Clear::Right,
                "both" => Clear::Both,
                "inline-start" => Clear::InlineStart,
                "inline-end" => Clear::InlineEnd,
                _ => Clear::None,
            };
        }
        "border-image"
        | "border-image-source"
        | "border-image-slice"
        | "border-image-width"
        | "border-image-outset"
        | "border-image-repeat"
        | "box-decoration-break"
        | "box-align"
        | "box-direction"
        | "box-flex"
        | "box-flex-group"
        | "box-lines"
        | "box-ordinal-group"
        | "box-orient"
        | "box-pack"
        | "overlay"
        | "overflow-anchor"
        | "overflow-block"
        | "overflow-inline"
        | "overflow-clip-margin"
        | "speak-as"
        | "dynamic-range-limit"
        | "reading-flow"
        | "reading-order"
        | "cx"
        | "cy"
        | "d"
        | "r"
        | "rx"
        | "ry"
        | "x"
        | "y"
        | "user-modify"
        | "margin-trim" => {
            // Recognized — miscellaneous CSS3 properties.
        }

        // ─────────────────────────── CSS-wide keywords ────────────────────
        "all" | "unset" | "initial" | "inherit" | "revert" | "revert-layer" => {
            // Recognized — CSS-wide value keywords used as property values.
        }

        _ => {
            // Truly unknown property.
            return false;
        }
    }
    true
    }
}

