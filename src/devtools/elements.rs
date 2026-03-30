// openrender-runtime/src/devtools/elements.rs
//
// Elements panel for the OpenRender DevTools.
// Renders a tree view of the CXRD document showing the DOM structure.

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeKind, NodeId};
use crate::cxrd::style::Display;
use crate::cxrd::value::Color;
use super::DevToolsTextEntry;

/// Generate text entries for the Elements panel (DOM tree view).
pub fn text_entries_elements(
    out: &mut Vec<DevToolsTextEntry>,
    doc: &CxrdDocument,
    x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
    selected_node: Option<u32>,
) {
    let line_h = 16.0;
    let indent_px = 16.0;
    let mut lines: Vec<(String, u32, u32)> = Vec::new(); // (text, depth, node_id)

    // Walk the tree depth-first.
    collect_tree_lines(doc, doc.root, 0, &mut lines);

    let visible_start = scroll;
    let visible_end = scroll + content_h;

    for (i, (text, depth, node_id)) in lines.iter().enumerate() {
        let entry_y = i as f32 * line_h;
        if entry_y + line_h < visible_start || entry_y > visible_end {
            continue;
        }
        let render_y = content_y + 4.0 + entry_y - scroll;
        let indent = x + 4.0 + *depth as f32 * indent_px;

        let is_selected = selected_node == Some(*node_id);
        let color = if is_selected {
            Color::new(0.39, 0.40, 0.95, 1.0) // Accent
        } else {
            Color::new(0.75, 0.75, 0.75, 1.0)
        };

        out.push(DevToolsTextEntry {
            text: text.clone(),
            x: indent,
            y: render_y,
            width: viewport_width - indent - 8.0,
            font_size: 11.0,
            color,
            bold: is_selected,
        });
    }
}

fn collect_tree_lines(
    doc: &CxrdDocument,
    node_id: NodeId,
    depth: u32,
    lines: &mut Vec<(String, u32, u32)>,
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

    match &node.kind {
        NodeKind::Text { content } => {
            let preview: String = content.chars().take(60).collect();
            label = format!("\"{}\"", preview);
        }
        _ => {
            label.push('<');
            label.push_str(tag);

            // Show id attribute
            if let Some(ref id) = node.html_id {
                label.push_str(&format!(" id=\"{}\"", id));
            }

            // Show classes
            if !node.classes.is_empty() {
                label.push_str(&format!(" class=\"{}\"", node.classes.join(" ")));
            }

            label.push('>');

            // Show computed style hints
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

    lines.push((label, depth, node_id));

    for &child_id in &node.children {
        collect_tree_lines(doc, child_id, depth + 1, lines);
    }
}
