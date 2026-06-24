//! Treemap rendering and interaction.
//!
//! Lays out a node's children with the `treemap` crate and paints them with the
//! egui `Painter`: flat category-colored cells, hard 1px borders, labels where
//! they fit, and one level of nested preview inside directories.
//!
//! Two interaction results are reported each frame: the *innermost* cell under
//! the pointer (for the status bar) and, on a click, the *top-level* child that
//! was clicked (the drill-down target — one level per click). During a zoom
//! animation the cells are drawn through an affine [`Xform`] so the new view
//! grows out of the cell that was clicked; interaction is suspended until the
//! animation settles.

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

/// One frame of an in-progress zoom: the cell the new view grows out of, and
/// the eased progress in `0.0..=1.0`.
#[derive(Clone, Copy)]
pub struct Anim {
    pub focal: ERect,
    pub t: f32,
}

/// What the treemap reported this frame.
pub struct Interaction {
    /// Innermost cell under the pointer — for inspection in the status bar.
    pub hovered: Option<Hit>,
    /// Top-level child clicked — the drill-down target (one level per click).
    pub clicked: Option<Hit>,
    /// The rect the treemap filled, so the caller can locate cells next frame.
    pub area: ERect,
}

/// Render the children of `current` as a treemap filling the available space.
pub fn show(ui: &mut egui::Ui, tree: &Tree, current: NodeId, anim: Option<Anim>) -> Interaction {
    let size = ui.available_size();
    let (area, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0, theme::BG);

    let interactive = anim.is_none();
    let xform = match anim {
        Some(a) => Xform {
            full: area,
            target: lerp_rect(a.focal, area, ease_out_cubic(a.t)),
        },
        None => Xform::identity(area),
    };

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
            area,
        };
    }

    let weights: Vec<u64> = children.iter().map(|&id| tree.node(id).size).collect();
    let rects = squarify(&weights, to_layout(area));

    let hover_pos = if interactive { response.hover_pos() } else { None };
    let mut hovered = None;
    let ctx = Paint {
        painter: &painter,
        tree,
        xform,
        hover_pos,
    };
    for (&id, rect) in children.iter().zip(&rects) {
        draw_cell(&ctx, id, *rect, NEST_PREVIEW, &mut hovered);
    }

    // A click targets the top-level child it landed in: one level per click,
    // regardless of how deep the nested preview goes.
    let clicked = if interactive && response.clicked() {
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

    Interaction {
        hovered,
        clicked,
        area,
    }
}

/// The screen rect a given `child` of `parent` would occupy in `area`. Used to
/// animate "zoom out" by growing the parent view out of the child we left.
pub fn child_rect(tree: &Tree, parent: NodeId, child: NodeId, area: ERect) -> Option<ERect> {
    let kids = sorted_children(tree, parent);
    let idx = kids.iter().position(|&k| k == child)?;
    let weights: Vec<u64> = kids.iter().map(|&k| tree.node(k).size).collect();
    let rects = squarify(&weights, to_layout(area));
    Some(to_screen(rects[idx]))
}

/// The unchanging context threaded through the recursive cell drawing.
struct Paint<'a> {
    painter: &'a egui::Painter,
    tree: &'a Tree,
    xform: Xform,
    hover_pos: Option<Pos2>,
}

fn draw_cell(ctx: &Paint, id: NodeId, layout: Rect, nest: u32, hovered: &mut Option<Hit>) {
    let rect = to_screen(layout); // untransformed (full-layout) coords
    if rect.width() < MIN_CELL || rect.height() < MIN_CELL {
        return;
    }
    let drawn = ctx.xform.apply(rect); // transformed for the zoom animation
    if drawn.width() < 0.5 || drawn.height() < 0.5 {
        return;
    }

    let node = ctx.tree.node(id);
    let color = Category::of(ctx.tree, id).color();
    ctx.painter.rect_filled(drawn, 0, color);
    ctx.painter
        .rect_stroke(drawn, 0, Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);

    // Hit-test against the untransformed rect; the caller only consults this
    // when no animation is running, so the two coincide then.
    if let Some(p) = ctx.hover_pos {
        if rect.contains(p) {
            *hovered = Some(Hit { id, rect });
        }
    }

    let can_nest = node.kind == NodeKind::Dir
        && nest > 0
        && !node.children.is_empty()
        && rect.height() > HEADER_H + 2.0 * PAD + MIN_CELL
        && rect.width() > 3.0 * MIN_CELL;

    if can_nest {
        let header = ERect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + HEADER_H));
        draw_label(ctx.painter, ctx.xform.apply(header), ctx.tree.name(id), node.size, theme::TEXT);

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
        draw_label(
            ctx.painter,
            ctx.xform.apply(rect),
            ctx.tree.name(id),
            node.size,
            theme::contrast_text(color),
        );
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
        Vec2::new(r.w as f32, r.h as f32),
    )
}

/// Affine map from the full treemap rect onto a (possibly shrunken) target rect.
/// Identity when `target == full`. Used to grow the new view out of a focal cell.
#[derive(Clone, Copy)]
struct Xform {
    full: ERect,
    target: ERect,
}

impl Xform {
    fn identity(full: ERect) -> Self {
        Xform { full, target: full }
    }

    fn apply(&self, r: ERect) -> ERect {
        if self.full.width() <= 0.0 || self.full.height() <= 0.0 {
            return r;
        }
        let sx = self.target.width() / self.full.width();
        let sy = self.target.height() / self.full.height();
        ERect::from_min_size(
            Pos2::new(
                self.target.min.x + (r.min.x - self.full.min.x) * sx,
                self.target.min.y + (r.min.y - self.full.min.y) * sy,
            ),
            Vec2::new(r.width() * sx, r.height() * sy),
        )
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

fn lerp_rect(a: ERect, b: ERect, t: f32) -> ERect {
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    ERect::from_min_max(
        Pos2::new(lerp(a.min.x, b.min.x), lerp(a.min.y, b.min.y)),
        Pos2::new(lerp(a.max.x, b.max.x), lerp(a.max.y, b.max.y)),
    )
}
