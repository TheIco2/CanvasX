// openrender-runtime/src/devtools/elements.rs
//
// Elements panel for the OpenRender DevTools.
// Renders a tree view of the CXRD document showing the DOM structure
// with collapsible nodes, computed styles sidebar, and box model display.

use std::collections::HashSet;
use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeKind, NodeId};
use crate::cxrd::style::{Display, ComputedStyle, Background, Position, FlexDirection, JustifyContent, AlignItems, AlignSelf, FlexWrap, Overflow, Visibility};
use crate::cxrd::value::Color;
use super::DevToolsTextEntry;

/// Width reserved for the computed styles sidebar when a node is selected.
pub const STYLES_SIDEBAR_WIDTH: f32 = 280.0;

/// A line in the elements tree view, tracking hit-test data.
pub struct ElementTreeLine {
    pub text: String,
    pub depth: u32,
    pub node_id: NodeId,
    pub has_children: bool,
}

/// Generate text entries for the Elements panel (DOM tree view + computed styles sidebar).
pub fn text_entries_elements(
    out: &mut Vec<DevToolsTextEntry>,
    doc: &CxrdDocument,
    x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
    selected_node: Option<u32>,
    expanded: &HashSet<NodeId>,
    hovered_line: Option<u32>,
) {
    let line_h = 16.0;
    let indent_px = 16.0;

    // Compute tree width: if a node is selected, reserve space for styles sidebar
    let tree_width = if selected_node.is_some() {
        (viewport_width - STYLES_SIDEBAR_WIDTH).max(200.0)
    } else {
        viewport_width
    };

    let mut lines: Vec<ElementTreeLine> = Vec::new();
    collect_tree_lines(doc, doc.root, 0, &mut lines, expanded);

    let visible_start = scroll;
    let visible_end = scroll + content_h;

    for (i, line) in lines.iter().enumerate() {
        let entry_y = i as f32 * line_h;
        if entry_y + line_h < visible_start || entry_y > visible_end {
            continue;
        }
        let render_y = content_y + 4.0 + entry_y - scroll;
        let indent = x + 4.0 + line.depth as f32 * indent_px;

        let is_selected = selected_node == Some(line.node_id);
        let is_hovered = hovered_line == Some(i as u32);

        // Collapse/expand indicator
        let arrow = if line.has_children {
            if expanded.contains(&line.node_id) { "\u{25BE} " } else { "\u{25B8} " }
        } else {
            "  "
        };

        let color = if is_selected {
            Color::new(0.39, 0.40, 0.95, 1.0)
        } else if is_hovered {
            Color::new(0.6, 0.7, 0.9, 1.0)
        } else {
            Color::new(0.75, 0.75, 0.75, 1.0)
        };

        out.push(DevToolsTextEntry {
            text: format!("{}{}", arrow, line.text),
            x: indent,
            y: render_y,
            width: tree_width - indent - 8.0,
            font_size: 11.0,
            color,
            bold: is_selected,
        });
    }

    // --- Computed Styles Sidebar ---
    if let Some(sel_id) = selected_node {
        if let Some(node) = doc.get_node(sel_id) {
            let sidebar_x = tree_width + 4.0;
            let sidebar_w = STYLES_SIDEBAR_WIDTH - 8.0;
            let mut y = content_y + 4.0;
            let line_h = 14.0;

            // Header
            let tag = node.tag.as_deref().unwrap_or("?");
            let header = if let Some(ref html_id) = node.html_id {
                format!("{}#{}", tag, html_id)
            } else if !node.classes.is_empty() {
                format!("{}.{}", tag, node.classes.join("."))
            } else {
                tag.to_string()
            };
            out.push(DevToolsTextEntry {
                text: format!("Computed — {}", header),
                x: sidebar_x,
                y,
                width: sidebar_w,
                font_size: 11.0,
                color: Color::new(0.39, 0.40, 0.95, 1.0),
                bold: true,
            });
            y += line_h + 4.0;

            // Box Model
            let layout = &node.layout;
            let r = &layout.rect;
            out.push(DevToolsTextEntry {
                text: format!("Box: {:.0} x {:.0} at ({:.0}, {:.0})", r.width, r.height, r.x, r.y),
                x: sidebar_x,
                y,
                width: sidebar_w,
                font_size: 10.0,
                color: Color::new(0.6, 0.8, 0.6, 1.0),
                bold: false,
            });
            y += line_h;

            let m = &layout.margin;
            let p = &layout.padding;
            out.push(DevToolsTextEntry {
                text: format!("Margin: {:.0} {:.0} {:.0} {:.0}", m.top, m.right, m.bottom, m.left),
                x: sidebar_x,
                y,
                width: sidebar_w,
                font_size: 10.0,
                color: Color::new(0.8, 0.7, 0.4, 1.0),
                bold: false,
            });
            y += line_h;

            out.push(DevToolsTextEntry {
                text: format!("Padding: {:.0} {:.0} {:.0} {:.0}", p.top, p.right, p.bottom, p.left),
                x: sidebar_x,
                y,
                width: sidebar_w,
                font_size: 10.0,
                color: Color::new(0.5, 0.8, 0.5, 1.0),
                bold: false,
            });
            y += line_h;

            let bw = &node.style.border_width;
            out.push(DevToolsTextEntry {
                text: format!("Border: {:.0} {:.0} {:.0} {:.0}", bw.top, bw.right, bw.bottom, bw.left),
                x: sidebar_x,
                y,
                width: sidebar_w,
                font_size: 10.0,
                color: Color::new(0.8, 0.7, 0.5, 1.0),
                bold: false,
            });
            y += line_h + 4.0;

            // Style Properties
            let style = &node.style;
            let props = computed_style_props(style);
            for (name, value) in &props {
                out.push(DevToolsTextEntry {
                    text: format!("{}: {}", name, value),
                    x: sidebar_x,
                    y,
                    width: sidebar_w,
                    font_size: 10.0,
                    color: Color::new(0.65, 0.65, 0.70, 1.0),
                    bold: false,
                });
                y += line_h;
            }

            // State flags
            y += 4.0;
            let mut state_parts = Vec::new();
            if node.hovered { state_parts.push("hovered"); }
            if node.active { state_parts.push("active"); }
            if node.focused { state_parts.push("focused"); }
            if !state_parts.is_empty() {
                out.push(DevToolsTextEntry {
                    text: format!("State: {}", state_parts.join(", ")),
                    x: sidebar_x,
                    y,
                    width: sidebar_w,
                    font_size: 10.0,
                    color: Color::new(0.9, 0.6, 0.3, 1.0),
                    bold: false,
                });
            }
        }
    }
}

/// Total number of visible lines in the tree (for scroll calculations).
pub fn tree_line_count(doc: &CxrdDocument, expanded: &HashSet<NodeId>) -> usize {
    let mut lines = Vec::new();
    collect_tree_lines(doc, doc.root, 0, &mut lines, expanded);
    lines.len()
}

/// Get the node_id for a given line index in the tree.
pub fn node_id_at_line(doc: &CxrdDocument, line_idx: usize, expanded: &HashSet<NodeId>) -> Option<NodeId> {
    let mut lines = Vec::new();
    collect_tree_lines(doc, doc.root, 0, &mut lines, expanded);
    lines.get(line_idx).map(|l| l.node_id)
}

/// Check if the node at a given line has children (for expand/collapse).
pub fn node_has_children_at_line(doc: &CxrdDocument, line_idx: usize, expanded: &HashSet<NodeId>) -> bool {
    let mut lines = Vec::new();
    collect_tree_lines(doc, doc.root, 0, &mut lines, expanded);
    lines.get(line_idx).map(|l| l.has_children).unwrap_or(false)
}

fn collect_tree_lines(
    doc: &CxrdDocument,
    node_id: NodeId,
    depth: u32,
    lines: &mut Vec<ElementTreeLine>,
    expanded: &HashSet<NodeId>,
) {
    let node = match doc.get_node(node_id) {
        Some(n) => n,
        None => return,
    };

    if matches!(node.style.display, Display::None) {
        return;
    }

    let tag = node.tag.as_deref().unwrap_or("?");
    let mut label = String::new();
    let has_children;

    match &node.kind {
        NodeKind::Text { content } => {
            let preview: String = content.chars().take(60).collect();
            label = format!("\"{}\"", preview);
            has_children = false;
        }
        _ => {
            label.push('<');
            label.push_str(tag);

            if let Some(ref id) = node.html_id {
                label.push_str(&format!(" id=\"{}\"", id));
            }
            if !node.classes.is_empty() {
                label.push_str(&format!(" class=\"{}\"", node.classes.join(" ")));
            }

            label.push('>');

            // Count visible children for collapse indicator
            has_children = node.children.iter().any(|&cid| {
                doc.get_node(cid)
                    .map(|c| !matches!(c.style.display, Display::None))
                    .unwrap_or(false)
            });

            // Style hints
            let style = &node.style;
            let mut hints = Vec::new();
            if !matches!(style.display, Display::Block) {
                hints.push(format!("{:?}", style.display).to_lowercase());
            }
            if style.flex_grow > 0.0 {
                hints.push(format!("grow:{}", style.flex_grow));
            }
            if !hints.is_empty() {
                label.push_str(&format!("  [{}]", hints.join(", ")));
            }
        }
    }

    lines.push(ElementTreeLine {
        text: label,
        depth,
        node_id,
        has_children,
    });

    // Only recurse into children if this node is expanded (or has no children to toggle).
    if has_children && !expanded.contains(&node_id) {
        return; // collapsed — skip children
    }
    let children = node.children.clone();
    for child_id in children {
        collect_tree_lines(doc, child_id, depth + 1, lines, expanded);
    }
}

/// Produce a list of (property_name, value) for the computed style sidebar.
fn computed_style_props(style: &ComputedStyle) -> Vec<(&'static str, String)> {
    let mut props = Vec::new();

    props.push(("display", format!("{:?}", style.display).to_lowercase()));
    if !matches!(style.position, Position::Relative) {
        props.push(("position", format!("{:?}", style.position).to_lowercase()));
    }
    if !matches!(style.overflow, Overflow::Visible) {
        props.push(("overflow", format!("{:?}", style.overflow).to_lowercase()));
    }
    if !matches!(style.visibility, Visibility::Visible) {
        props.push(("visibility", format!("{:?}", style.visibility).to_lowercase()));
    }

    props.push(("width", format!("{:?}", style.width)));
    props.push(("height", format!("{:?}", style.height)));

    // Flex properties (only if flex container or flex item with non-defaults)
    if matches!(style.display, Display::Flex) {
        props.push(("flex-direction", format!("{:?}", style.flex_direction).to_lowercase()));
        if !matches!(style.flex_wrap, FlexWrap::NoWrap) {
            props.push(("flex-wrap", format!("{:?}", style.flex_wrap).to_lowercase()));
        }
        props.push(("justify-content", format!("{:?}", style.justify_content).to_lowercase()));
        props.push(("align-items", format!("{:?}", style.align_items).to_lowercase()));
    }
    if style.flex_grow != 0.0 {
        props.push(("flex-grow", format!("{}", style.flex_grow)));
    }
    if style.flex_shrink != 1.0 {
        props.push(("flex-shrink", format!("{}", style.flex_shrink)));
    }
    if !matches!(style.align_self, AlignSelf::Auto) {
        props.push(("align-self", format!("{:?}", style.align_self).to_lowercase()));
    }
    if style.gap > 0.0 {
        props.push(("gap", format!("{}px", style.gap)));
    }

    // Background
    match &style.background {
        Background::None => {}
        Background::Solid(c) => {
            props.push(("background", format!("rgba({},{},{},{:.2})", (c.r*255.0) as u8, (c.g*255.0) as u8, (c.b*255.0) as u8, c.a)));
        }
        _ => {
            props.push(("background", format!("{:?}", style.background)));
        }
    }

    // Color
    let c = &style.color;
    props.push(("color", format!("rgba({},{},{},{:.2})", (c.r*255.0) as u8, (c.g*255.0) as u8, (c.b*255.0) as u8, c.a)));

    // Typography
    if !style.font_family.is_empty() {
        props.push(("font-family", style.font_family.clone()));
    }
    props.push(("font-size", format!("{}px", style.font_size)));
    props.push(("font-weight", format!("{:?}", style.font_weight)));

    if style.opacity < 1.0 {
        props.push(("opacity", format!("{:.2}", style.opacity)));
    }
    if style.z_index != 0 {
        props.push(("z-index", format!("{}", style.z_index)));
    }
    if style.border_radius.top_left > 0.0 || style.border_radius.top_right > 0.0
        || style.border_radius.bottom_right > 0.0 || style.border_radius.bottom_left > 0.0
    {
        props.push(("border-radius", format!("{:.0} {:.0} {:.0} {:.0}",
            style.border_radius.top_left, style.border_radius.top_right,
            style.border_radius.bottom_right, style.border_radius.bottom_left)));
    }
    if !style.box_shadow.is_empty() {
        props.push(("box-shadow", format!("{} shadow(s)", style.box_shadow.len())));
    }
    if style.transform_scale != 1.0 {
        props.push(("transform-scale", format!("{:.2}", style.transform_scale)));
    }

    props
}
