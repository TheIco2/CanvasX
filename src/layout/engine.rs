// canvasx-runtime/src/layout/engine.rs
//
// Top-level layout engine — traverses the CXRD node tree and
// computes layout positions for each node using block flow or flexbox.

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::input::InputKind;
use crate::cxrd::node::{NodeId, NodeKind};
use crate::cxrd::style::{Display, FlexDirection, Position, GridTrackSize};
use crate::cxrd::value::{Dimension, Rect, EdgeInsets};
use crate::layout::types::LayoutConstraints;

/// Perform a full layout pass on a CXRD document.
///
/// After this, every node's `layout.rect` is populated with its
/// absolute pixel position and size.
pub fn compute_layout(doc: &mut CxrdDocument, viewport_width: f32, viewport_height: f32) {
    let constraints = LayoutConstraints::new(viewport_width, viewport_height);

    // Set root node to fill viewport.
    if let Some(root) = doc.nodes.get_mut(doc.root as usize) {
        root.layout.rect = Rect {
            x: 0.0,
            y: 0.0,
            width: viewport_width,
            height: viewport_height,
        };
        root.layout.content_rect = root.layout.rect;
    }

    // Layout tree recursively (iterative via stack to avoid deep recursion).
    let root_id = doc.root;
    layout_node_recursive(doc, root_id, &constraints, None);
}

/// Recursively layout a node and its children.
///
/// We do this in a slightly awkward way because we can't hold mutable
/// references to multiple nodes simultaneously in a Vec.  We collect
/// child info, compute layout, then write results back.
fn layout_node_recursive(
    doc: &mut CxrdDocument,
    node_id: NodeId,
    constraints: &LayoutConstraints,
    clip: Option<Rect>,
) {
    let node = match doc.nodes.get(node_id as usize) {
        Some(n) => n,
        None => return,
    };

    if matches!(node.style.display, Display::None) {
        return;
    }

    let style = node.style.clone();
    let container_rect = node.layout.content_rect;
    let children: Vec<NodeId> = node.children.clone();

    // Set clip for overflow: hidden containers.
    let child_clip = if matches!(style.overflow, crate::cxrd::style::Overflow::Hidden) {
        Some(container_rect)
    } else {
        clip
    };

    if children.is_empty() {
        return;
    }

    // Determine layout mode.
    let is_flex = matches!(style.display, Display::Flex);
    let is_grid = matches!(style.display, Display::Grid);

    if is_grid {
        // --- CSS Grid layout ---
        layout_grid_children(doc, node_id, container_rect, &children, constraints);
    } else if is_flex {
        // --- Flexbox layout ---
        layout_flex_children(doc, node_id, container_rect, &children, constraints);
    } else {
        // --- Block flow layout ---
        layout_block_children(doc, container_rect, &children, constraints);
    }

    // Handle absolute-positioned children.
    let mut abs_shrink: Vec<(NodeId, bool, bool)> = Vec::new();
    for &child_id in &children {
        if let Some(child) = doc.nodes.get(child_id as usize) {
            if matches!(child.style.position, Position::Absolute | Position::Fixed) {
                let cs = child.style.clone();
                let containing = if matches!(cs.position, Position::Fixed) {
                    Rect::new(0.0, 0.0, constraints.viewport_width, constraints.viewport_height)
                } else {
                    container_rect
                };
                let (sw, sh) = layout_absolute_child(doc, child_id, containing, constraints);
                if sw || sh {
                    abs_shrink.push((child_id, sw, sh));
                }
            }
        }

        // Set clip on children.
        if let Some(child) = doc.nodes.get_mut(child_id as usize) {
            child.layout.clip = child_clip;
        }
    }

    // Recurse into children.
    for &child_id in &children {
        layout_node_recursive(doc, child_id, constraints, child_clip);
    }

    // After recursive layout, shrink absolute/fixed elements that had
    // auto dimensions without opposing insets.
    for (child_id, sw, sh) in abs_shrink {
        shrink_absolute_to_content(doc, child_id, sw, sh, constraints);
    }

    // Apply simple transform scale() (top-left origin) to this subtree.
    if (style.transform_scale - 1.0).abs() > f32::EPSILON {
        apply_scale_to_subtree(doc, node_id, container_rect.x, container_rect.y, style.transform_scale);
    }
}

fn apply_scale_to_subtree(doc: &mut CxrdDocument, root_id: NodeId, origin_x: f32, origin_y: f32, scale: f32) {
    let mut stack = vec![root_id];
    while let Some(node_id) = stack.pop() {
        if let Some(node) = doc.nodes.get_mut(node_id as usize) {
            let rect = &mut node.layout.rect;
            rect.x = origin_x + (rect.x - origin_x) * scale;
            rect.y = origin_y + (rect.y - origin_y) * scale;
            rect.width *= scale;
            rect.height *= scale;

            let content = &mut node.layout.content_rect;
            content.x = origin_x + (content.x - origin_x) * scale;
            content.y = origin_y + (content.y - origin_y) * scale;
            content.width *= scale;
            content.height *= scale;

            if let Some(clip) = &mut node.layout.clip {
                clip.x = origin_x + (clip.x - origin_x) * scale;
                clip.y = origin_y + (clip.y - origin_y) * scale;
                clip.width *= scale;
                clip.height *= scale;
            }

            for &child in &node.children {
                stack.push(child);
            }
        }
    }
}

/// Layout children using CSS Grid.
///
/// Implements a subset of CSS Grid Layout:
///   - grid-template-columns / grid-template-rows with px, fr, %, auto sizes
///   - grid-column / grid-row child placement (including negative line numbers)
///   - Auto-placement for children without explicit positioning
///   - Auto-sized rows expand to fit content
fn layout_grid_children(
    doc: &mut CxrdDocument,
    parent_id: NodeId,
    container_rect: Rect,
    child_ids: &[NodeId],
    constraints: &LayoutConstraints,
) {
    let parent_style = doc.nodes[parent_id as usize].style.clone();
    let gap = parent_style.gap;
    let col_templates = &parent_style.grid_template_columns;
    let row_templates = &parent_style.grid_template_rows;

    // Collect non-absolute children.
    let grid_children: Vec<NodeId> = child_ids.iter()
        .copied()
        .filter(|&cid| {
            doc.nodes.get(cid as usize)
                .map(|c| !matches!(c.style.position, Position::Absolute | Position::Fixed)
                      && !matches!(c.style.display, Display::None))
                .unwrap_or(false)
        })
        .collect();

    if grid_children.is_empty() {
        return;
    }

    let num_cols = col_templates.len().max(1);
    // Determine number of rows: at least what the template says, but may grow for auto-placement.
    let mut num_rows = row_templates.len().max(1);

    // Phase 1: Assign children to grid cells.
    // Each child gets (col_start, col_end, row_start, row_end) in 0-based indices.
    struct GridPlacement {
        node_id: NodeId,
        col_start: usize,
        col_end: usize,
        row_start: usize,
        row_end: usize,
    }

    let mut placements: Vec<GridPlacement> = Vec::with_capacity(grid_children.len());
    let mut auto_cursor_col = 0usize;
    let mut auto_cursor_row = 0usize;

    for &cid in &grid_children {
        let cs = doc.nodes[cid as usize].style.clone();
        let cs_start = cs.grid_column_start;
        let cs_end = cs.grid_column_end;
        let rs_start = cs.grid_row_start;
        let rs_end = cs.grid_row_end;

        // Resolve column placement.
        let (col_s, col_e) = resolve_grid_lines(cs_start, cs_end, num_cols);
        // Resolve row placement.
        let (row_s, row_e) = resolve_grid_lines(rs_start, rs_end, num_rows);

        if col_s.is_some() && row_s.is_some() {
            // Explicitly placed.
            let c0 = col_s.unwrap();
            let c1 = col_e.unwrap_or(c0 + 1);
            let r0 = row_s.unwrap();
            let r1 = row_e.unwrap_or(r0 + 1);
            // Grow rows if needed.
            if r1 > num_rows { num_rows = r1; }
            placements.push(GridPlacement { node_id: cid, col_start: c0, col_end: c1, row_start: r0, row_end: r1 });
        } else if col_s.is_some() {
            // Column specified, row auto-placed.
            // First, advance row if previous row was filled by auto-placed items.
            if auto_cursor_col >= num_cols {
                auto_cursor_col = 0;
                auto_cursor_row += 1;
            }
            let c0 = col_s.unwrap();
            let c1 = col_e.unwrap_or(c0 + 1);
            let r0 = auto_cursor_row;
            let r1 = row_e.map(|re| r0 + (re.saturating_sub(row_s.unwrap_or(0)))).unwrap_or(r0 + 1);
            if r1 > num_rows { num_rows = r1; }
            placements.push(GridPlacement { node_id: cid, col_start: c0, col_end: c1, row_start: r0, row_end: r1 });
            // If this spanning item covers all columns, advance to next row.
            if c0 == 0 && c1 >= num_cols {
                auto_cursor_row = r1;
                auto_cursor_col = 0;
            }
        } else {
            // Fully auto-placed.
            if auto_cursor_col >= num_cols {
                auto_cursor_col = 0;
                auto_cursor_row += 1;
            }
            let c0 = auto_cursor_col;
            let c1 = c0 + 1;
            let r0 = auto_cursor_row;
            let r1 = r0 + 1;
            if r1 > num_rows { num_rows = r1; }
            placements.push(GridPlacement { node_id: cid, col_start: c0, col_end: c1, row_start: r0, row_end: r1 });
            auto_cursor_col = c1;
        }
    }


    // Phase 2: Resolve track sizes.
    // 2a. Resolve column widths.
    let total_col_gaps = if num_cols > 1 { gap * (num_cols - 1) as f32 } else { 0.0 };
    let available_width = container_rect.width - total_col_gaps;
    let col_widths = resolve_track_sizes(col_templates, available_width, num_cols);

    // 2b. Resolve row heights.
    // First pass: resolve fixed rows, estimate auto rows from content.
    let total_row_gaps = if num_rows > 1 { gap * (num_rows - 1) as f32 } else { 0.0 };
    let available_height = container_rect.height - total_row_gaps;

    // For auto rows, estimate content height.
    let mut row_heights = vec![0.0f32; num_rows];
    let mut row_is_fr = vec![false; num_rows];
    let mut total_fr_row: f32 = 0.0;
    let mut fixed_row_height: f32 = 0.0;

    for r in 0..num_rows {
        let template = row_templates.get(r);
        match template {
            Some(GridTrackSize::Px(v)) => {
                row_heights[r] = *v;
                fixed_row_height += *v;
            }
            Some(GridTrackSize::Percent(v)) => {
                row_heights[r] = available_height * *v / 100.0;
                fixed_row_height += row_heights[r];
            }
            Some(GridTrackSize::Fr(v)) => {
                row_is_fr[r] = true;
                total_fr_row += *v;
            }
            Some(GridTrackSize::Auto) | Some(GridTrackSize::MinContent) | Some(GridTrackSize::MaxContent) | None => {
                // Auto row: estimate from children placed in this row.
                let mut max_h: f32 = 0.0;
                for pl in &placements {
                    if pl.row_start <= r && pl.row_end > r && pl.row_end - pl.row_start == 1 {
                        let child_h = estimate_content_height(doc, pl.node_id, constraints);
                        max_h = max_h.max(child_h);
                    }
                }
                row_heights[r] = max_h.max(0.0);
                fixed_row_height += row_heights[r];
            }
        }
    }

    // Distribute remaining height among fr rows.
    let remaining_h = (available_height - fixed_row_height).max(0.0);
    if total_fr_row > 0.0 {
        for r in 0..num_rows {
            if row_is_fr[r] {
                let fr_val = match row_templates.get(r) {
                    Some(GridTrackSize::Fr(v)) => *v,
                    _ => 1.0,
                };
                row_heights[r] = remaining_h * (fr_val / total_fr_row);
            }
        }
    }

    // Phase 3: Compute cumulative offsets for column and row start positions.
    let mut col_offsets = vec![0.0f32; num_cols + 1];
    for c in 0..num_cols {
        col_offsets[c + 1] = col_offsets[c] + col_widths[c] + if c < num_cols - 1 { gap } else { 0.0 };
    }
    let mut row_offsets = vec![0.0f32; num_rows + 1];
    for r in 0..num_rows {
        row_offsets[r + 1] = row_offsets[r] + row_heights[r] + if r < num_rows - 1 { gap } else { 0.0 };
    }

    // Phase 4: Position each child in its grid cell.
    for pl in &placements {
        let cs = doc.nodes[pl.node_id as usize].style.clone();
        let resolve = |d: Dimension, parent: f32| -> f32 {
            d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
        };

        // Cell area.
        let cell_x = container_rect.x + col_offsets[pl.col_start];
        let cell_y = container_rect.y + row_offsets[pl.row_start];
        let cell_w = col_offsets[pl.col_end.min(num_cols)] - col_offsets[pl.col_start]
            - if pl.col_end < num_cols { gap } else { 0.0 };
        let cell_h = row_offsets[pl.row_end.min(num_rows)] - row_offsets[pl.row_start]
            - if pl.row_end < num_rows { gap } else { 0.0 };

        let margin = EdgeInsets {
            top: resolve(cs.margin.top, cell_h),
            right: resolve(cs.margin.right, cell_w),
            bottom: resolve(cs.margin.bottom, cell_h),
            left: resolve(cs.margin.left, cell_w),
        };
        let padding = EdgeInsets {
            top: resolve(cs.padding.top, cell_h),
            right: resolve(cs.padding.right, cell_w),
            bottom: resolve(cs.padding.bottom, cell_h),
            left: resolve(cs.padding.left, cell_w),
        };
        let border = cs.border_width;

        // Child width/height: use explicit if set, else fill cell.
        let w = if !cs.width.is_auto() {
            resolve(cs.width, cell_w)
        } else {
            (cell_w - margin.horizontal()).max(0.0)
        };
        let h = if !cs.height.is_auto() {
            resolve(cs.height, cell_h)
        } else {
            (cell_h - margin.vertical()).max(0.0)
        };

        let x = cell_x + margin.left;
        let y = cell_y + margin.top;

        let node = &mut doc.nodes[pl.node_id as usize];
        node.layout.rect = Rect { x, y, width: w, height: h };
        node.layout.content_rect = Rect {
            x: x + padding.left + border.left,
            y: y + padding.top + border.top,
            width: (w - padding.horizontal() - border.horizontal()).max(0.0),
            height: (h - padding.vertical() - border.vertical()).max(0.0),
        };
        node.layout.padding = padding;
        node.layout.margin = margin;
    }
}

/// Resolve grid line numbers to 0-based track indices.
/// CSS grid lines are 1-based; negative values count from the end.
/// Returns (start_index, end_index) where None means auto.
fn resolve_grid_lines(start: i32, end: i32, num_tracks: usize) -> (Option<usize>, Option<usize>) {
    let resolve_line = |line: i32, num: usize| -> Option<usize> {
        if line == 0 {
            None // auto
        } else if line > 0 {
            Some((line as usize - 1).min(num))
        } else {
            // Negative: count from end. -1 = last line = num_tracks
            let from_end = (-line) as usize;
            if from_end <= num + 1 {
                Some((num + 1).saturating_sub(from_end))
            } else {
                Some(0)
            }
        }
    };

    // Handle span encoding (value > 1000 means "span N" where N = value - 1000)
    let s = resolve_line(start, num_tracks);
    let e = if end > 1000 {
        // span encoding
        let span = (end - 1000) as usize;
        s.map(|sv| sv + span)
    } else {
        resolve_line(end, num_tracks)
    };

    (s, e)
}

/// Resolve column/row track sizes from templates.
/// Handles Px, Percent, Fr, Auto.
fn resolve_track_sizes(templates: &[GridTrackSize], available: f32, num_tracks: usize) -> Vec<f32> {
    let mut sizes = vec![0.0f32; num_tracks];
    let mut total_fr: f32 = 0.0;
    let mut fixed_total: f32 = 0.0;

    for i in 0..num_tracks {
        match templates.get(i) {
            Some(GridTrackSize::Px(v)) => {
                sizes[i] = *v;
                fixed_total += *v;
            }
            Some(GridTrackSize::Percent(v)) => {
                sizes[i] = available * *v / 100.0;
                fixed_total += sizes[i];
            }
            Some(GridTrackSize::Fr(v)) => {
                total_fr += *v;
            }
            Some(GridTrackSize::Auto) | Some(GridTrackSize::MinContent) | Some(GridTrackSize::MaxContent) | None => {
                // Auto columns: give a minimum size; will be adjusted later if needed.
                // For now, treat as 0 and let fr take over.
                // If there are no fr tracks, auto columns share remaining space equally.
                sizes[i] = 0.0;
            }
        }
    }

    // Distribute remaining space among fr tracks.
    let remaining = (available - fixed_total).max(0.0);
    if total_fr > 0.0 {
        for i in 0..num_tracks {
            if let Some(GridTrackSize::Fr(v)) = templates.get(i) {
                sizes[i] = remaining * (*v / total_fr);
            }
        }
    } else {
        // No fr tracks: distribute remaining among auto tracks.
        let auto_count = (0..num_tracks).filter(|i| {
            matches!(templates.get(*i), Some(GridTrackSize::Auto) | Some(GridTrackSize::MinContent) | Some(GridTrackSize::MaxContent) | None)
        }).count();
        if auto_count > 0 {
            let per_auto = remaining / auto_count as f32;
            for i in 0..num_tracks {
                if matches!(templates.get(i), Some(GridTrackSize::Auto) | Some(GridTrackSize::MinContent) | Some(GridTrackSize::MaxContent) | None) {
                    sizes[i] = per_auto;
                }
            }
        }
    }

    sizes
}

/// Estimate the content height of a node for auto-sizing.
/// Recursively sums children heights or uses font metrics for text.
fn estimate_content_height(doc: &CxrdDocument, node_id: NodeId, constraints: &LayoutConstraints) -> f32 {
    let node = match doc.nodes.get(node_id as usize) {
        Some(n) => n,
        None => return 0.0,
    };

    let cs = &node.style;
    let resolve = |d: Dimension, parent: f32| -> f32 {
        d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
    };

    // If height is explicitly set, use it.
    if !cs.height.is_auto() {
        return resolve(cs.height, constraints.viewport_height);
    }

    let padding_v = resolve(cs.padding.top, 0.0) + resolve(cs.padding.bottom, 0.0);
    let border_v = cs.border_width.top + cs.border_width.bottom;
    let margin_v = resolve(cs.margin.top, 0.0) + resolve(cs.margin.bottom, 0.0);

    if node.children.is_empty() {
        // Leaf node: estimate intrinsic height.
        if let NodeKind::Input(input) = &node.kind {
            let intrinsic = match input {
                InputKind::TextArea { rows, .. } => (cs.font_size * cs.line_height * (*rows).max(1) as f32).max(30.0),
                InputKind::TabBar { .. } => 30.0,
                InputKind::Checkbox { .. } => 22.0,
                InputKind::Slider { .. } => 22.0,
                _ => 30.0,
            };
            return padding_v + border_v + margin_v + intrinsic;
        }

        // Text leaf fallback.
        let font_size = cs.font_size.max(1.0);
        let line_h = cs.line_height * font_size;
        return padding_v + border_v + margin_v + line_h;
    }

    // Sum children heights (block flow) or max (flex-row).
    let gap = cs.gap;
    let is_flex_row = matches!(cs.display, Display::Flex)
        && matches!(cs.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);
    let is_wrap_row = is_flex_row && !matches!(cs.flex_wrap, crate::cxrd::style::FlexWrap::NoWrap);
    let is_paragraph_like = node.tag.as_deref() == Some("p");
    let mut total_h: f32 = 0.0;
    let mut max_h: f32 = 0.0;
    let mut count = 0;
    for &child_id in &node.children {
        if let Some(child) = doc.nodes.get(child_id as usize) {
            if matches!(child.style.display, Display::None) || matches!(child.style.position, Position::Absolute | Position::Fixed) {
                continue;
            }
        }
        let ch = estimate_content_height(doc, child_id, constraints);
        total_h += ch;
        if ch > max_h { max_h = ch; }
        count += 1;
    }
    if count > 1 && (!is_flex_row || (is_wrap_row && is_paragraph_like)) {
        total_h += gap * (count - 1) as f32;
    }

    let children_h = if is_flex_row && !(is_wrap_row && is_paragraph_like) { max_h } else { total_h };
    padding_v + border_v + margin_v + children_h
}

/// Estimate the intrinsic content width of a node (for flex-row auto-basis).
///
/// For leaf text/data-bound nodes, uses a heuristic based on font metrics.
/// For containers, sums children widths (flex-row) or takes max (block/column).
fn estimate_content_width(doc: &CxrdDocument, node_id: NodeId, constraints: &LayoutConstraints) -> f32 {
    let node = match doc.nodes.get(node_id as usize) {
        Some(n) => n,
        None => return 0.0,
    };

    let cs = &node.style;
    let resolve = |d: Dimension, parent: f32| -> f32 {
        d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
    };

    // If width is explicitly set, use it.
    if !cs.width.is_auto() {
        return resolve(cs.width, constraints.viewport_width);
    }

    let padding_h = resolve(cs.padding.left, 0.0) + resolve(cs.padding.right, 0.0);
    let border_h = cs.border_width.left + cs.border_width.right;
    let margin_h = resolve(cs.margin.left, 0.0) + resolve(cs.margin.right, 0.0);

    if node.children.is_empty() {
        // Leaf node: estimate intrinsic width from control type/content.
        if let NodeKind::Input(input) = &node.kind {
            let font_size = cs.font_size.max(1.0);
            let char_width = font_size * 0.62;
            let intrinsic = match input {
                InputKind::Button { label, .. } => (label.len() as f32 * char_width) + 20.0,
                InputKind::TextInput { value, placeholder, .. } => {
                    let txt = if value.is_empty() { placeholder } else { value };
                    (txt.len().max(8) as f32 * char_width) + 24.0
                }
                InputKind::Dropdown { selected, options, placeholder, .. } => {
                    let txt_len = if let Some(sel) = selected {
                        options
                            .iter()
                            .find(|o| &o.0 == sel)
                            .map(|o| o.1.len())
                            .unwrap_or(sel.len())
                    } else {
                        placeholder.len().max(6)
                    };
                    (txt_len as f32 * char_width) + 34.0
                }
                InputKind::Checkbox { label, .. } => (label.len() as f32 * char_width) + 28.0,
                InputKind::Slider { .. } => 120.0,
                InputKind::ColorPicker { .. } => 46.0,
                InputKind::TextArea { value, placeholder, .. } => {
                    let txt = if value.is_empty() { placeholder } else { value };
                    (txt.len().max(12) as f32 * char_width) + 24.0
                }
                _ => 80.0,
            };
            return padding_h + border_h + margin_h + intrinsic;
        }

        // Text leaf fallback.
        let font_size = cs.font_size.max(1.0);
        // Average character width ~0.65 × font-size for a proportional sans-serif.
        // 0.55 was too narrow and caused labels with wide glyphs (M, W, N) to clip.
        let char_width = font_size * 0.65;
        let text_chars = match &node.kind {
            NodeKind::Text { content } => content.len(),
            _ => 0,
        };
        return padding_h + border_h + margin_h + (text_chars as f32 * char_width);
    }

    // Container node.
    let is_flex_row = matches!(cs.display, Display::Flex)
        && matches!(cs.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);
    let gap = cs.gap;
    let mut total_w: f32 = 0.0;
    let mut max_w: f32 = 0.0;
    let mut count = 0;
    for &child_id in &node.children {
        if let Some(child) = doc.nodes.get(child_id as usize) {
            if matches!(child.style.display, Display::None)
                || matches!(child.style.position, Position::Absolute | Position::Fixed)
            {
                continue;
            }
        }
        let cw = estimate_content_width(doc, child_id, constraints);
        total_w += cw;
        if cw > max_w { max_w = cw; }
        count += 1;
    }
    if count > 1 && is_flex_row {
        total_w += gap * (count - 1) as f32;
    }

    let children_w = if is_flex_row { total_w } else { max_w };
    padding_h + border_h + margin_h + children_w
}

/// Layout children using flexbox.
fn layout_flex_children(
    doc: &mut CxrdDocument,
    parent_id: NodeId,
    container_rect: Rect,
    child_ids: &[NodeId],
    constraints: &LayoutConstraints,
) {
    // We need to temporarily extract children to mutate them together with the parent.
    // Since they're all in the same Vec, we use index-based access.
    // First, initialize child rects.
    for &cid in child_ids {
        if let Some(child) = doc.nodes.get(cid as usize) {
            let cs = &child.style;
            if matches!(cs.position, Position::Absolute | Position::Fixed) {
                continue; // Handled separately.
            }
        }
    }

    // Collect non-absolute child IDs for flex layout.
    let flex_children: Vec<NodeId> = child_ids.iter()
        .copied()
        .filter(|&cid| {
            doc.nodes.get(cid as usize)
                .map(|c| !matches!(c.style.position, Position::Absolute | Position::Fixed))
                .unwrap_or(false)
        })
        .collect();

    if flex_children.is_empty() {
        return;
    }

    // We'll do the flex computation using extracted data then write back.
    // Extract the parent style and children as a separate working set.
    let parent_style = doc.nodes[parent_id as usize].style.clone();

    // Pre-resolve child sizes and flex properties, then use a simplified
    // inline flex algorithm (since we can't pass &mut [&mut CxrdNode]).
    let gap = parent_style.gap;
    let dir = parent_style.flex_direction;
    let is_row = matches!(dir, crate::cxrd::style::FlexDirection::Row | crate::cxrd::style::FlexDirection::RowReverse);
    let main_size = if is_row { container_rect.width } else { container_rect.height };
    let cross_size = if is_row { container_rect.height } else { container_rect.width };

    struct ItemData {
        base_main: f32,
        base_cross: f32,
        min_main: f32,
        max_main: f32,
        flex_grow: f32,
        flex_shrink: f32,
        m_start: f32,
        m_end: f32,
        c_start: f32,
        c_end: f32,
        padding: EdgeInsets,
        border: EdgeInsets,
        align: crate::cxrd::style::AlignItems,
    }

    let mut items: Vec<ItemData> = Vec::with_capacity(flex_children.len());
    for &cid in &flex_children {
        let cs = &doc.nodes[cid as usize].style;
        let resolve = |d: Dimension, parent: f32| -> f32 {
            d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
        };

        let margin = EdgeInsets {
            top: resolve(cs.margin.top, container_rect.height),
            right: resolve(cs.margin.right, container_rect.width),
            bottom: resolve(cs.margin.bottom, container_rect.height),
            left: resolve(cs.margin.left, container_rect.width),
        };
        let padding = EdgeInsets {
            top: resolve(cs.padding.top, container_rect.height),
            right: resolve(cs.padding.right, container_rect.width),
            bottom: resolve(cs.padding.bottom, container_rect.height),
            left: resolve(cs.padding.left, container_rect.width),
        };
        let border = cs.border_width;

        let (m_start, m_end, c_start, c_end) = if is_row {
            (margin.left, margin.right, margin.top, margin.bottom)
        } else {
            (margin.top, margin.bottom, margin.left, margin.right)
        };

        let basis = if !cs.flex_basis.is_auto() {
            resolve(cs.flex_basis, main_size)
        } else if is_row && !cs.width.is_auto() {
            resolve(cs.width, container_rect.width)
        } else if !is_row && !cs.height.is_auto() {
            resolve(cs.height, container_rect.height)
        } else {
            // Auto basis: use intrinsic content size.
            if !is_row {
                estimate_content_height(doc, cid, constraints)
            } else {
                estimate_content_width(doc, cid, constraints)
            }
        };

        let cross = if is_row && !cs.height.is_auto() {
            resolve(cs.height, container_rect.height)
        } else if !is_row && !cs.width.is_auto() {
            resolve(cs.width, container_rect.width)
        } else {
            0.0
        };

        let align = if cs.align_self != crate::cxrd::style::AlignSelf::Auto {
            match cs.align_self {
                crate::cxrd::style::AlignSelf::FlexStart => crate::cxrd::style::AlignItems::FlexStart,
                crate::cxrd::style::AlignSelf::FlexEnd => crate::cxrd::style::AlignItems::FlexEnd,
                crate::cxrd::style::AlignSelf::Center => crate::cxrd::style::AlignItems::Center,
                crate::cxrd::style::AlignSelf::Stretch => crate::cxrd::style::AlignItems::Stretch,
                crate::cxrd::style::AlignSelf::Auto => parent_style.align_items,
            }
        } else {
            parent_style.align_items
        };

        let min_main = if is_row {
            if cs.min_width.is_auto() { 0.0 } else { resolve(cs.min_width, container_rect.width) }
        } else {
            if cs.min_height.is_auto() { 0.0 } else { resolve(cs.min_height, container_rect.height) }
        };
        let max_main = if is_row {
            if cs.max_width.is_auto() { f32::INFINITY } else { resolve(cs.max_width, container_rect.width) }
        } else {
            if cs.max_height.is_auto() { f32::INFINITY } else { resolve(cs.max_height, container_rect.height) }
        };

        items.push(ItemData {
            base_main: basis,
            base_cross: cross,
            min_main,
            max_main,
            flex_grow: cs.flex_grow,
            flex_shrink: cs.flex_shrink,
            m_start, m_end, c_start, c_end,
            padding, border, align,
        });
    }

    // ── Wrap: group items into lines ──────────────────────────────────
    let wrap = parent_style.flex_wrap;
    let lines: Vec<Vec<usize>> = if matches!(wrap, crate::cxrd::style::FlexWrap::NoWrap) {
        // Single line containing all items.
        vec![(0..items.len()).collect()]
    } else {
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut used = 0.0f32;
        for i in 0..items.len() {
            let item_main = items[i].base_main + items[i].m_start + items[i].m_end;
            let gap_before = if cur.is_empty() { 0.0 } else { gap };
            if !cur.is_empty() && used + gap_before + item_main > main_size {
                lines.push(std::mem::take(&mut cur));
                used = item_main;
                cur.push(i);
            } else {
                used += gap_before + item_main;
                cur.push(i);
            }
        }
        if !cur.is_empty() { lines.push(cur); }
        if matches!(wrap, crate::cxrd::style::FlexWrap::WrapReverse) {
            lines.reverse();
        }
        lines
    };

    let num_lines = lines.len().max(1);
    let line_cross = cross_size / num_lines as f32;
    let mut cross_pos = 0.0f32;

    for line in &lines {
        if line.is_empty() { continue; }

        // Flex distribution for this line
        let num_gaps_l = if line.len() > 1 { (line.len() - 1) as f32 } else { 0.0 };
        let total_gap_l = gap * num_gaps_l;
        let total_base_l: f32 = line.iter().map(|&i| items[i].base_main + items[i].m_start + items[i].m_end).sum();
        let free_l = main_size - total_base_l - total_gap_l;
        let total_grow_l: f32 = line.iter().map(|&i| items[i].flex_grow).sum();
        let total_shrink_l: f32 = line.iter().map(|&i| items[i].flex_shrink * items[i].base_main).sum();
        let pure_base_l: f32 = line.iter().map(|&i| items[i].base_main).sum();

        let finals_l: Vec<f32> = line.iter().map(|&idx| {
            let item = &items[idx];
            let mut sz = item.base_main;
            if free_l > 0.0 && total_grow_l > 0.0 {
                sz += free_l * (item.flex_grow / total_grow_l);
            } else if free_l < 0.0 && total_shrink_l > 0.0 {
                sz += free_l * (item.flex_shrink * item.base_main / total_shrink_l);
            } else if free_l > 0.0 && total_grow_l == 0.0 && pure_base_l == 0.0 && !line.is_empty() {
                sz = free_l / line.len() as f32;
            }
            sz.max(item.min_main).min(item.max_main).max(0.0)
        }).collect();

        // Justify-content for this line
        let used_l: f32 = finals_l.iter().sum::<f32>()
            + line.iter().map(|&i| items[i].m_start + items[i].m_end).sum::<f32>()
            + total_gap_l;
        let remaining_l = (main_size - used_l).max(0.0);

        let (mut offset, extra_gap) = match parent_style.justify_content {
            crate::cxrd::style::JustifyContent::FlexStart => (0.0, 0.0),
            crate::cxrd::style::JustifyContent::FlexEnd => (remaining_l, 0.0),
            crate::cxrd::style::JustifyContent::Center => (remaining_l / 2.0, 0.0),
            crate::cxrd::style::JustifyContent::SpaceBetween => {
                if line.len() > 1 { (0.0, remaining_l / (line.len() - 1) as f32) } else { (0.0, 0.0) }
            }
            crate::cxrd::style::JustifyContent::SpaceAround => {
                let sp = remaining_l / line.len() as f32;
                (sp / 2.0, sp)
            }
            crate::cxrd::style::JustifyContent::SpaceEvenly => {
                let sp = remaining_l / (line.len() + 1) as f32;
                (sp, sp)
            }
        };

        // Position each child in this line
        for (li, &idx) in line.iter().enumerate() {
            let item = &items[idx];
            let m = finals_l[li];
            let cid = flex_children[idx];

            let c = if item.base_cross > 0.0 {
                item.base_cross
            } else if matches!(item.align, crate::cxrd::style::AlignItems::Stretch) {
                line_cross - item.c_start - item.c_end
            } else {
                let intrinsic = if is_row {
                    estimate_content_height(doc, cid, constraints)
                } else {
                    estimate_content_width(doc, cid, constraints)
                };
                intrinsic.min(line_cross - item.c_start - item.c_end)
            };

            let item_cross_offset = match item.align {
                crate::cxrd::style::AlignItems::FlexStart => item.c_start,
                crate::cxrd::style::AlignItems::FlexEnd => line_cross - c - item.c_end,
                crate::cxrd::style::AlignItems::Center => (line_cross - c) / 2.0,
                _ => item.c_start,
            };

            offset += item.m_start;

            let (x, y, w, h) = if is_row {
                (container_rect.x + offset, container_rect.y + cross_pos + item_cross_offset, m, c)
            } else {
                (container_rect.x + cross_pos + item_cross_offset, container_rect.y + offset, c, m)
            };

            let node = &mut doc.nodes[cid as usize];
            node.layout.rect = Rect { x, y, width: w, height: h };
            node.layout.content_rect = Rect {
                x: x + item.padding.left + item.border.left,
                y: y + item.padding.top + item.border.top,
                width: (w - item.padding.horizontal() - item.border.horizontal()).max(0.0),
                height: (h - item.padding.vertical() - item.border.vertical()).max(0.0),
            };
            node.layout.padding = item.padding;

            offset += m + item.m_end + gap + extra_gap;
        }

        cross_pos += line_cross;
    }
}

/// Layout children using simple block flow (stack vertically).
fn layout_block_children(
    doc: &mut CxrdDocument,
    container_rect: Rect,
    child_ids: &[NodeId],
    constraints: &LayoutConstraints,
) {
    let mut y_cursor = container_rect.y;

    for &cid in child_ids {
        let cs = doc.nodes[cid as usize].style.clone();
        if matches!(cs.display, Display::None) || matches!(cs.position, Position::Absolute | Position::Fixed) {
            continue;
        }

        let resolve = |d: Dimension, parent: f32| -> f32 {
            d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
        };

        let margin = EdgeInsets {
            top: resolve(cs.margin.top, container_rect.height),
            right: resolve(cs.margin.right, container_rect.width),
            bottom: resolve(cs.margin.bottom, container_rect.height),
            left: resolve(cs.margin.left, container_rect.width),
        };
        let padding = EdgeInsets {
            top: resolve(cs.padding.top, container_rect.height),
            right: resolve(cs.padding.right, container_rect.width),
            bottom: resolve(cs.padding.bottom, container_rect.height),
            left: resolve(cs.padding.left, container_rect.width),
        };
        let border = cs.border_width;

        let w = if !cs.width.is_auto() {
            resolve(cs.width, container_rect.width)
        } else {
            container_rect.width - margin.horizontal()
        };

        let h = if !cs.height.is_auto() {
            resolve(cs.height, container_rect.height)
        } else {
            // Auto height: estimate from content.
            // For containers with children, estimate recursively.
            // For leaf nodes, use text line height.
            estimate_content_height(doc, cid, constraints)
        };

        y_cursor += margin.top;

        let node = &mut doc.nodes[cid as usize];
        node.layout.rect = Rect {
            x: container_rect.x + margin.left,
            y: y_cursor,
            width: w,
            height: h,
        };
        node.layout.content_rect = Rect {
            x: container_rect.x + margin.left + padding.left + border.left,
            y: y_cursor + padding.top + border.top,
            width: (w - padding.horizontal() - border.horizontal()).max(0.0),
            height: (h - padding.vertical() - border.vertical()).max(0.0),
        };
        node.layout.padding = padding;
        node.layout.margin = margin;

        y_cursor += h + margin.bottom;
    }
}

/// Layout an absolutely-positioned child within its containing block.
/// Returns `(shrink_w, shrink_h)` — whether each dimension should be
/// shrunk to content after recursive layout (true when the dimension is
/// `auto` and NOT constrained by opposing insets).
fn layout_absolute_child(
    doc: &mut CxrdDocument,
    child_id: NodeId,
    containing: Rect,
    constraints: &LayoutConstraints,
) -> (bool, bool) {
    let cs = doc.nodes[child_id as usize].style.clone();

    let resolve = |d: Dimension, parent: f32| -> f32 {
        d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
    };

    let padding = EdgeInsets {
        top: resolve(cs.padding.top, containing.height),
        right: resolve(cs.padding.right, containing.width),
        bottom: resolve(cs.padding.bottom, containing.height),
        left: resolve(cs.padding.left, containing.width),
    };
    let border = cs.border_width;

    // Width: explicit > opposing insets > right-anchor intrinsic > shrink-to-fit
    let (w, shrink_w) = if !cs.width.is_auto() {
        (resolve(cs.width, containing.width), false)
    } else if !cs.left.is_auto() && !cs.right.is_auto() {
        let l = resolve(cs.left, containing.width);
        let r = resolve(cs.right, containing.width);
        ((containing.width - l - r).max(0.0), false)
    } else if cs.left.is_auto() && !cs.right.is_auto() {
        // Only 'right' is set: compute intrinsic width now so x is correct from the start.
        let intrinsic = estimate_content_width(doc, child_id, constraints);
        (intrinsic.max(0.0), false)
    } else {
        // Shrink-to-fit: use containing width as max constraint for now;
        // we will shrink after recursive layout.
        (containing.width, true)
    };

    // Height: explicit > opposing insets > bottom-anchor intrinsic > shrink-to-fit
    let (h, shrink_h) = if !cs.height.is_auto() {
        (resolve(cs.height, containing.height), false)
    } else if !cs.top.is_auto() && !cs.bottom.is_auto() {
        let t = resolve(cs.top, containing.height);
        let b = resolve(cs.bottom, containing.height);
        ((containing.height - t - b).max(0.0), false)
    } else if cs.top.is_auto() && !cs.bottom.is_auto() {
        // Only 'bottom' is set: compute intrinsic height now so y is correct from the start.
        let intrinsic = estimate_content_height(doc, child_id, constraints);
        (intrinsic.max(0.0), false)
    } else {
        (containing.height, true)
    };

    let x = if !cs.left.is_auto() {
        containing.x + resolve(cs.left, containing.width)
    } else if !cs.right.is_auto() {
        containing.x + containing.width - resolve(cs.right, containing.width) - w
    } else {
        containing.x
    };

    let y = if !cs.top.is_auto() {
        containing.y + resolve(cs.top, containing.height)
    } else if !cs.bottom.is_auto() {
        containing.y + containing.height - resolve(cs.bottom, containing.height) - h
    } else {
        containing.y
    };

    let node = &mut doc.nodes[child_id as usize];
    node.layout.rect = Rect { x, y, width: w, height: h };
    node.layout.content_rect = Rect {
        x: x + padding.left + border.left,
        y: y + padding.top + border.top,
        width: (w - padding.horizontal() - border.horizontal()).max(0.0),
        height: (h - padding.vertical() - border.vertical()).max(0.0),
    };
    node.layout.padding = padding;

    (shrink_w, shrink_h)
}

/// After recursive layout, shrink an absolute/fixed element's rect to
/// fit its children when the dimension was auto-sized without opposing insets.
fn shrink_absolute_to_content(
    doc: &mut CxrdDocument,
    node_id: NodeId,
    shrink_w: bool,
    shrink_h: bool,
    constraints: &LayoutConstraints,
) {
    if !shrink_w && !shrink_h {
        return;
    }

    let node = &doc.nodes[node_id as usize];
    let rect = node.layout.rect;

    // Use intrinsic content estimates instead of laid-out children bounds.
    // Block-flow children expand to fill the initially-large absolute
    // container, making their laid-out rects useless for shrink-to-fit.
    // The estimate functions compute intrinsic sizes from text content
    // and explicit dimensions, which is what shrink-to-fit needs.
    let mut max_child_w: f32 = 0.0;
    let mut total_child_h: f32 = 0.0;
    let mut child_count: usize = 0;

    for &child_id in &node.children {
        if let Some(child) = doc.nodes.get(child_id as usize) {
            if matches!(child.style.display, Display::None) {
                continue;
            }
        }
        if shrink_w {
            let cw = estimate_content_width(doc, child_id, constraints);
            if cw > max_child_w { max_child_w = cw; }
        }
        if shrink_h {
            total_child_h += estimate_content_height(doc, child_id, constraints);
            child_count += 1;
        }
    }

    let gap = doc.nodes[node_id as usize].style.gap;
    if child_count > 1 {
        total_child_h += gap * (child_count - 1) as f32;
    }

    let node = &mut doc.nodes[node_id as usize];
    let pad_h = node.layout.padding.horizontal() + node.style.border_width.horizontal();
    let pad_v = node.layout.padding.vertical() + node.style.border_width.vertical();

    if shrink_w {
        let new_w = max_child_w + pad_h;
        node.layout.rect.width = new_w;
        node.layout.content_rect.width = (new_w - pad_h).max(0.0);

        // Re-anchor x if positioned from the right.
        if node.style.left.is_auto() && !node.style.right.is_auto() {
            let old_x = node.layout.rect.x;
            let shift = rect.width - new_w;
            node.layout.rect.x = old_x + shift;
            node.layout.content_rect.x += shift;
        }
    }
    if shrink_h {
        let new_h = total_child_h + pad_v;
        node.layout.rect.height = new_h;
        node.layout.content_rect.height = (new_h - pad_v).max(0.0);

        if node.style.top.is_auto() && !node.style.bottom.is_auto() {
            let old_y = node.layout.rect.y;
            let shift = rect.height - new_h;
            node.layout.rect.y = old_y + shift;
            node.layout.content_rect.y += shift;
        }
    }
}
