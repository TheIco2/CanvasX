// canvasx-runtime/src/compiler/html.rs
//
// HTML subset parser for the CanvasX Runtime.
// Converts restricted HTML into CXRD nodes.
//
// Supported elements:
//   div, span, p, h1–h6, img, button, input, label, svg, path, section
//   Custom: <data-bind> for live system data.
//
// Attributes: class, id, style (inline), data-*, src, alt

use crate::cxrd::document::{CxrdDocument, SceneType};
use crate::cxrd::node::{CxrdNode, NodeKind, ImageFit, NodeId, EventBinding, EventAction};
use crate::cxrd::input::{InputKind, TextInputType, ButtonVariant, CheckboxStyle};
use crate::cxrd::style::{ComputedStyle, Display, FlexDirection, FontWeight, TextAlign};
use crate::cxrd::value::Dimension;
use crate::compiler::css::{parse_css, apply_property, parse_color, CssRule, CompoundSelector};
use std::collections::HashMap;
use std::path::Path;

/// A collected script block from the HTML source.
#[derive(Debug, Clone)]
pub struct ScriptBlock {
    /// Inline script text content, or empty if external.
    pub content: String,
    /// External script src path (relative to HTML file).
    pub src: Option<String>,
}

// Thread-local storage for collecting scripts during tokenization.
std::thread_local! {
    static COLLECTED_SCRIPTS: std::cell::RefCell<Vec<ScriptBlock>> = std::cell::RefCell::new(Vec::new());
}

/// Compile an HTML file + CSS into a CXRD document.
///
/// `html_source` — the HTML content.
/// `css_source` — the CSS content (from <link> or <style>).
/// `asset_dir` — base directory for resolving local asset paths.
/// `scene_type` — what kind of scene this is (wallpaper, widget, etc.).
pub fn compile_html(
    html_source: &str,
    css_source: &str,
    name: &str,
    scene_type: SceneType,
    _asset_dir: Option<&Path>,
) -> anyhow::Result<(CxrdDocument, Vec<ScriptBlock>, Vec<CssRule>)> {
    let mut doc = CxrdDocument::new(name, scene_type);

    // 1. Parse CSS rules.
    let rules = parse_css(css_source);

    // 2. Extract CSS custom properties (:root variables).
    let mut variables: HashMap<String, String> = HashMap::new();
    for rule in &rules {
        if rule.selector == ":root" {
            for (prop, val) in &rule.declarations {
                if prop.starts_with("--") {
                    variables.insert(prop.clone(), val.clone());
                }
            }
        }
    }
    doc.variables = variables.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    // Collect script blocks from the HTML.
    // Clear thread-local storage before tokenization.
    COLLECTED_SCRIPTS.with(|s| s.borrow_mut().clear());

    // 2b. Extract document background from body/html CSS rules.
    extract_document_background(&mut doc, &rules, &variables);

    // 3. Parse HTML into node tree.
    let tokens = tokenize_html(html_source);
    let (root_children, _) = build_node_tree(&tokens, 0);

    // 3b. Retrieve collected scripts.
    let scripts = COLLECTED_SCRIPTS.with(|s| s.borrow().clone());

    // 4. Add nodes to document and apply CSS.
    // Pass ancestor chain for descendant selector matching.
    let root_ancestors: Vec<AncestorInfo> = Vec::new();
    for child in root_children {
        let child_id = add_node_recursive(&mut doc, child, &rules, &variables, &root_ancestors, None);
        doc.add_child(doc.root, child_id);
    }

    // 5. Apply root styles.
    if let Some(root) = doc.get_node_mut(doc.root) {
        apply_rules_to_node(root, &rules, &variables);
    }

    // 6. CSS inheritance pass — propagate inheritable properties
    //    (color, font-family, font-size, font-weight, line-height,
    //     letter-spacing, text-align) from parent to child nodes.
    propagate_inherited_styles(&mut doc);

    Ok((doc, scripts, rules))
}

/// Extract document background color from body/html/:root CSS rules.
///
/// The sentinel.default wallpaper uses `background: var(--bg-color)` on body,
/// which resolves to a hex color. We check body, html, and :root rules in
/// order, taking the last match (highest specificity).
fn extract_document_background(
    doc: &mut CxrdDocument,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
) {
    use crate::compiler::css::resolve_var_pub;

    let bg_selectors = ["html", "body", ":root", "html,body", "html, body"];
    for rule in rules {
        let sel = rule.selector.trim();
        // Check if selector targets html, body, or :root.
        let matches = bg_selectors.iter().any(|s| sel == *s)
            || sel.split(',').any(|part| {
                let part = part.trim();
                part == "html" || part == "body"
            });

        if !matches {
            continue;
        }

        for (prop, val) in &rule.declarations {
            if prop == "background" || prop == "background-color" {
                let resolved = resolve_var_pub(val, variables);
                if let Some(color) = parse_color(&resolved) {
                    doc.background = color;
                }
            }
        }
    }
}

/// Propagate CSS-inherited properties from parent to child nodes.
///
/// In CSS, properties like `color`, `font-family`, `font-size`, `font-weight`,
/// `line-height`, `letter-spacing`, and `text-align` are inherited.  If a child
/// node doesn't have an explicitly-set value (still at default), it should
/// inherit from its parent.
///
/// We do a depth-first traversal, carrying the parent's style down.
fn propagate_inherited_styles(doc: &mut CxrdDocument) {
    let defaults = ComputedStyle::default();
    let root_id = doc.root;

    // Collect root's inheritable props as initial values.
    let root_inherited = {
        let root = &doc.nodes[root_id as usize];
        InheritedProps::from_style(&root.style)
    };

    let children: Vec<u32> = doc.nodes[root_id as usize].children.clone();
    for child_id in children {
        propagate_recursive(doc, child_id, &root_inherited, &defaults);
    }
}

/// Inheritable CSS property bundle.
#[derive(Clone)]
struct InheritedProps {
    color: crate::cxrd::value::Color,
    font_family: String,
    font_size: f32,
    font_weight: FontWeight,
    line_height: f32,
    letter_spacing: f32,
    text_align: TextAlign,
}

impl InheritedProps {
    fn from_style(s: &ComputedStyle) -> Self {
        Self {
            color: s.color,
            font_family: s.font_family.clone(),
            font_size: s.font_size,
            font_weight: s.font_weight,
            line_height: s.line_height,
            letter_spacing: s.letter_spacing,
            text_align: s.text_align,
        }
    }
}

fn propagate_recursive(
    doc: &mut CxrdDocument,
    node_id: u32,
    parent: &InheritedProps,
    defaults: &ComputedStyle,
) {
    // Apply inherited values where the node still has the default.
    {
        let node = &mut doc.nodes[node_id as usize];
        // Color: inherit if still at the default (WHITE).
        if node.style.color == defaults.color {
            node.style.color = parent.color;
        }
        // font-family: inherit if empty (default).
        if node.style.font_family.is_empty() {
            node.style.font_family = parent.font_family.clone();
        }
        // font-size: inherit if same as default (16.0).
        if (node.style.font_size - defaults.font_size).abs() < 0.01 {
            node.style.font_size = parent.font_size;
        }
        // font-weight: inherit if default.
        if node.style.font_weight == defaults.font_weight {
            node.style.font_weight = parent.font_weight;
        }
        // line-height: inherit if default (1.5).
        if (node.style.line_height - defaults.line_height).abs() < 0.01 {
            node.style.line_height = parent.line_height;
        }
        // letter-spacing: inherit if zero (default).
        if node.style.letter_spacing.abs() < 0.001 {
            node.style.letter_spacing = parent.letter_spacing;
        }
        // text-align: inherit if default.
        if node.style.text_align == defaults.text_align {
            node.style.text_align = parent.text_align;
        }
    }

    // Build inherited props from this node's current (post-inheritance) style.
    let my_inherited = {
        let node = &doc.nodes[node_id as usize];
        InheritedProps::from_style(&node.style)
    };

    let children: Vec<u32> = doc.nodes[node_id as usize].children.clone();
    for child_id in children {
        propagate_recursive(doc, child_id, &my_inherited, defaults);
    }
}

/// A temporary parsed HTML node before adding to the document.
struct ParsedNode {
    tag: String,
    classes: Vec<String>,
    id: Option<String>,
    attributes: HashMap<String, String>,
    inline_style: String,
    text_content: Option<String>,
    children: Vec<ParsedNode>,
}

/// Tokenize HTML into a flat list of events.
#[derive(Debug)]
enum HtmlToken {
    OpenTag {
        tag: String,
        attributes: HashMap<String, String>,
        self_closing: bool,
    },
    CloseTag {
        #[allow(dead_code)]
        tag: String,
    },
    Text(String),
}

fn tokenize_html(source: &str) -> Vec<HtmlToken> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    let bytes = source.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            // Check for comment
            if pos + 3 < bytes.len() && &source[pos..pos+4] == "<!--" {
                if let Some(end) = source[pos..].find("-->") {
                    pos += end + 3;
                    continue;
                }
            }

            // Check for <!DOCTYPE ...> — skip entirely, it's not a renderable element.
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'!' {
                // Skip to the closing >.
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                if pos < bytes.len() { pos += 1; } // skip >
                continue;
            }

            // Check for close tag
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
                pos += 2;
                let tag_start = pos;
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                let tag = source[tag_start..pos].trim().to_lowercase();
                pos += 1; // skip >
                tokens.push(HtmlToken::CloseTag { tag });
                continue;
            }

            // Open tag
            pos += 1; // skip <
            let tag_start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                pos += 1;
            }
            let tag = source[tag_start..pos].trim().to_lowercase();

            // Parse attributes
            let mut attributes = HashMap::new();
            loop {
                // Skip whitespace
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                if pos >= bytes.len() || bytes[pos] == b'>' || bytes[pos] == b'/' {
                    break;
                }

                // Attribute name
                let attr_start = pos;
                while pos < bytes.len() && bytes[pos] != b'=' && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                    pos += 1;
                }
                let attr_name = source[attr_start..pos].to_lowercase();

                if pos < bytes.len() && bytes[pos] == b'=' {
                    pos += 1; // skip =
                    // Attribute value
                    let val = if pos < bytes.len() && (bytes[pos] == b'"' || bytes[pos] == b'\'') {
                        let quote = bytes[pos];
                        pos += 1;
                        let val_start = pos;
                        while pos < bytes.len() && bytes[pos] != quote {
                            pos += 1;
                        }
                        let val = source[val_start..pos].to_string();
                        if pos < bytes.len() { pos += 1; } // skip closing quote
                        val
                    } else {
                        let val_start = pos;
                        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' {
                            pos += 1;
                        }
                        source[val_start..pos].to_string()
                    };
                    attributes.insert(attr_name, val);
                } else {
                    attributes.insert(attr_name, String::new());
                }
            }

            let self_closing = pos < bytes.len() && bytes[pos] == b'/';
            if self_closing { pos += 1; }
            if pos < bytes.len() && bytes[pos] == b'>' { pos += 1; }

            // Skip <style>, <head>, <meta>, <link> tags entirely.
            // <script> tags are collected as ScriptBlock rather than skipped.
            let skip_tags = ["style", "head", "meta", "link", "title"];
            if skip_tags.contains(&tag.as_str()) {
                if !self_closing {
                    // Find closing tag
                    let close = format!("</{}>", tag);
                    if let Some(end) = source[pos..].to_lowercase().find(&close) {
                        pos += end + close.len();
                    }
                }
                continue;
            }

            // Collect <script> tags as ScriptBlock entries.
            if tag == "script" {
                if !self_closing {
                    let close = "</script>";
                    if let Some(end) = source[pos..].to_lowercase().find(close) {
                        let script_content = &source[pos..pos + end];
                        let src = attributes.get("src").cloned();
                        // Store script block in a thread-local for later retrieval.
                        COLLECTED_SCRIPTS.with(|s| {
                            s.borrow_mut().push(ScriptBlock {
                                content: script_content.trim().to_string(),
                                src,
                            });
                        });
                        pos += end + close.len();
                    }
                } else if let Some(src) = attributes.get("src") {
                    COLLECTED_SCRIPTS.with(|s| {
                        s.borrow_mut().push(ScriptBlock {
                            content: String::new(),
                            src: Some(src.clone()),
                        });
                    });
                }
                continue;
            }

            tokens.push(HtmlToken::OpenTag { tag, attributes, self_closing });
        } else {
            // Text content — collapse whitespace runs to a single space
            // but preserve boundary spaces for inline element spacing.
            let text_start = pos;
            while pos < bytes.len() && bytes[pos] != b'<' {
                pos += 1;
            }
            let raw = &source[text_start..pos];
            // Collapse internal whitespace runs to single spaces (CSS white-space: normal).
            let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            if !collapsed.is_empty() {
                // Preserve a leading space if the raw text started with whitespace
                // and a trailing space if the raw text ended with whitespace.
                // This is essential for inline element spacing (e.g. "text <strong>bold</strong>").
                let leading = raw.starts_with(char::is_whitespace) && !collapsed.is_empty();
                let trailing = raw.ends_with(char::is_whitespace) && !collapsed.is_empty();
                let mut text = String::with_capacity(collapsed.len() + 2);
                if leading { text.push(' '); }
                text.push_str(&collapsed);
                if trailing { text.push(' '); }
                tokens.push(HtmlToken::Text(text));
            }
        }
    }

    tokens
}

/// Build node tree from tokens.
fn build_node_tree(tokens: &[HtmlToken], start: usize) -> (Vec<ParsedNode>, usize) {
    let mut nodes = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match &tokens[i] {
            HtmlToken::OpenTag { tag, attributes, self_closing } => {
                let classes: Vec<String> = attributes.get("class")
                    .map(|c| c.split_whitespace().map(String::from).collect())
                    .unwrap_or_default();
                let id = attributes.get("id").cloned();
                let inline_style = attributes.get("style").cloned().unwrap_or_default();

                let mut node = ParsedNode {
                    tag: tag.clone(),
                    classes,
                    id,
                    attributes: attributes.clone(),
                    inline_style,
                    text_content: None,
                    children: Vec::new(),
                };

                if *self_closing || is_void_element(tag) {
                    i += 1;
                } else {
                    let (children, end_pos) = build_node_tree(tokens, i + 1);
                    node.children = children;
                    i = end_pos + 1; // skip past the close tag
                }

                nodes.push(node);
            }
            HtmlToken::CloseTag { .. } => {
                return (nodes, i);
            }
            HtmlToken::Text(text) => {
                nodes.push(ParsedNode {
                    tag: "#text".to_string(),
                    classes: Vec::new(),
                    id: None,
                    attributes: HashMap::new(),
                    inline_style: String::new(),
                    text_content: Some(text.clone()),
                    children: Vec::new(),
                });
                i += 1;
            }
        }
    }

    (nodes, i)
}

fn is_void_element(tag: &str) -> bool {
    matches!(tag, "img" | "br" | "hr" | "input" | "meta" | "link" | "source" | "svg" | "path" | "line" | "circle" | "rect" | "polyline" | "ellipse" | "polygon")
}

/// Info about an ancestor element, used for descendant selector matching.
#[derive(Clone)]
pub struct AncestorInfo {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub html_id: Option<String>,
}

/// Add a parsed node tree to the CXRD document.
fn add_node_recursive(
    doc: &mut CxrdDocument,
    parsed: ParsedNode,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
    ancestors: &[AncestorInfo],
    parent_style: Option<&ComputedStyle>,
) -> NodeId {
    let kind = determine_node_kind(&parsed, variables);

    // For widget elements, children are consumed by the widget (label, options, etc.)
    // and should not be added as child scene nodes.
    let skip_children = matches!(&kind,
        NodeKind::Input(InputKind::Button { .. }) |
        NodeKind::Input(InputKind::Dropdown { .. }) |
        NodeKind::Input(InputKind::TextArea { .. })
    );

    let mut style = ComputedStyle::default();

    // Tag-specific display defaults — mirrors HTML's block/inline model.
    // Inline-level tags default to flex-row so their children flow horizontally,
    // which is the closest equivalent to inline-flow in our block/flex engine.
    match parsed.tag.as_str() {
        "span" | "a" | "label" | "code" | "small" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
        }
        // Inline + bold
        "strong" | "b" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
            style.font_weight = FontWeight(700);
        }
        // Inline + italic (note: we store italic as weight 0 sentinel; see text painter)
        "em" | "i" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
            // Italic handled via tag check in text painter; no weight change.
        }
        // Heading defaults — browser UA sizes relative to 16px base.
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.67);
            style.margin.bottom = Dimension::Em(0.67);
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.83);
            style.margin.bottom = Dimension::Em(0.83);
        }
        "h3" => {
            style.font_size = 18.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        "h4" => {
            style.font_size = 16.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.33);
            style.margin.bottom = Dimension::Em(1.33);
        }
        "h5" => {
            style.font_size = 13.28;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.67);
            style.margin.bottom = Dimension::Em(1.67);
        }
        "h6" => {
            style.font_size = 10.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(2.33);
            style.margin.bottom = Dimension::Em(2.33);
        }
        // <p> uses flex-row-wrap so inline children (text, <strong>, <em>, etc.)
        // flow horizontally, approximating CSS inline formatting context.
        "p" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
            style.flex_wrap = crate::cxrd::style::FlexWrap::Wrap;
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        // data-bind custom tag default.
        "data-bind" => {
            style.display = Display::InlineBlock;
            if parsed.classes.iter().any(|c| c == "val") {
                style.flex_grow = 1.0;
            }
        }
        // data-bar custom tag default.
        "data-bar" => {
            style.display = Display::Block;
        }
        // canvas element — block-level, sized by width/height attributes.
        "canvas" => {
            style.display = Display::Block;
            // Use HTML width/height attributes as intrinsic CSS dimensions,
            // matching browser behavior where canvas elements have
            // intrinsic size from their attributes (default 300×150).
            if let Some(w) = parsed.attributes.get("width").and_then(|v| v.parse::<f32>().ok()) {
                style.width = Dimension::Px(w);
            }
            if let Some(h) = parsed.attributes.get("height").and_then(|v| v.parse::<f32>().ok()) {
                style.height = Dimension::Px(h);
            }
        }
        _ => {} // default Block
    }

    // Inherit CSS properties from parent (CSS inheritance model).
    if let Some(ps) = parent_style {
        style.color = ps.color;
        style.font_size = ps.font_size;
        style.font_family = ps.font_family.clone();
        style.font_weight = ps.font_weight;
        style.letter_spacing = ps.letter_spacing;
        style.line_height = ps.line_height;
        style.text_align = ps.text_align;
        style.text_transform = ps.text_transform;
    }

    let mut node = CxrdNode {
        id: 0, // Will be set by add_node
        tag: Some(parsed.tag.clone()),
        html_id: parsed.id.clone(),
        classes: parsed.classes.clone(),
        attributes: parsed.attributes.clone(),
        kind,
        style,
        children: Vec::new(),
        events: extract_event_bindings(&parsed),
        animations: Vec::new(),
        layout: Default::default(),
    };

    // Keep the raw id for selector matching.
    let html_id = parsed.id.clone();

    // Apply CSS rules in order with ancestor-aware matching.
    apply_rules_to_node_with_ancestors(&mut node, &html_id, rules, variables, ancestors);

    // Apply inline styles (highest specificity).
    if !parsed.inline_style.is_empty() {
        for decl in parsed.inline_style.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                apply_property(&mut node.style, prop.trim(), val.trim(), variables);
            }
        }
    }

    // Snapshot the finalized style for children to inherit from.
    let inherited_style = node.style.clone();
    let node_id = doc.add_node(node);

    // Build ancestor info for children.
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        tag: Some(parsed.tag.clone()),
        classes: parsed.classes.clone(),
        html_id,
    });

    // Add children (unless consumed by widget).
    if !skip_children {
        for child in parsed.children {
            let child_id = add_node_recursive(doc, child, rules, variables, &child_ancestors, Some(&inherited_style));
            doc.add_child(node_id, child_id);
        }
    }

    node_id
}

/// Extract text content from direct #text children of a parsed node.
fn extract_text_content(parsed: &ParsedNode) -> String {
    let mut text = String::new();
    for child in &parsed.children {
        if child.tag == "#text" {
            if let Some(t) = &child.text_content {
                if !text.is_empty() { text.push(' '); }
                text.push_str(t);
            }
        }
    }
    text
}

/// Extract <option> children from a <select> element.
fn extract_select_options(parsed: &ParsedNode) -> Vec<(String, String)> {
    let mut options = Vec::new();
    for child in &parsed.children {
        if child.tag == "option" {
            let value = child.attributes.get("value").cloned().unwrap_or_default();
            let label = extract_text_content(child);
            let label = if label.is_empty() { value.clone() } else { label };
            options.push((value, label));
        }
    }
    options
}

/// Extract event bindings from data-* attributes.
fn extract_event_bindings(parsed: &ParsedNode) -> Vec<EventBinding> {
    let mut events = Vec::new();

    // data-action with optional data-event (defaults to "click")
    if let Some(action_type) = parsed.attributes.get("data-action") {
        let event_type = parsed.attributes.get("data-event")
            .cloned()
            .unwrap_or_else(|| "click".to_string());

        let action = match action_type.as_str() {
            "navigate" => {
                let target = parsed.attributes.get("data-target")
                    .cloned().unwrap_or_default();
                EventAction::Navigate { scene_id: target }
            }
            "ipc" => {
                let ns = parsed.attributes.get("data-ns")
                    .cloned().unwrap_or_default();
                let cmd = parsed.attributes.get("data-cmd")
                    .cloned().unwrap_or_default();
                let args = parsed.attributes.get("data-args")
                    .and_then(|a| serde_json::from_str(a).ok());
                EventAction::IpcCommand { ns, cmd, args }
            }
            "toggle-class" => {
                let class = parsed.attributes.get("data-class")
                    .cloned().unwrap_or_default();
                EventAction::ToggleClass { target: 0, class }
            }
            _ => {
                // Treat the action string as an IPC command name.
                EventAction::IpcCommand {
                    ns: String::new(),
                    cmd: action_type.clone(),
                    args: None,
                }
            }
        };

        events.push(EventBinding { event: event_type, action });
    }

    // data-navigate shorthand
    if let Some(target) = parsed.attributes.get("data-navigate") {
        events.push(EventBinding {
            event: "click".to_string(),
            action: EventAction::Navigate { scene_id: target.clone() },
        });
    }

    events
}

/// Determine the NodeKind from the HTML element.
fn determine_node_kind(parsed: &ParsedNode, _variables: &HashMap<String, String>) -> NodeKind {
    match parsed.tag.as_str() {
        "#text" => {
            NodeKind::Text {
                content: parsed.text_content.clone().unwrap_or_default(),
            }
        }
        "img" => {
            NodeKind::Image {
                asset_index: 0, // Will be resolved during asset bundling.
                fit: ImageFit::Cover,
            }
        }
        "data-bind" | "data-bar" => NodeKind::Container,
        "canvas" => {
            let width: u32 = parsed.attributes.get("width")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300);
            let height: u32 = parsed.attributes.get("height")
                .and_then(|v| v.parse().ok())
                .unwrap_or(150);
            NodeKind::Canvas { width, height }
        }
        "path" => {
            let d = parsed.attributes.get("d").cloned().unwrap_or_default();
            NodeKind::SvgPath {
                d,
                stroke_color: None,
                fill_color: None,
                stroke_width: 2.0,
            }
        }

        // ── Interactive elements ────────────────────────────────────

        "button" => {
            let label = extract_text_content(parsed);
            let disabled = parsed.attributes.contains_key("disabled");
            let variant = match parsed.attributes.get("data-variant").map(|s| s.as_str()) {
                Some("primary") => ButtonVariant::Primary,
                Some("secondary") => ButtonVariant::Secondary,
                Some("danger") => ButtonVariant::Danger,
                Some("ghost") => ButtonVariant::Ghost,
                Some("link") => ButtonVariant::Link,
                _ => ButtonVariant::Primary,
            };
            NodeKind::Input(InputKind::Button { label, disabled, variant })
        }

        "input" => {
            let input_type = parsed.attributes.get("type")
                .map(|s| s.as_str()).unwrap_or("text");
            match input_type {
                "checkbox" => {
                    let checked = parsed.attributes.contains_key("checked");
                    let disabled = parsed.attributes.contains_key("disabled");
                    let label = parsed.attributes.get("data-label")
                        .cloned().unwrap_or_default();
                    let style = match parsed.attributes.get("data-style").map(|s| s.as_str()) {
                        Some("toggle") => CheckboxStyle::Toggle,
                        _ => CheckboxStyle::Checkbox,
                    };
                    NodeKind::Input(InputKind::Checkbox { label, checked, disabled, style })
                }
                "range" => {
                    let value = parsed.attributes.get("value")
                        .and_then(|v| v.parse().ok()).unwrap_or(50.0);
                    let min = parsed.attributes.get("min")
                        .and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let max = parsed.attributes.get("max")
                        .and_then(|v| v.parse().ok()).unwrap_or(100.0);
                    let step = parsed.attributes.get("step")
                        .and_then(|v| v.parse().ok()).unwrap_or(1.0);
                    let disabled = parsed.attributes.contains_key("disabled");
                    NodeKind::Input(InputKind::Slider {
                        value, min, max, step, disabled, show_value: true,
                    })
                }
                _ => {
                    // text, password, number, email, search
                    let placeholder = parsed.attributes.get("placeholder")
                        .cloned().unwrap_or_default();
                    let value = parsed.attributes.get("value")
                        .cloned().unwrap_or_default();
                    let max_length = parsed.attributes.get("maxlength")
                        .and_then(|v| v.parse().ok()).unwrap_or(0);
                    let read_only = parsed.attributes.contains_key("readonly");
                    let kind = match input_type {
                        "password" => TextInputType::Password,
                        "number"   => TextInputType::Number,
                        "email"    => TextInputType::Email,
                        "search"   => TextInputType::Search,
                        _          => TextInputType::Text,
                    };
                    NodeKind::Input(InputKind::TextInput {
                        placeholder, value, max_length, read_only, input_type: kind,
                    })
                }
            }
        }

        "select" => {
            let options = extract_select_options(parsed);
            let selected = parsed.attributes.get("value").cloned();
            let placeholder = parsed.attributes.get("placeholder")
                .cloned().unwrap_or_else(|| "Select...".to_string());
            let disabled = parsed.attributes.contains_key("disabled");
            NodeKind::Input(InputKind::Dropdown {
                options, selected, placeholder, disabled, open: false,
            })
        }

        "textarea" => {
            let placeholder = parsed.attributes.get("placeholder")
                .cloned().unwrap_or_default();
            let value = extract_text_content(parsed);
            let max_length = parsed.attributes.get("maxlength")
                .and_then(|v| v.parse().ok()).unwrap_or(0);
            let read_only = parsed.attributes.contains_key("readonly");
            let rows = parsed.attributes.get("rows")
                .and_then(|v| v.parse().ok()).unwrap_or(4);
            NodeKind::Input(InputKind::TextArea {
                placeholder, value, max_length, read_only, rows,
            })
        }

        _ => NodeKind::Container,
    }
}

/// Apply matching CSS rules to a node with ancestor context for descendant matching.
pub fn apply_rules_to_node_with_ancestors(
    node: &mut CxrdNode,
    html_id: &Option<String>,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
    ancestors: &[AncestorInfo],
) {
    for rule in rules {
        if compound_selector_matches(&rule.compound_selectors, node, html_id, ancestors) {
            for (prop, val) in &rule.declarations {
                apply_property(&mut node.style, prop, val, variables);
            }
        }
    }
}

/// Apply matching CSS rules to a node (legacy, for root node without ancestors).
fn apply_rules_to_node(
    node: &mut CxrdNode,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
) {
    let no_ancestors: Vec<AncestorInfo> = Vec::new();
    apply_rules_to_node_with_ancestors(node, &None, rules, variables, &no_ancestors);
}

/// Check if a compound selector chain matches a node within its ancestor context.
///
/// The last compound selector must match the node itself.
/// Earlier compound selectors must match ancestors (in order, not necessarily consecutive).
pub fn compound_selector_matches(
    selectors: &[CompoundSelector],
    node: &CxrdNode,
    html_id: &Option<String>,
    ancestors: &[AncestorInfo],
) -> bool {
    if selectors.is_empty() {
        return false;
    }

    // The last compound must match the node itself.
    let last = &selectors[selectors.len() - 1];
    if !last.matches_node(
        node.tag.as_deref(),
        &node.classes,
        html_id.as_deref(),
    ) {
        return false;
    }

    // If there's only one compound, we're done (simple selector).
    if selectors.len() == 1 {
        return true;
    }

    // Walk remaining selectors right-to-left, matching against ancestors.
    // Descendant combinator: ancestor doesn't need to be direct parent.
    let remaining = &selectors[..selectors.len() - 1];
    let mut sel_idx = remaining.len() as i32 - 1;
    let mut anc_idx = ancestors.len() as i32 - 1;

    while sel_idx >= 0 && anc_idx >= 0 {
        let sel = &remaining[sel_idx as usize];
        let anc = &ancestors[anc_idx as usize];
        if sel.matches_node(anc.tag.as_deref(), &anc.classes, anc.html_id.as_deref()) {
            sel_idx -= 1;
        }
        anc_idx -= 1;
    }

    // All ancestor selectors must have been matched.
    sel_idx < 0
}

// ═══════════════════════════════════════════════════════════════════════════
// Post-JS CSS restyle pass
// ═══════════════════════════════════════════════════════════════════════════

/// Re-apply CSS rules to every node in the document using the full
/// compile-time pipeline (tag defaults → parent inherit → compound-selector
/// matching → inline styles).  Call this after JS has finished mutating the
/// DOM so that dynamically-created nodes receive proper styling.
pub fn restyle_document(
    doc: &mut CxrdDocument,
    rules: &[CssRule],
    variables: &std::collections::HashMap<String, String>,
) {
    let root = doc.root;
    restyle_recursive(doc, root, rules, variables, &[], None);
}

fn restyle_recursive(
    doc: &mut CxrdDocument,
    node_id: NodeId,
    rules: &[CssRule],
    variables: &std::collections::HashMap<String, String>,
    ancestors: &[AncestorInfo],
    parent_style: Option<&ComputedStyle>,
) {
    // ── 1. Read node metadata (immutable borrow) ──────────────────────
    let (tag, classes, html_id, inline_style, children, canvas_dims) = {
        let node = match doc.get_node(node_id) {
            Some(n) => n,
            None => return,
        };
        let cdims = if let crate::cxrd::node::NodeKind::Canvas { width, height } = &node.kind {
            Some((*width, *height))
        } else {
            None
        };
        (
            node.tag.clone(),
            node.classes.clone(),
            node.html_id.clone(),
            // Reconstruct inline style from the node's `style` attribute if present.
            node.attributes.get("style").cloned().unwrap_or_default(),
            node.children.clone(),
            cdims,
        )
    };

    // ── 2. Build fresh style: defaults → tag defaults → inherit → CSS rules → inline ─
    let mut style = ComputedStyle::default();

    // Tag-specific display defaults (mirrors add_node_recursive).
    if let Some(ref t) = tag {
        match t.as_str() {
            "span" | "a" | "label" | "code" | "small" => {
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
            }
            "strong" | "b" => {
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
                style.font_weight = FontWeight(700);
            }
            "em" | "i" => {
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
            }
            "h1" => { style.font_size = 32.0; style.font_weight = FontWeight(700); }
            "h2" => { style.font_size = 24.0; style.font_weight = FontWeight(700); }
            "h3" => { style.font_size = 18.72; style.font_weight = FontWeight(700); }
            "h4" => { style.font_size = 16.0; style.font_weight = FontWeight(700); }
            "h5" => { style.font_size = 13.28; style.font_weight = FontWeight(700); }
            "h6" => { style.font_size = 10.72; style.font_weight = FontWeight(700); }
            "p" => {
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
                style.flex_wrap = crate::cxrd::style::FlexWrap::Wrap;
            }
            "data-bind" => {
                style.display = crate::cxrd::style::Display::InlineBlock;
                if classes.iter().any(|c| c == "val") {
                    style.flex_grow = 1.0;
                }
            }
            "data-bar" => {
                style.display = crate::cxrd::style::Display::Block;
            }
            "canvas" => {
                style.display = Display::Block;
                // Use canvas buffer dimensions as intrinsic CSS size,
                // matching browser behavior.
                if let Some((w, h)) = canvas_dims {
                    style.width = Dimension::Px(w as f32);
                    style.height = Dimension::Px(h as f32);
                }
            }
            _ => {} // default Block
        }
    }

    // Inherit from parent.
    if let Some(ps) = parent_style {
        style.color = ps.color;
        style.font_size = ps.font_size;
        style.font_family = ps.font_family.clone();
        style.font_weight = ps.font_weight;
        style.letter_spacing = ps.letter_spacing;
        style.line_height = ps.line_height;
        style.text_align = ps.text_align;
        style.text_transform = ps.text_transform;
    }

    // Apply CSS rules with full ancestor-aware matching.
    {
        let node_ref = doc.get_node(node_id).unwrap();
        // Build a temporary CxrdNode-like view for matching.
        for rule in rules {
            if compound_selector_matches(
                &rule.compound_selectors,
                node_ref,
                &html_id,
                ancestors,
            ) {
                // We can't mutate yet — collect declarations to apply.
                // But apply_property needs &mut style, so we match rule, store index.
                for (prop, val) in &rule.declarations {
                    apply_property(&mut style, prop, val, variables);
                }
            }
        }
    }

    // Inline styles (highest specificity).
    if !inline_style.is_empty() {
        for decl in inline_style.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                apply_property(&mut style, prop.trim(), val.trim(), variables);
            }
        }
    }

    // ── 2b. Contain position:fixed inside stacking-context ancestors ──
    // Per CSS spec, an ancestor with `transform` establishes a containing
    // block for fixed-positioned descendants. In practice this means
    // position:fixed elements that are NOT direct children of <body> are
    // almost always contained by a transformed ancestor (e.g. .hud).
    // Our layout engine doesn't track containing blocks, so we approximate
    // this by downgrading position:fixed to position:absolute when the
    // element is deeper than a direct child of body (ancestors > 2 levels).
    if matches!(style.position, crate::cxrd::style::Position::Fixed) && ancestors.len() > 2 {
        style.position = crate::cxrd::style::Position::Absolute;
    }

    // ── 3. Write the new style back to the node ──────────────────────
    let finalized_style = style.clone();
    if let Some(node) = doc.get_node_mut(node_id) {
        node.style = style;
    }

    // ── 4. Build ancestor info and recurse into children ─────────────
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        tag: tag.clone(),
        classes: classes.clone(),
        html_id,
    });

    for cid in children {
        restyle_recursive(doc, cid, rules, variables, &child_ancestors, Some(&finalized_style));
    }
}
