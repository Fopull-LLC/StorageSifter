//! Treemap rendering and interaction.
//!
//! Lays out a node's children with the `treemap` crate and paints them with the
//! egui `Painter`: flat category-colored cells, hard 1px borders, ellipsized
//! labels, one level of nested preview, selection highlights, and a danger
//! outline on flagged (system) top-level cells.
//!
//! The zoom is a cross-fade between two views (parent zooming in, child growing
//! out), so it morphs continuously with no snap at the end.
//!
//! Interaction (when idle) is reported as: the innermost cell hovered (status
//! bar), the top-level child left-clicked (drill, one level per click), whether
//! a modifier was held (select instead of drill), and the innermost cell
//! right-clicked (context menu). The treemap's `Response` is returned so the
//! caller can attach a context menu to it.

use std::collections::HashSet;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, StrokeKind, Vec2};
use scanner::{NodeId, NodeKind, Tree};
use treemap::{squarify, Rect};

use crate::theme::{self, Category};

const MIN_CELL: f32 = 3.0; // don't draw cells smaller than this (short side, px)
const HEADER_H: f32 = 18.0; // directory title strip
const PAD: f32 = 2.0; // gap between a directory header and its nested children
const NEST_PREVIEW: u32 = 1; // levels of nested preview drawn under each cell
const LABEL_FONT: f32 = 11.0;

/// A cell hit by the pointer: the node and the (untransformed) screen rect it
/// occupies.
#[derive(Clone, Copy)]
pub struct Hit {
    pub id: NodeId,
    pub rect: ERect,
}

/// One frame of an in-progress zoom between `parent` and `child`, connected at
/// the `pivot` cell.
#[derive(Clone, Copy)]
pub struct Anim {
    pub parent: NodeId,
    pub child: NodeId,
    pub pivot: ERect,
    pub e: f32,
    pub drilling_in: bool,
}

/// What the treemap reported this frame.
pub struct Interaction {
    /// Innermost cell under the pointer — for inspection in the status bar.
    pub hovered: Option<Hit>,
    /// Top-level child left-clicked — the drill-down target.
    pub clicked: Option<Hit>,
    /// Whether Ctrl/Shift was held during the click (→ select instead of drill).
    pub modified: bool,
    /// Innermost cell right-clicked — the context-menu target.
    pub secondary: Option<Hit>,
    /// The treemap's response, for attaching a context menu.
    pub response: egui::Response,
    /// The rect the treemap filled.
    pub area: ERect,
}

/// Render the children of `current` (or, during a zoom, the cross-faded parent
/// and child views). `selection` cells are highlighted; `warn` cells get a
/// danger outline.
pub fn show(
    ui: &mut egui::Ui,
    tree: &Tree,
    current: NodeId,
    anim: Option<Anim>,
    selection: &HashSet<NodeId>,
    warn: &HashSet<NodeId>,
) -> Interaction {
    let size = ui.available_size();
    let (area, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0, theme::BG);

    let probe = painter.layout_no_wrap("0".to_owned(), FontId::monospace(LABEL_FONT), theme::TEXT);
    let (char_w, line_h) = (probe.size().x, probe.size().y);
    let empty: HashSet<NodeId> = HashSet::new();

    // --- Animating: cross-fade the parent (zooming) and child (growing) views.
    if let Some(a) = anim {
        let (p_src, p_alpha, c_dst, c_alpha) = if a.drilling_in {
            (lerp_rect(area, a.pivot, a.e), 1.0 - a.e, lerp_rect(a.pivot, area, a.e), a.e)
        } else {
            (lerp_rect(a.pivot, area, a.e), a.e, lerp_rect(area, a.pivot, a.e), 1.0 - a.e)
        };
        let mut sink = None;
        let parent = Paint {
            painter: &painter,
            tree,
            area,
            xform: Xform { src: p_src, dst: area },
            alpha: p_alpha,
            hover_pos: None,
            char_w,
            line_h,
            selection,
            warn: &empty,
        };
        draw_node_children(&parent, a.parent, &mut sink);
        let child = Paint {
            xform: Xform { src: area, dst: c_dst },
            alpha: c_alpha,
            ..parent
        };
        draw_node_children(&child, a.child, &mut sink);
        return Interaction {
            hovered: None,
            clicked: None,
            modified: false,
            secondary: None,
            response,
            area,
        };
    }

    // --- Idle: a single static view that responds to the pointer.
    let children = sorted_children(tree, current);
    if children.is_empty() {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            "(empty folder)",
            FontId::proportional(14.0),
            theme::TEXT_DIM,
        );
        return Interaction {
            hovered: None,
            clicked: None,
            modified: false,
            secondary: None,
            response,
            area,
        };
    }

    let ctx = Paint {
        painter: &painter,
        tree,
        area,
        xform: Xform::identity(area),
        alpha: 1.0,
        hover_pos: response.hover_pos(),
        char_w,
        line_h,
        selection,
        warn,
    };
    let weights: Vec<u64> = children.iter().map(|&id| tree.node(id).size).collect();
    let rects = squarify(&weights, to_layout(area));
    let mut hovered = None;
    for (&id, rect) in children.iter().zip(&rects) {
        draw_cell(&ctx, id, *rect, NEST_PREVIEW, &mut hovered);
    }

    let modifiers = ui.input(|i| i.modifiers);
    let modified = modifiers.ctrl || modifiers.shift || modifiers.mac_cmd;

    // Left click targets the top-level child it landed in (one level per click).
    let clicked = if response.clicked() {
        response.interact_pointer_pos().and_then(|p| {
            children
                .iter()
                .zip(&rects)
                .map(|(&id, r)| (id, to_screen(*r)))
                .find(|(_, r)| r.contains(p))
                .map(|(id, rect)| Hit { id, rect })
        })
    } else {
        None
    };
    let secondary = if response.secondary_clicked() { hovered } else { None };

    Interaction {
        hovered,
        clicked,
        modified,
        secondary,
        response,
        area,
    }
}

/// The screen rect a given `child` of `parent` would occupy in `area`.
pub fn child_rect(tree: &Tree, parent: NodeId, child: NodeId, area: ERect) -> Option<ERect> {
    let kids = sorted_children(tree, parent);
    let idx = kids.iter().position(|&k| k == child)?;
    let weights: Vec<u64> = kids.iter().map(|&k| tree.node(k).size).collect();
    let rects = squarify(&weights, to_layout(area));
    Some(to_screen(rects[idx]))
}

/// The unchanging context threaded through the recursive cell drawing.
#[derive(Clone, Copy)]
struct Paint<'a> {
    painter: &'a egui::Painter,
    tree: &'a Tree,
    area: ERect,
    xform: Xform,
    alpha: f32,
    hover_pos: Option<Pos2>,
    char_w: f32,
    line_h: f32,
    selection: &'a HashSet<NodeId>,
    warn: &'a HashSet<NodeId>,
}

fn draw_node_children(ctx: &Paint, node: NodeId, hovered: &mut Option<Hit>) {
    let children = sorted_children(ctx.tree, node);
    if children.is_empty() {
        return;
    }
    let weights: Vec<u64> = children.iter().map(|&id| ctx.tree.node(id).size).collect();
    let rects = squarify(&weights, to_layout(ctx.area));
    for (&id, rect) in children.iter().zip(&rects) {
        draw_cell(ctx, id, *rect, NEST_PREVIEW, hovered);
    }
}

fn draw_cell(ctx: &Paint, id: NodeId, layout: Rect, nest: u32, hovered: &mut Option<Hit>) {
    let rect = to_screen(layout);
    if rect.width() < MIN_CELL || rect.height() < MIN_CELL {
        return;
    }
    let drawn = ctx.xform.apply(rect);
    if drawn.width() < 0.5 || drawn.height() < 0.5 {
        return;
    }

    let node = ctx.tree.node(id);
    let color = Category::of(ctx.tree, id).color();
    ctx.painter.rect_filled(drawn, 0, color.gamma_multiply(ctx.alpha));
    ctx.painter.rect_stroke(
        drawn,
        0,
        Stroke::new(1.0, theme::BORDER.gamma_multiply(ctx.alpha)),
        StrokeKind::Inside,
    );
    if node.is_mountpoint() {
        ctx.painter.rect_stroke(
            drawn,
            0,
            Stroke::new(2.5, theme::MOUNT.gamma_multiply(ctx.alpha)),
            StrokeKind::Inside,
        );
    }
    if ctx.selection.contains(&id) {
        ctx.painter
            .rect_filled(drawn, 0, theme::ACCENT.gamma_multiply(0.30 * ctx.alpha));
        ctx.painter.rect_stroke(
            drawn,
            0,
            Stroke::new(2.0, theme::ACCENT.gamma_multiply(ctx.alpha)),
            StrokeKind::Inside,
        );
    } else if ctx.warn.contains(&id) {
        ctx.painter.rect_stroke(
            drawn,
            0,
            Stroke::new(2.0, theme::DANGER.gamma_multiply(ctx.alpha)),
            StrokeKind::Inside,
        );
    }

    if let Some(p) = ctx.hover_pos {
        if rect.contains(p) {
            *hovered = Some(Hit { id, rect });
        }
    }

    let label_color = if node.is_mountpoint() {
        theme::MOUNT
    } else {
        theme::contrast_text(color)
    };
    let can_nest = node.kind == NodeKind::Dir
        && nest > 0
        && !node.children.is_empty()
        && rect.height() > HEADER_H + 2.0 * PAD + MIN_CELL
        && rect.width() > 3.0 * MIN_CELL;

    if can_nest {
        let header = ERect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + HEADER_H));
        draw_label(ctx, ctx.xform.apply(header), ctx.tree.name(id), node.size, label_color);

        let inner = ERect::from_min_max(
            Pos2::new(rect.min.x + PAD, rect.min.y + HEADER_H),
            Pos2::new(rect.max.x - PAD, rect.max.y - PAD),
        );
        let kids = sorted_children(ctx.tree, id);
        let weights: Vec<u64> = kids.iter().map(|&c| ctx.tree.node(c).size).collect();
        for (&cid, crect) in kids.iter().zip(&squarify(&weights, to_layout(inner))) {
            draw_cell(ctx, cid, *crect, nest - 1, hovered);
        }
    } else {
        draw_label(ctx, drawn, ctx.tree.name(id), node.size, label_color);
    }
}

/// Draw `name` (plus size if there is room), ellipsized to fit `area`.
fn draw_label(ctx: &Paint, area: ERect, name: &str, size: u64, color: Color32) {
    let inner = area.shrink(4.0);
    if ctx.char_w <= 0.0 || inner.width() < ctx.char_w * 2.0 || inner.height() < ctx.line_h {
        return;
    }
    let max_chars = (inner.width() / ctx.char_w).floor() as usize;
    let text = fit_text(name, &theme::format_size(size), max_chars);
    let galley = ctx.painter.layout_no_wrap(
        text,
        FontId::monospace(LABEL_FONT),
        color.gamma_multiply(ctx.alpha),
    );
    ctx.painter
        .galley(inner.min, galley, color.gamma_multiply(ctx.alpha));
}

/// Fit `name` (and, space permitting, its size) into `max_chars`, ellipsizing
/// the name with `…` when it must be cut.
fn fit_text(name: &str, size_str: &str, max_chars: usize) -> String {
    let name_chars = name.chars().count();
    if name_chars + 3 + size_str.chars().count() <= max_chars {
        format!("{name}   {size_str}")
    } else if name_chars <= max_chars {
        name.to_owned()
    } else {
        let take = max_chars.saturating_sub(1).max(1);
        let truncated: String = name.chars().take(take).collect();
        format!("{truncated}…")
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
        Vec2::new(r.w as f32, r.h as f32),
    )
}

/// Affine map of one rectangle (`src`) onto another (`dst`). Identity when
/// `src == dst`.
#[derive(Clone, Copy)]
struct Xform {
    src: ERect,
    dst: ERect,
}

impl Xform {
    fn identity(area: ERect) -> Self {
        Xform { src: area, dst: area }
    }

    fn apply(&self, r: ERect) -> ERect {
        if self.src.width() <= 0.0 || self.src.height() <= 0.0 {
            return r;
        }
        let sx = self.dst.width() / self.src.width();
        let sy = self.dst.height() / self.src.height();
        ERect::from_min_size(
            Pos2::new(
                self.dst.min.x + (r.min.x - self.src.min.x) * sx,
                self.dst.min.y + (r.min.y - self.src.min.y) * sy,
            ),
            Vec2::new(r.width() * sx, r.height() * sy),
        )
    }
}

fn lerp_rect(a: ERect, b: ERect, t: f32) -> ERect {
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    ERect::from_min_max(
        Pos2::new(lerp(a.min.x, b.min.x), lerp(a.min.y, b.min.y)),
        Pos2::new(lerp(a.max.x, b.max.x), lerp(a.max.y, b.max.y)),
    )
}
