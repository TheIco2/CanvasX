// prism-runtime/src/devtools/elements.rs
//
// Elements panel — Chrome-DevTools-style DOM tree on the left, computed-style
// sidebar with box-model diagram + style sections + state simulators +
// animations on the right. Includes a search box at the top and a breadcrumb
// trail at the bottom.

use std::collections::HashSet;

use crate::gpu::vertex::UiInstance;
use crate::prd::document::PrdDocument;
use crate::prd::node::{NodeId, NodeKind};
use crate::prd::style::{
    AlignSelf, Display, FlexWrap, Overflow, Position, Visibility,
};
use crate::prd::value::{Color, Rect};

use super::theme;
use super::widgets;
use super::DevToolsTextEntry;

// ---------------------------------------------------------------------------
// Layout constants (panel-local)
// ---------------------------------------------------------------------------

const SEARCH_H: f32 = 26.0;
const BREADCRUMB_H: f32 = 22.0;

const ROW_H: f32 = 18.0;
const INDENT: f32 = 14.0;
const CARET_W: f32 = 12.0;

// Chrome-style box model dimensions
const BOX_LABEL_W: f32 = 38.0;

/// Approximate width per character at FONT_SMALL — used for layout-time
/// positioning of syntax-coloured runs since glyphon doesn't expose a
/// synchronous measure.
const CH_W: f32 = 6.2;

// ---------------------------------------------------------------------------
// Public state
// ---------------------------------------------------------------------------

/// Which tab of the right sidebar is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab { Computed, Animations, Listeners }

/// Per-frame state for the Elements panel. Lives on `DevTools`.
pub struct ElementsState {
    pub selected: Option<NodeId>,
    pub expanded: HashSet<NodeId>,
    pub hovered_line: Option<u32>,
    pub scroll: f32,
    pub search_query: String,
    pub search_focused: bool,
    pub sidebar_tab: SidebarTab,
    /// Force-state simulators (hover/active/focus). When set, the renderer's
    /// state-style application reads these as overrides for the selected node.
    pub force_hover: bool,
    pub force_active: bool,
    pub force_focus: bool,
    /// The horizontal split between tree and sidebar (in px from the right edge).
    pub sidebar_width: f32,
    /// Whether the user is currently dragging the sidebar splitter.
    pub dragging_sidebar: bool,
}

impl ElementsState {
    pub fn new() -> Self {
        Self {
            selected: None,
            expanded: HashSet::new(),
            hovered_line: None,
            scroll: 0.0,
            search_query: String::new(),
            search_focused: false,
            sidebar_tab: SidebarTab::Computed,
            force_hover: false,
            force_active: false,
            force_focus: false,
            sidebar_width: theme::SIDEBAR_W,
            dragging_sidebar: false,
        }
    }
}

impl Default for ElementsState { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// Hit-test geometry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub content_x: f32,
    pub content_y: f32,
    pub content_w: f32,
    pub content_h: f32,
    pub search_y: f32,
    pub tree_x: f32,
    pub tree_y: f32,
    pub tree_w: f32,
    pub tree_h: f32,
    pub sidebar_x: f32,
    pub sidebar_y: f32,
    pub sidebar_w: f32,
    pub sidebar_h: f32,
    pub breadcrumb_y: f32,
    pub splitter_x: f32,
}

impl Geometry {
    pub fn compute(state: &ElementsState, content_x: f32, content_y: f32, content_w: f32, content_h: f32) -> Self {
        let sidebar_w = state.sidebar_width.clamp(theme::PANEL_MIN_WIDTH * 0.4, content_w * 0.7);
        let search_y = content_y;
        let tree_y = search_y + SEARCH_H + theme::SP_1;
        let breadcrumb_y = content_y + content_h - BREADCRUMB_H;
        let tree_h = (breadcrumb_y - tree_y - theme::SP_1).max(20.0);
        let sidebar_x = content_x + content_w - sidebar_w;
        let splitter_x = sidebar_x - theme::SPLITTER * 0.5;
        let tree_w = (sidebar_x - content_x - theme::SPLITTER).max(80.0);
        Self {
            content_x, content_y, content_w, content_h,
            search_y,
            tree_x: content_x, tree_y, tree_w, tree_h,
            sidebar_x, sidebar_y: tree_y, sidebar_w, sidebar_h: tree_h,
            breadcrumb_y,
            splitter_x,
        }
    }
}

// ---------------------------------------------------------------------------
// Tree model
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TreeRow {
    pub depth: u32,
    pub node_id: NodeId,
    pub kind: TreeRowKind,
}

#[derive(Debug)]
pub enum TreeRowKind {
    Open { has_children: bool, expanded: bool, inline_text: Option<String> },
    Close,
    Text(String),
}

pub fn build_rows(doc: &PrdDocument, expanded: &HashSet<NodeId>, query: &str) -> Vec<TreeRow> {
    let q = query.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    // If the document root is an anonymous wrapper (no tag, e.g. the synthetic
    // Document/Fragment node produced by the HTML compiler), skip it and walk
    // its children at depth 0 — Chrome-style ("show <html> at the top").
    let root = doc.get_node(doc.root);
    let skip_root = matches!(root, Some(n) if n.tag.is_none());
    if skip_root {
        if let Some(n) = root {
            for &cid in &n.children {
                walk(doc, cid, 0, expanded, &q, &mut out);
            }
        }
    } else {
        walk(doc, doc.root, 0, expanded, &q, &mut out);
    }
    out
}

fn walk(
    doc: &PrdDocument, id: NodeId, depth: u32,
    expanded: &HashSet<NodeId>, query: &str,
    out: &mut Vec<TreeRow>,
) {
    let node = match doc.get_node(id) { Some(n) => n, None => return };
    if matches!(node.style.display, Display::None) { return; }

    if !query.is_empty() && !subtree_matches(doc, id, query) { return; }

    match &node.kind {
        NodeKind::Text { content } => {
            let preview: String = content.chars().take(80).collect();
            out.push(TreeRow { depth, node_id: id, kind: TreeRowKind::Text(preview) });
        }
        _ => {
            let visible_children: Vec<NodeId> = node.children.iter().copied().filter(|cid| {
                doc.get_node(*cid).map(|c| !matches!(c.style.display, Display::None)).unwrap_or(false)
            }).collect();
            let has_children = !visible_children.is_empty();
            // Default-expand the first two depths so the tree shows useful
            // content on first open. The `expanded` set acts as a "user
            // toggled this node" marker — XOR with the default policy.
            let default_expanded = depth < 2;
            let toggled = expanded.contains(&id);
            let is_expanded = (default_expanded ^ toggled) || !query.is_empty();

            let inline_text = if has_children && !is_expanded && visible_children.len() == 1 {
                doc.get_node(visible_children[0]).and_then(|c| match &c.kind {
                    NodeKind::Text { content } => Some(content.chars().take(60).collect::<String>()),
                    _ => None,
                })
            } else { None };

            out.push(TreeRow {
                depth, node_id: id,
                kind: TreeRowKind::Open { has_children, expanded: is_expanded, inline_text: inline_text.clone() },
            });

            if has_children && is_expanded && inline_text.is_none() {
                for cid in visible_children {
                    walk(doc, cid, depth + 1, expanded, query, out);
                }
                out.push(TreeRow { depth, node_id: id, kind: TreeRowKind::Close });
            }
        }
    }
}

fn node_matches_query(node: &crate::prd::node::PrdNode, q: &str) -> bool {
    if let Some(t) = &node.tag { if t.to_ascii_lowercase().contains(q) { return true; } }
    if let Some(i) = &node.html_id { if i.to_ascii_lowercase().contains(q) { return true; } }
    if node.classes.iter().any(|c| c.to_ascii_lowercase().contains(q)) { return true; }
    if let NodeKind::Text { content } = &node.kind {
        if content.to_ascii_lowercase().contains(q) { return true; }
    }
    false
}

fn subtree_matches(doc: &PrdDocument, id: NodeId, q: &str) -> bool {
    if let Some(n) = doc.get_node(id) {
        if node_matches_query(n, q) { return true; }
        for &cid in &n.children {
            if subtree_matches(doc, cid, q) { return true; }
        }
    }
    false
}

pub fn tree_height(rows: &[TreeRow]) -> f32 { rows.len() as f32 * ROW_H + theme::SP_2 }

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

pub fn paint_panel(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
) {
    // Splitter line
    rects.push(widgets::vline(geom.splitter_x, geom.tree_y, geom.tree_h, theme::LINE));
    // Breadcrumb separator
    rects.push(widgets::hline(geom.content_x, geom.breadcrumb_y, geom.content_w, theme::LINE_SOFT));

    // Search bar
    widgets::search_box(
        rects, texts,
        geom.content_x + theme::SP_2, geom.search_y, geom.tree_w - theme::SP_2, SEARCH_H,
        &state.search_query, "Find by tag, id, class or text",
        state.search_focused,
    );

    // Tree
    let rows = build_rows(doc, &state.expanded, &state.search_query);
    paint_tree(rects, texts, &rows, state, doc, geom);

    // Tree scrollbar
    let total = tree_height(&rows);
    widgets::vscrollbar(
        rects,
        geom.splitter_x - theme::SCROLLBAR_W - 2.0,
        geom.tree_y, geom.tree_h, total, state.scroll,
    );

    // Sidebar
    paint_sidebar(rects, texts, state, doc, geom);

    // Breadcrumb
    paint_breadcrumb(rects, texts, state, doc, geom);
}

fn paint_tree(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    rows: &[TreeRow],
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
) {
    let clip = [geom.tree_x, geom.tree_y, geom.tree_x + geom.tree_w, geom.tree_y + geom.tree_h];

    let visible_top = state.scroll;
    let visible_bot = state.scroll + geom.tree_h;
    let row_w = geom.tree_w;

    for (i, row) in rows.iter().enumerate() {
        let local_top = i as f32 * ROW_H;
        if local_top + ROW_H < visible_top || local_top > visible_bot { continue; }

        let row_y = geom.tree_y + theme::SP_1 + local_top - state.scroll;
        let is_selected = state.selected == Some(row.node_id)
            && matches!(row.kind, TreeRowKind::Open { .. } | TreeRowKind::Text(_));
        let is_hovered = state.hovered_line == Some(i as u32);

        if is_selected {
            rects.push(widgets::rect_styled(
                geom.tree_x, row_y, row_w, ROW_H,
                theme::BG_ROW_SELECT, Color::TRANSPARENT, 0.0, [0.0; 4], clip,
            ));
        } else if is_hovered {
            rects.push(widgets::rect_styled(
                geom.tree_x, row_y, row_w, ROW_H,
                theme::BG_ROW_HOVER, Color::TRANSPARENT, 0.0, [0.0; 4], clip,
            ));
        }

        let base_x = geom.tree_x + theme::SP_2 + row.depth as f32 * INDENT;
        let text_y = row_y + (ROW_H - 11.0) * 0.5 - 1.0;

        match &row.kind {
            TreeRowKind::Open { has_children, expanded, inline_text } => {
                if let Some(node) = doc.get_node(row.node_id) {
                    if *has_children {
                        let glyph = if *expanded { "\u{25BE}" } else { "\u{25B8}" };
                        texts.push(DevToolsTextEntry {
                            text: glyph.to_string(),
                            x: base_x, y: text_y, width: CARET_W,
                            font_size: theme::FONT_SMALL, color: theme::TEXT_MUTED, bold: false,
                        });
                    }
                    let mut x = base_x + CARET_W;
                    paint_open_tag(texts, node, &mut x, text_y);
                    if let Some(inline) = inline_text {
                        push_run(texts, inline, &mut x, text_y, theme::SYN_TEXT);
                        push_run(texts, "</", &mut x, text_y, theme::SYN_PUNCT);
                        push_run(texts, node.tag.as_deref().unwrap_or("?"), &mut x, text_y, theme::SYN_TAG);
                        push_run(texts, ">", &mut x, text_y, theme::SYN_PUNCT);
                    }
                }
            }
            TreeRowKind::Close => {
                if let Some(node) = doc.get_node(row.node_id) {
                    let mut x = base_x + CARET_W;
                    push_run(texts, "</", &mut x, text_y, theme::SYN_PUNCT);
                    push_run(texts, node.tag.as_deref().unwrap_or("?"), &mut x, text_y, theme::SYN_TAG);
                    push_run(texts, ">", &mut x, text_y, theme::SYN_PUNCT);
                }
            }
            TreeRowKind::Text(content) => {
                let mut x = base_x + CARET_W;
                push_run(texts, &format!("\"{}\"", content), &mut x, text_y, theme::SYN_TEXT);
            }
        }
    }
}

fn paint_open_tag(
    texts: &mut Vec<DevToolsTextEntry>,
    node: &crate::prd::node::PrdNode,
    x: &mut f32, y: f32,
) {
    let tag = node.tag.as_deref().unwrap_or("?");
    push_run(texts, "<", x, y, theme::SYN_PUNCT);
    push_run(texts, tag, x, y, theme::SYN_TAG);

    if let Some(id) = &node.html_id {
        push_run(texts, " id=", x, y, theme::SYN_ATTR_NAME);
        push_run(texts, &format!("\"{}\"", id), x, y, theme::SYN_ATTR_VAL);
    }
    if !node.classes.is_empty() {
        push_run(texts, " class=", x, y, theme::SYN_ATTR_NAME);
        push_run(texts, &format!("\"{}\"", node.classes.join(" ")), x, y, theme::SYN_ATTR_VAL);
    }
    for (k, v) in &node.attributes {
        if k == "id" || k == "class" { continue; }
        push_run(texts, &format!(" {}=", k), x, y, theme::SYN_ATTR_NAME);
        push_run(texts, &format!("\"{}\"", v), x, y, theme::SYN_ATTR_VAL);
    }
    push_run(texts, ">", x, y, theme::SYN_PUNCT);
}

fn push_run(out: &mut Vec<DevToolsTextEntry>, s: &str, x: &mut f32, y: f32, color: Color) {
    let w = (s.chars().count() as f32 + 0.5) * CH_W;
    out.push(DevToolsTextEntry {
        text: s.to_string(), x: *x, y, width: w + 4.0,
        font_size: theme::FONT_SMALL, color, bold: false,
    });
    *x += w;
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn paint_sidebar(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
) {
    let tab_specs = [
        widgets::TabSpec { label: "Computed",   is_active: state.sidebar_tab == SidebarTab::Computed,
            is_hover: false, badge: None, badge_color: theme::ACCENT },
        widgets::TabSpec { label: "Animations", is_active: state.sidebar_tab == SidebarTab::Animations,
            is_hover: false, badge: None, badge_color: theme::ACCENT },
        widgets::TabSpec { label: "Listeners",  is_active: state.sidebar_tab == SidebarTab::Listeners,
            is_hover: false, badge: None, badge_color: theme::ACCENT },
    ];
    let _ = widgets::tab_bar(
        rects, texts,
        geom.sidebar_x, geom.sidebar_y, geom.sidebar_w,
        &tab_specs,
    );

    let sel = state.selected.and_then(|id| doc.get_node(id));
    let sel = match sel {
        Some(n) => n,
        None => {
            let cy = geom.sidebar_y + theme::TAB_BAR_HEIGHT + theme::SP_5;
            widgets::text(texts, "No element selected.",
                geom.sidebar_x + theme::SP_4, cy, geom.sidebar_w - theme::SP_4 * 2.0,
                theme::FONT_SMALL, theme::TEXT_MUTED);
            return;
        }
    };

    let mut y = geom.sidebar_y + theme::TAB_BAR_HEIGHT + theme::SP_3;
    let x = geom.sidebar_x + theme::SP_4;
    let w = geom.sidebar_w - theme::SP_4 * 2.0;

    paint_state_chips(rects, texts, state, x, y, w);
    y += 24.0 + theme::SP_2;

    match state.sidebar_tab {
        SidebarTab::Computed => {
            paint_box_model(rects, texts, sel, x, y, w);
            y += 96.0 + theme::SP_5;
            paint_computed_props(texts, sel, x, y, w);
        }
        SidebarTab::Animations => {
            paint_animations(texts, sel, doc, x, y, w);
        }
        SidebarTab::Listeners => {
            paint_listeners(texts, sel, x, y, w);
        }
    }
}

fn paint_state_chips(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    state: &ElementsState,
    x: f32, y: f32, _w: f32,
) {
    widgets::text(texts, "Force state", x, y + 4.0, 70.0, theme::FONT_TINY, theme::TEXT_MUTED);
    let chips = [
        widgets::ChipSpec { label: ":hov", active: state.force_hover, count: None, color: theme::ACCENT },
        widgets::ChipSpec { label: ":act", active: state.force_active, count: None, color: theme::ACCENT },
        widgets::ChipSpec { label: ":foc", active: state.force_focus, count: None, color: theme::ACCENT },
    ];
    widgets::chips(rects, texts, x + 70.0, y, 18.0, &chips);
}

// ---------------------------------------------------------------------------
// Box model diagram
// ---------------------------------------------------------------------------

fn paint_box_model(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    node: &crate::prd::node::PrdNode,
    x: f32, y: f32, w: f32,
) {
    widgets::text(texts, "Box model", x, y, 100.0, theme::FONT_TINY, theme::TEXT_MUTED);
    let dy = y + 14.0;
    let area_h = 96.0;
    let m = node.layout.margin;
    let p = node.layout.padding;
    let bw = node.style.border_width;
    let r = node.layout.rect;

    let cx = x;
    let cy = dy;
    let cw = w;
    let ch = area_h;
    rects.push(widgets::rect(cx, cy, cw, ch, theme::BOX_MARGIN, Some(theme::LINE), 0.0));
    widgets::text(texts, "margin",  cx + 4.0, cy + 2.0, BOX_LABEL_W, theme::FONT_TINY, theme::TEXT_PRIMARY);

    let inset = 14.0;
    rects.push(widgets::rect(cx + inset, cy + inset, cw - inset * 2.0, ch - inset * 2.0,
        theme::BOX_BORDER, Some(theme::LINE), 0.0));
    widgets::text(texts, "border",  cx + inset + 4.0, cy + inset + 2.0, BOX_LABEL_W, theme::FONT_TINY, theme::TEXT_PRIMARY);

    let inset2 = inset + 14.0;
    rects.push(widgets::rect(cx + inset2, cy + inset2, cw - inset2 * 2.0, ch - inset2 * 2.0,
        theme::BOX_PADDING, Some(theme::LINE), 0.0));
    widgets::text(texts, "padding", cx + inset2 + 4.0, cy + inset2 + 2.0, BOX_LABEL_W, theme::FONT_TINY, theme::TEXT_PRIMARY);

    let inset3 = inset2 + 14.0;
    let cw3 = (cw - inset3 * 2.0).max(1.0);
    let ch3 = (ch - inset3 * 2.0).max(1.0);
    rects.push(widgets::rect(cx + inset3, cy + inset3, cw3, ch3, theme::BOX_CONTENT, Some(theme::LINE), 0.0));
    let center_label = format!("{:.0} \u{00D7} {:.0}", r.width, r.height);
    widgets::text(texts, &center_label,
        cx + inset3 + cw3 * 0.5 - 24.0, cy + inset3 + ch3 * 0.5 - 6.0,
        cw3, theme::FONT_SMALL, theme::TEXT_PRIMARY);

    let num = |v: f32| if v == 0.0 { "-".to_string() } else { format!("{:.0}", v) };
    let push_num = |t: &mut Vec<DevToolsTextEntry>, s: String, nx: f32, ny: f32| {
        t.push(DevToolsTextEntry {
            text: s, x: nx, y: ny, width: 20.0,
            font_size: theme::FONT_TINY, color: theme::TEXT_PRIMARY, bold: false,
        });
    };
    push_num(texts, num(m.top),    cx + cw * 0.5 - 6.0, cy + 4.0);
    push_num(texts, num(m.bottom), cx + cw * 0.5 - 6.0, cy + ch - 12.0);
    push_num(texts, num(m.left),   cx + 4.0,            cy + ch * 0.5 - 6.0);
    push_num(texts, num(m.right),  cx + cw - 18.0,      cy + ch * 0.5 - 6.0);
    push_num(texts, num(bw.top),    cx + cw * 0.5 - 6.0, cy + inset + 4.0);
    push_num(texts, num(bw.bottom), cx + cw * 0.5 - 6.0, cy + ch - inset - 12.0);
    push_num(texts, num(bw.left),   cx + inset + 4.0,    cy + ch * 0.5 - 6.0);
    push_num(texts, num(bw.right),  cx + cw - inset - 18.0, cy + ch * 0.5 - 6.0);
    push_num(texts, num(p.top),    cx + cw * 0.5 - 6.0, cy + inset2 + 4.0);
    push_num(texts, num(p.bottom), cx + cw * 0.5 - 6.0, cy + ch - inset2 - 12.0);
    push_num(texts, num(p.left),   cx + inset2 + 4.0,   cy + ch * 0.5 - 6.0);
    push_num(texts, num(p.right),  cx + cw - inset2 - 18.0, cy + ch * 0.5 - 6.0);
}

// ---------------------------------------------------------------------------
// Computed style sections
// ---------------------------------------------------------------------------

fn paint_computed_props(
    texts: &mut Vec<DevToolsTextEntry>,
    node: &crate::prd::node::PrdNode,
    x: f32, y: f32, w: f32,
) {
    let s = &node.style;
    let mut y = y;

    section_header(texts, x, &mut y, w, "Layout");
    section_row(texts, x, &mut y, w, "display",        format!("{:?}", s.display).to_lowercase());
    if !matches!(s.position, Position::Static) {
        section_row(texts, x, &mut y, w, "position",   format!("{:?}", s.position).to_lowercase());
    }
    if !matches!(s.overflow, Overflow::Visible) {
        section_row(texts, x, &mut y, w, "overflow",   format!("{:?}", s.overflow).to_lowercase());
    }
    if !matches!(s.visibility, Visibility::Visible) {
        section_row(texts, x, &mut y, w, "visibility", format!("{:?}", s.visibility).to_lowercase());
    }
    section_row(texts, x, &mut y, w, "width",          format!("{:?}", s.width));
    section_row(texts, x, &mut y, w, "height",         format!("{:?}", s.height));
    if matches!(s.display, Display::Flex) {
        section_row(texts, x, &mut y, w, "flex-direction", format!("{:?}", s.flex_direction).to_lowercase());
        if !matches!(s.flex_wrap, FlexWrap::NoWrap) {
            section_row(texts, x, &mut y, w, "flex-wrap", format!("{:?}", s.flex_wrap).to_lowercase());
        }
        section_row(texts, x, &mut y, w, "justify-content", format!("{:?}", s.justify_content).to_lowercase());
        section_row(texts, x, &mut y, w, "align-items",     format!("{:?}", s.align_items).to_lowercase());
    }
    if s.flex_grow != 0.0    { section_row(texts, x, &mut y, w, "flex-grow",   format!("{}", s.flex_grow)); }
    if s.flex_shrink != 1.0  { section_row(texts, x, &mut y, w, "flex-shrink", format!("{}", s.flex_shrink)); }
    if !matches!(s.align_self, AlignSelf::Auto) {
        section_row(texts, x, &mut y, w, "align-self", format!("{:?}", s.align_self).to_lowercase());
    }
    if s.gap > 0.0 { section_row(texts, x, &mut y, w, "gap", format!("{}px", s.gap)); }

    section_header(texts, x, &mut y, w, "Typography");
    if !s.font_family.is_empty() {
        section_row(texts, x, &mut y, w, "font-family", s.font_family.clone());
    }
    section_row(texts, x, &mut y, w, "font-size",   format!("{}px", s.font_size));
    section_row(texts, x, &mut y, w, "font-weight", format!("{:?}", s.font_weight).to_lowercase());
    section_row(texts, x, &mut y, w, "color",       color_to_css(s.color));

    section_header(texts, x, &mut y, w, "Background");
    use crate::prd::style::Background as Bg;
    let bg_text = match &s.background {
        Bg::None => "none".to_string(),
        Bg::Solid(c) => color_to_css(*c),
        _ => format!("{:?}", s.background),
    };
    section_row(texts, x, &mut y, w, "background", bg_text);

    section_header(texts, x, &mut y, w, "Effects");
    if s.opacity < 1.0  { section_row(texts, x, &mut y, w, "opacity", format!("{:.2}", s.opacity)); }
    if s.z_index != 0   { section_row(texts, x, &mut y, w, "z-index", format!("{}", s.z_index)); }
    let br = s.border_radius;
    if br.top_left > 0.0 || br.top_right > 0.0 || br.bottom_right > 0.0 || br.bottom_left > 0.0 {
        section_row(texts, x, &mut y, w, "border-radius",
            format!("{:.0} {:.0} {:.0} {:.0}", br.top_left, br.top_right, br.bottom_right, br.bottom_left));
    }
    if !s.box_shadow.is_empty() {
        section_row(texts, x, &mut y, w, "box-shadow", format!("{} shadow(s)", s.box_shadow.len()));
    }
    if s.transform_scale != 1.0 {
        section_row(texts, x, &mut y, w, "transform", format!("scale({:.2})", s.transform_scale));
    }
}

fn section_header(texts: &mut Vec<DevToolsTextEntry>, x: f32, y: &mut f32, w: f32, label: &str) {
    *y += theme::SP_3;
    widgets::text_bold(texts, label, x, *y, w, theme::FONT_TINY, theme::TEXT_SECONDARY);
    *y += 14.0;
}

fn section_row(texts: &mut Vec<DevToolsTextEntry>, x: f32, y: &mut f32, w: f32, name: &str, value: String) {
    let nw = (w * 0.45).clamp(80.0, 160.0);
    widgets::text(texts, name, x, *y, nw, theme::FONT_TINY, theme::SYN_PROP_NAME);
    widgets::text(texts, value, x + nw + theme::SP_2, *y, w - nw - theme::SP_2, theme::FONT_TINY, theme::SYN_PROP_VAL);
    *y += 14.0;
}

fn color_to_css(c: Color) -> String {
    if (c.a - 1.0).abs() < 0.001 {
        format!("rgb({}, {}, {})", (c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8)
    } else {
        format!("rgba({}, {}, {}, {:.2})", (c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8, c.a)
    }
}

// ---------------------------------------------------------------------------
// Animations / Listeners panes
// ---------------------------------------------------------------------------

fn paint_animations(
    texts: &mut Vec<DevToolsTextEntry>,
    node: &crate::prd::node::PrdNode,
    doc: &PrdDocument,
    x: f32, mut y: f32, w: f32,
) {
    if node.animations.is_empty() {
        widgets::text(texts, "No animations on this node.", x, y, w, theme::FONT_SMALL, theme::TEXT_MUTED);
        return;
    }
    for &anim_id in &node.animations {
        if let Some(anim) = doc.animations.get(anim_id as usize) {
            let line = format!("\u{25B6} {}  ({:.0}ms)",
                anim.name, anim.duration_ms);
            widgets::text(texts, &line, x, y, w, theme::FONT_SMALL, theme::TEXT_PRIMARY);
            y += 16.0;
            let line2 = format!("    keyframes: {},  iter: {}",
                anim.keyframes.len(), anim.iteration_count);
            widgets::text(texts, &line2, x, y, w, theme::FONT_TINY, theme::TEXT_MUTED);
            y += 16.0;
        }
    }
}

fn paint_listeners(
    texts: &mut Vec<DevToolsTextEntry>,
    node: &crate::prd::node::PrdNode,
    x: f32, mut y: f32, w: f32,
) {
    if node.events.is_empty() {
        widgets::text(texts, "No event listeners on this node.", x, y, w, theme::FONT_SMALL, theme::TEXT_MUTED);
        return;
    }
    for ev in &node.events {
        let line = format!("\u{2022} {}  \u{2192}  {:?}", ev.event, ev.action);
        widgets::text(texts, &line, x, y, w, theme::FONT_SMALL, theme::TEXT_PRIMARY);
        y += 16.0;
    }
}

// ---------------------------------------------------------------------------
// Breadcrumb
// ---------------------------------------------------------------------------

fn paint_breadcrumb(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
) {
    paint_breadcrumb_into(rects, texts, state, doc, geom);
}

/// Public helper used by `DevTools::post_text_instances` /
/// `post_text_entries` to repaint the breadcrumb in the overlay layer
/// (drawn after the DevTools text pass) so that tree-row text drawn in
/// the same text batch never bleeds through the breadcrumb bar.
pub fn paint_breadcrumb_into(
    rects: &mut Vec<UiInstance>,
    texts: &mut Vec<DevToolsTextEntry>,
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
) {
    rects.push(widgets::rect(geom.content_x, geom.breadcrumb_y, geom.content_w, BREADCRUMB_H, theme::BG_TOOLBAR, None, 0.0));
    let path = ancestor_path(doc, state.selected);
    if path.is_empty() {
        widgets::text(texts, "html", geom.content_x + theme::SP_4, geom.breadcrumb_y + 5.0,
            200.0, theme::FONT_SMALL, theme::TEXT_MUTED);
        return;
    }
    let mut x = geom.content_x + theme::SP_4;
    let y = geom.breadcrumb_y + 5.0;
    for (i, id) in path.iter().enumerate() {
        let label = node_summary(doc, *id);
        let color = if Some(*id) == state.selected { theme::ACCENT } else { theme::TEXT_SECONDARY };
        let w = label.chars().count() as f32 * CH_W + 6.0;
        widgets::text(texts, &label, x, y, w, theme::FONT_SMALL, color);
        x += w;
        if i + 1 < path.len() {
            widgets::text(texts, "\u{203A}", x, y, 10.0, theme::FONT_SMALL, theme::TEXT_MUTED);
            x += 12.0;
        }
    }
}

pub fn ancestor_path(doc: &PrdDocument, selected: Option<NodeId>) -> Vec<NodeId> {
    let target = match selected { Some(t) => t, None => return Vec::new() };
    let mut path = Vec::new();
    fn search(doc: &PrdDocument, here: NodeId, target: NodeId, path: &mut Vec<NodeId>) -> bool {
        if here == target { path.push(here); return true; }
        if let Some(n) = doc.get_node(here) {
            for &c in &n.children {
                if search(doc, c, target, path) { path.insert(0, here); return true; }
            }
        }
        false
    }
    search(doc, doc.root, target, &mut path);
    path
}

fn node_summary(doc: &PrdDocument, id: NodeId) -> String {
    if let Some(n) = doc.get_node(id) {
        let mut s = n.tag.clone().unwrap_or_else(|| "?".to_string());
        if let Some(i) = &n.html_id { s.push('#'); s.push_str(i); }
        else if let Some(c) = n.classes.first() { s.push('.'); s.push_str(c); }
        s
    } else {
        "?".to_string()
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    None,
    Search,
    Splitter,
    SidebarTab(SidebarTab),
    StateChip(StateChip),
    TreeRow(usize),
    TreeCaret(usize),
    Breadcrumb(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChip { Hover, Active, Focus }

pub fn hit_test(
    state: &ElementsState,
    doc: &PrdDocument,
    geom: &Geometry,
    x: f32, y: f32,
) -> Hit {
    if y >= geom.search_y && y < geom.search_y + SEARCH_H
        && x >= geom.content_x + theme::SP_2 && x < geom.content_x + geom.tree_w {
        return Hit::Search;
    }
    if x >= geom.splitter_x - 3.0 && x <= geom.splitter_x + 3.0
        && y >= geom.tree_y && y <= geom.tree_y + geom.tree_h {
        return Hit::Splitter;
    }
    if y >= geom.sidebar_y && y < geom.sidebar_y + theme::TAB_BAR_HEIGHT
        && x >= geom.sidebar_x && x < geom.sidebar_x + geom.sidebar_w {
        let local = (x - geom.sidebar_x - theme::SP_2).max(0.0);
        let tab_w = 95.0;
        let idx = (local / tab_w) as usize;
        return match idx {
            0 => Hit::SidebarTab(SidebarTab::Computed),
            1 => Hit::SidebarTab(SidebarTab::Animations),
            _ => Hit::SidebarTab(SidebarTab::Listeners),
        };
    }
    let chips_y = geom.sidebar_y + theme::TAB_BAR_HEIGHT + theme::SP_3;
    if y >= chips_y && y <= chips_y + 22.0 {
        let chips_x = geom.sidebar_x + theme::SP_4 + 70.0;
        let chip_w = 44.0;
        if x >= chips_x && x < chips_x + (chip_w + theme::SP_2) * 3.0 {
            let idx = ((x - chips_x) / (chip_w + theme::SP_2)) as usize;
            return Hit::StateChip(match idx {
                0 => StateChip::Hover, 1 => StateChip::Active, _ => StateChip::Focus,
            });
        }
    }
    if y >= geom.tree_y && y < geom.tree_y + geom.tree_h
        && x >= geom.tree_x && x < geom.splitter_x - 4.0 {
        let local_y = y - geom.tree_y - theme::SP_1 + state.scroll;
        if local_y >= 0.0 {
            let idx = (local_y / ROW_H) as usize;
            let rows = build_rows(doc, &state.expanded, &state.search_query);
            if idx < rows.len() {
                let row = &rows[idx];
                let base_x = geom.tree_x + theme::SP_2 + row.depth as f32 * INDENT;
                if matches!(row.kind, TreeRowKind::Open { has_children: true, .. })
                    && x >= base_x && x < base_x + CARET_W + 2.0
                {
                    return Hit::TreeCaret(idx);
                }
                return Hit::TreeRow(idx);
            }
        }
    }
    if y >= geom.breadcrumb_y && y < geom.breadcrumb_y + BREADCRUMB_H {
        let path = ancestor_path(doc, state.selected);
        let mut cx = geom.content_x + theme::SP_4;
        for (i, id) in path.iter().enumerate() {
            let label = node_summary(doc, *id);
            let w = label.chars().count() as f32 * CH_W + 6.0;
            if x >= cx && x < cx + w {
                return Hit::Breadcrumb(i as u32);
            }
            cx += w + 12.0;
        }
    }
    Hit::None
}

/// Resolve the row index returned by [`hit_test`] back to a `NodeId`, using
/// the same row list semantics. Returns `None` if out of range.
pub fn row_node(rows: &[TreeRow], idx: usize) -> Option<NodeId> {
    rows.get(idx).map(|r| r.node_id)
}

/// Highlight rect for the in-scene element overlay.
pub fn scene_highlight(node_rect: Rect, fill: Color, border: Color, weight: f32) -> Vec<UiInstance> {
    if node_rect.width <= 0.0 || node_rect.height <= 0.0 { return Vec::new(); }
    vec![widgets::rect_styled(
        node_rect.x, node_rect.y, node_rect.width, node_rect.height,
        fill, border, weight, [0.0; 4], [0.0, 0.0, 99999.0, 99999.0],
    )]
}

// ---------------------------------------------------------------------------
// Backwards-compat shims for callers still using the old API.
// These mirror the prior elements.rs surface so main.rs / app_host.rs / overlay
// don't break during the transition. All delegate to the new model.
// ---------------------------------------------------------------------------

/// Width reserved for the styles sidebar (legacy callers).
pub const STYLES_SIDEBAR_WIDTH: f32 = theme::SIDEBAR_W;

/// Total visible row count given the expanded set (legacy callers).
pub fn tree_line_count(doc: &PrdDocument, expanded: &HashSet<NodeId>) -> usize {
    build_rows(doc, expanded, "").len()
}

/// Look up the NodeId at a tree row index (legacy callers).
pub fn node_id_at_line(doc: &PrdDocument, line_idx: usize, expanded: &HashSet<NodeId>) -> Option<NodeId> {
    build_rows(doc, expanded, "").get(line_idx).map(|r| r.node_id)
}

/// Whether the row at `line_idx` is an open-tag row with children (legacy).
pub fn node_has_children_at_line(doc: &PrdDocument, line_idx: usize, expanded: &HashSet<NodeId>) -> bool {
    build_rows(doc, expanded, "").get(line_idx).is_some_and(|r| {
        matches!(r.kind, TreeRowKind::Open { has_children: true, .. })
    })
}

/// Legacy text-entries entry point that the existing mod.rs `text_entries`
/// path still calls. Wraps the new paint pipeline but only retains the text
/// entries (rect background drawing happens in `paint_rects_elements`).
#[allow(clippy::too_many_arguments)]
pub fn text_entries_elements(
    out: &mut Vec<DevToolsTextEntry>,
    doc: &PrdDocument,
    content_x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
    selected: Option<u32>,
    expanded: &HashSet<NodeId>,
    hovered_line: Option<u32>,
) {
    let state = make_legacy_state(scroll, selected, expanded, hovered_line);
    let geom = Geometry::compute(
        &state, content_x, content_y,
        (viewport_width - content_x * 2.0).max(80.0), content_h,
    );
    let mut rects: Vec<UiInstance> = Vec::new();
    paint_panel(&mut rects, out, &state, doc, &geom);
    drop(rects);
}

/// Rect-only entry point used by `overlay::paint_panel` for the Elements tab.
/// Mirrors `text_entries_elements` but discards text and keeps rects.
#[allow(clippy::too_many_arguments)]
pub fn paint_rects_elements(
    out: &mut Vec<UiInstance>,
    doc: &PrdDocument,
    content_x: f32,
    content_y: f32,
    viewport_width: f32,
    content_h: f32,
    scroll: f32,
    selected: Option<u32>,
    expanded: &HashSet<NodeId>,
    hovered_line: Option<u32>,
) {
    let state = make_legacy_state(scroll, selected, expanded, hovered_line);
    let geom = Geometry::compute(
        &state, content_x, content_y,
        (viewport_width - content_x * 2.0).max(80.0), content_h,
    );
    let mut texts: Vec<DevToolsTextEntry> = Vec::new();
    paint_panel(out, &mut texts, &state, doc, &geom);
    drop(texts);
}

/// State-aware rect entry point. Use this when DevTools owns persistent
/// `ElementsState` so search/chip/sidebar state survives across frames.
pub fn paint_rects_with_state(
    out: &mut Vec<UiInstance>,
    doc: &PrdDocument,
    state: &ElementsState,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
) {
    let geom = Geometry::compute(state, content_x, content_y, content_w, content_h);
    let mut texts: Vec<DevToolsTextEntry> = Vec::new();
    paint_panel(out, &mut texts, state, doc, &geom);
    drop(texts);
}

/// State-aware text entry point. Twin of [`paint_rects_with_state`].
pub fn text_entries_with_state(
    out: &mut Vec<DevToolsTextEntry>,
    doc: &PrdDocument,
    state: &ElementsState,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
) {
    let geom = Geometry::compute(state, content_x, content_y, content_w, content_h);
    let mut rects: Vec<UiInstance> = Vec::new();
    paint_panel(&mut rects, out, state, doc, &geom);
    drop(rects);
}

fn make_legacy_state(
    scroll: f32,
    selected: Option<u32>,
    expanded: &HashSet<NodeId>,
    hovered_line: Option<u32>,
) -> ElementsState {
    ElementsState {
        selected,
        expanded: expanded.clone(),
        hovered_line,
        scroll,
        search_query: String::new(),
        search_focused: false,
        sidebar_tab: SidebarTab::Computed,
        force_hover: false,
        force_active: false,
        force_focus: false,
        sidebar_width: theme::SIDEBAR_W,
        dragging_sidebar: false,
    }
}
