//! Treemap rendering.
//!
//! Lays out a node's children with the `treemap` crate and paints them with the
//! egui `Painter`: flat category-colored cells, hard 1px borders, labels where
//! they fit, and one level of nested preview inside directories (per the
//! project's "one level of nesting" decision). Tiny sub-pixel cells are culled
//! here — the layout keeps true areas; the renderer decides what is worth
//! drawing.
//!
//! Returns the node currently under the pointer so the caller can show its path
//! and size in the status bar. No drill-down yet — that arrives in Phase 4.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, StrokeKind};
use scanner::{NodeId, NodeKind, Tree};
use treemap::{squarify, Rect};

use crate::theme::{self, Category};

const MIN_CELL: f32 = 3.0; // do not draw cells smaller than this (short side, px)
const HEADER_H: f32 = 18.0; // directory title strip
const PAD: f32 = 2.0; // gap between a directory header and its nested children
const NEST_PREVIEW: u32 = 1; // levels of nested preview to draw under each cell
const LABEL_FONT: f32 = 11.0;

/// Render the children of `root` as a treemap filling the available space.
/// Returns the node under the pointer, if any.
pub fn show(ui: &mut egui::Ui, tree: &Tree, root: NodeId) -> Option<NodeId> {
    let size = ui.available_size();
    let (area, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0, theme::BG);

    let children = sorted_children(tree, root);
    if children.is_empty() {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            "(nothing to show)",
            FontId::proportional(14.0),
            theme::TEXT_DIM,
        );
        return None;
    }

    let pointer = response.hover_pos();
    let mut hovered = None;

    let weights: Vec<u64> = children.iter().map(|&id| tree.node(id).size).collect();
    let rects = squarify(&weights, to_layout(area));
    for (&id, rect) in children.iter().zip(&rects) {
        draw_cell(&painter, tree, id, *rect, NEST_PREVIEW, pointer, &mut hovered);
    }

    hovered
}

/// Recursively draw one cell (and, for directories, a shallow preview of its
/// contents).
fn draw_cell(
    painter: &egui::Painter,
    tree: &Tree,
    id: NodeId,
    layout: Rect,
    nest: u32,
    pointer: Option<Pos2>,
    hovered: &mut Option<NodeId>,
) {
    let rect = to_screen(layout);
    if rect.width() < MIN_CELL || rect.height() < MIN_CELL {
        return;
    }

    let node = tree.node(id);
    let color = Category::of(tree, id).color();
    painter.rect_filled(rect, 0, color);
    painter.rect_stroke(rect, 0, Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);

    // Innermost cell under the pointer wins, since children paint after parents.
    if let Some(p) = pointer {
        if rect.contains(p) {
            *hovered = Some(id);
        }
    }

    let has_room_to_nest = node.kind == NodeKind::Dir
        && nest > 0
        && !node.children.is_empty()
        && rect.height() > HEADER_H + 2.0 * PAD + MIN_CELL
        && rect.width() > 3.0 * MIN_CELL;

    if has_room_to_nest {
        // Title strip, then a recessed area for the nested preview.
        let header = ERect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x, rect.min.y + HEADER_H),
        );
        draw_label(painter, header, tree.name(id), node.size, theme::TEXT);

        let inner = ERect::from_min_max(
            Pos2::new(rect.min.x + PAD, rect.min.y + HEADER_H),
            Pos2::new(rect.max.x - PAD, rect.max.y - PAD),
        );
        let kids = sorted_children(tree, id);
        let weights: Vec<u64> = kids.iter().map(|&c| tree.node(c).size).collect();
        for (&cid, crect) in kids.iter().zip(&squarify(&weights, to_layout(inner))) {
            draw_cell(painter, tree, cid, *crect, nest - 1, pointer, hovered);
        }
    } else {
        // Leaf, or no room to nest: a single label if it fits.
        draw_label(painter, rect, tree.name(id), node.size, theme::contrast_text(color));
    }
}

/// Draw `name + size` clipped to `area`, but only if it fits.
fn draw_label(painter: &egui::Painter, area: ERect, name: &str, size: u64, color: Color32) {
    let inner = area.shrink(4.0);
    if inner.width() < 12.0 || inner.height() < 8.0 {
        return;
    }
    let text = format!("{}   {}", name, theme::format_size(size));
    let galley = painter.layout_no_wrap(text, FontId::monospace(LABEL_FONT), color);
    if galley.size().x <= inner.width() && galley.size().y <= inner.height() {
        painter.galley(inner.min, galley, color);
    }
}

fn sorted_children(tree: &Tree, id: NodeId) -> Vec<NodeId> {
    let mut children = tree.node(id).children.clone();
    children.sort_unstable_by_key(|&c| std::cmp::Reverse(tree.node(c).size));
    children
}

fn to_layout(r: ERect) -> Rect {
    Rect::new(r.min.x as f64, r.min.y as f64, r.width() as f64, r.height() as f64)
}

fn to_screen(r: Rect) -> ERect {
    ERect::from_min_size(
        Pos2::new(r.x as f32, r.y as f32),
        egui::Vec2::new(r.w as f32, r.h as f32),
    )
}
