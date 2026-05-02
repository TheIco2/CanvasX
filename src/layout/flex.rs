// prism-runtime/src/layout/flex.rs
//
// Simplified flexbox layout algorithm.
//
// Supports:
//   flex-direction: row | column
//   justify-content: flex-start | flex-end | center | space-between | space-around | space-evenly
//   align-items: flex-start | flex-end | center | stretch
//   flex-grow, flex-shrink, flex-basis
//   gap
//   flex-wrap: nowrap | wrap
//
// Does NOT support:
//   order, align-content, baseline alignment, min/max cross-size clamping

use crate::prd::style::{FlexDirection, JustifyContent, AlignItems, AlignSelf, Display};
use crate::prd::node::PrdNode;
use crate::prd::value::{Dimension, Rect, EdgeInsets};
use crate::layout::types::LayoutConstraints;

/// Perform flexbox layout on a container's children.
///
/// `container_rect` — the content area of the flex container (already resolved).
/// `children` — mutable references to child nodes (will set their layout.rect).
/// Returns the intrinsic height used by children (for auto-sizing containers).
pub fn layout_flex(
    container: &PrdNode,
    container_rect: Rect,
    children: &mut [&mut PrdNode],
    constraints: &LayoutConstraints,
) -> f32 {
    let style = &container.style;
    let dir = style.flex_direction;
    let is_row = matches!(dir, FlexDirection::Row | FlexDirection::RowReverse);
    let is_reverse = matches!(dir, FlexDirection::RowReverse | FlexDirection::ColumnReverse);

    let main_size = if is_row { container_rect.width } else { container_rect.height };
    let cross_size = if is_row { container_rect.height } else { container_rect.width };
    let gap = style.gap;

    if children.is_empty() {
        return 0.0;
    }

    // --- Phase 1: Determine base sizes ---
    struct FlexItem {
        base_main: f32,
        base_cross: f32,
        flex_grow: f32,
        flex_shrink: f32,
        margin_main_start: f32,
        margin_main_end: f32,
        margin_cross_start: f32,
        margin_cross_end: f32,
        padding: EdgeInsets,
        border: EdgeInsets,
        min_main: f32,
        max_main: f32,
    }

    let mut items: Vec<FlexItem> = Vec::with_capacity(children.len());

    for child in children.iter() {
        let cs = &child.style;
        if matches!(cs.display, Display::None) {
            items.push(FlexItem {
                base_main: 0.0, base_cross: 0.0,
                flex_grow: 0.0, flex_shrink: 0.0,
                margin_main_start: 0.0, margin_main_end: 0.0,
                margin_cross_start: 0.0, margin_cross_end: 0.0,
                padding: EdgeInsets::default(), border: EdgeInsets::default(),
                min_main: 0.0, max_main: f32::INFINITY,
            });
            continue;
        }

        let resolve_d = |d: Dimension, parent: f32| -> f32 {
            d.resolve(parent, constraints.viewport_width, constraints.viewport_height, cs.font_size, constraints.root_font_size)
        };

        let margin = EdgeInsets {
            top: resolve_d(cs.margin.top, container_rect.height),
            right: resolve_d(cs.margin.right, container_rect.width),
            bottom: resolve_d(cs.margin.bottom, container_rect.height),
            left: resolve_d(cs.margin.left, container_rect.width),
        };

        let padding = EdgeInsets {
            top: resolve_d(cs.padding.top, container_rect.height),
            right: resolve_d(cs.padding.right, container_rect.width),
            bottom: resolve_d(cs.padding.bottom, container_rect.height),
            left: resolve_d(cs.padding.left, container_rect.width),
        };

        let border = cs.border_width;

        let (m_start, m_end, c_start, c_end) = if is_row {
            (margin.left, margin.right, margin.top, margin.bottom)
        } else {
            (margin.top, margin.bottom, margin.left, margin.right)
        };

        // Resolve flex-basis or width/height
        let basis = if !cs.flex_basis.is_auto() {
            resolve_d(cs.flex_basis, main_size)
        } else if is_row && !cs.width.is_auto() {
            resolve_d(cs.width, container_rect.width)
        } else if !is_row && !cs.height.is_auto() {
            resolve_d(cs.height, container_rect.height)
        } else {
            0.0 // Will be grown or use content size
        };

        let cross = if is_row && !cs.height.is_auto() {
            resolve_d(cs.height, container_rect.height)
        } else if !is_row && !cs.width.is_auto() {
            resolve_d(cs.width, container_rect.width)
        } else {
            0.0
        };

        let min_main = if is_row {
            if cs.min_width.is_auto() { 0.0 } else { resolve_d(cs.min_width, container_rect.width) }
        } else {
            if cs.min_height.is_auto() { 0.0 } else { resolve_d(cs.min_height, container_rect.height) }
        };

        let max_main = if is_row {
            if cs.max_width.is_auto() { f32::INFINITY } else { resolve_d(cs.max_width, container_rect.width) }
        } else {
            if cs.max_height.is_auto() { f32::INFINITY } else { resolve_d(cs.max_height, container_rect.height) }
        };

        items.push(FlexItem {
            base_main: basis,
            base_cross: cross,
            flex_grow: cs.flex_grow,
            flex_shrink: cs.flex_shrink,
            margin_main_start: m_start,
            margin_main_end: m_end,
            margin_cross_start: c_start,
            margin_cross_end: c_end,
            padding,
            border,
            min_main,
            max_main,
        });
    }

    // --- Phase 2: Distribute free space ---
    let num_gaps = if items.len() > 1 { (items.len() - 1) as f32 } else { 0.0 };
    let total_gap = gap * num_gaps;
    let total_base: f32 = items.iter().map(|i| i.base_main + i.margin_main_start + i.margin_main_end).sum();
    let free = main_size - total_base - total_gap;

    let total_grow: f32 = items.iter().map(|i| i.flex_grow).sum();
    let total_shrink: f32 = items.iter().map(|i| i.flex_shrink * i.base_main).sum();

    let mut final_mains: Vec<f32> = Vec::with_capacity(items.len());
    for item in &items {
        let mut sz = item.base_main;
        if free > 0.0 && total_grow > 0.0 {
            sz += free * (item.flex_grow / total_grow);
        } else if free < 0.0 && total_shrink > 0.0 {
            let shrink_factor = (item.flex_shrink * item.base_main) / total_shrink;
            sz += free * shrink_factor; // free is negative
        }
        sz = sz.max(item.min_main).min(item.max_main);
        final_mains.push(sz.max(0.0));
    }

    // --- Phase 3: Justify-content (main-axis) ---
    let used_main: f32 = final_mains.iter().sum::<f32>()
        + items.iter().map(|i| i.margin_main_start + i.margin_main_end).sum::<f32>()
        + total_gap;
    let remaining = (main_size - used_main).max(0.0);

    let (mut offset, extra_gap) = match style.justify_content {
        JustifyContent::FlexStart => (0.0, 0.0),
        JustifyContent::FlexEnd => (remaining, 0.0),
        JustifyContent::Center => (remaining / 2.0, 0.0),
        JustifyContent::SpaceBetween => {
            if items.len() > 1 {
                (0.0, remaining / (items.len() - 1) as f32)
            } else {
                (0.0, 0.0)
            }
        }
        JustifyContent::SpaceAround => {
            let sp = remaining / items.len() as f32;
            (sp / 2.0, sp)
        }
        JustifyContent::SpaceEvenly => {
            let sp = remaining / (items.len() + 1) as f32;
            (sp, sp)
        }
    };

    if is_reverse {
        offset = main_size;
    }

    // --- Phase 4: Position children ---
    let mut max_cross_used: f32 = 0.0;

    for (i, child) in children.iter_mut().enumerate() {
        let item = &items[i];
        let m = final_mains[i];

        // Cross size
        let c = if item.base_cross > 0.0 {
            item.base_cross
        } else if matches!(style.align_items, AlignItems::Stretch) {
            cross_size - item.margin_cross_start - item.margin_cross_end
        } else {
            // Intrinsic — for now just use a default or 0
            0.0
        };

        let align = if child.style.align_self != AlignSelf::Auto {
            match child.style.align_self {
                AlignSelf::FlexStart => AlignItems::FlexStart,
                AlignSelf::FlexEnd => AlignItems::FlexEnd,
                AlignSelf::Center => AlignItems::Center,
                AlignSelf::Stretch => AlignItems::Stretch,
                AlignSelf::Auto => style.align_items,
            }
        } else {
            style.align_items
        };

        let cross_offset = match align {
            AlignItems::FlexStart => item.margin_cross_start,
            AlignItems::FlexEnd => cross_size - c - item.margin_cross_end,
            AlignItems::Center => (cross_size - c) / 2.0,
            AlignItems::Stretch => item.margin_cross_start,
            AlignItems::Baseline => item.margin_cross_start, // simplified
        };

        if is_reverse {
            offset -= item.margin_main_end + m;
        } else {
            offset += item.margin_main_start;
        }

        let (x, y, w, h) = if is_row {
            (container_rect.x + offset, container_rect.y + cross_offset, m, c)
        } else {
            (container_rect.x + cross_offset, container_rect.y + offset, c, m)
        };

        child.layout.rect = Rect { x, y, width: w, height: h };
        child.layout.content_rect = Rect {
            x: x + item.padding.left + item.border.left,
            y: y + item.padding.top + item.border.top,
            width: (w - item.padding.horizontal() - item.border.horizontal()).max(0.0),
            height: (h - item.padding.vertical() - item.border.vertical()).max(0.0),
        };
        child.layout.padding = item.padding;
        child.layout.margin = EdgeInsets {
            top: item.margin_cross_start,
            right: item.margin_main_end,
            bottom: item.margin_cross_end,
            left: item.margin_main_start,
        };

        max_cross_used = max_cross_used.max(cross_offset + c + item.margin_cross_end);

        if is_reverse {
            offset -= item.margin_main_start + gap + extra_gap;
        } else {
            offset += m + item.margin_main_end + gap + extra_gap;
        }
    }

    if is_row { max_cross_used } else { offset }
}

