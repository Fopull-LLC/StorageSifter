//! Treemap rendering and interaction.
//!
//! Lays out a node's children with the `treemap` crate and paints them with the
//! egui `Painter`: flat category-colored cells, hard 1px borders, ellipsized
//! labels, and one level of nested preview inside directories.
//!
//! The zoom animation is a *camera*: during a transition the whole current view
//! is rendered through an affine [`Xform`] that maps a shrinking/growing
//! `source` rect onto the full area, so drilling in zooms *into* a cell (its
//! neighbours slide off the edges) rather than blanking the screen. Interaction
//! is suspended until the camera settles.
//!
//! Two interaction results are reported: the *innermost* cell under the pointer
//! (for the status bar) and, on a click, the *top-level* child clicked (the
//! drill-down target — one level per click).

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

/// One frame of an in-progress zoom: the (already-eased) `source` rect that the
/// camera maps onto the full area.
#[derive(Clone, Copy)]
pub struct Anim {
    pub source: ERect,
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
        Some(a) => Xform { source: a.source, area },
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

    // Monospace metrics, measured once and reused for every label.
    let probe = painter.layout_no_wrap("0".to_owned(), FontId::monospace(LABEL_FONT), theme::TEXT);
    let ctx = Paint {
        painter: &painter,
        tree,
        xform,
        hover_pos: if interactive { response.hover_pos() } else { None },
        char_w: probe.size().x,
        line_h: probe.size().y,
    };

    let weights: Vec<u64> = children.iter().map(|&id| tree.node(id).size).collect();
    let rects = squarify(&weights, to_layout(area));

    let mut hovered = None;
    for (&id, rect) in children.iter().zip(&rects) {
        draw_cell(&ctx, id, *rect, NEST_PREVIEW, &mut hovered);
    }

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
    char_w: f32,
    line_h: f32,
}

fn draw_cell(ctx: &Paint, id: NodeId, layout: Rect, nest: u32, hovered: &mut Option<Hit>) {
    let rect = to_screen(layout); // untransformed (full-layout) coords
    if rect.width() < MIN_CELL || rect.height() < MIN_CELL {
        return;
    }
    let drawn = ctx.xform.apply(rect); // camera-transformed for the zoom
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

    let label_color = theme::contrast_text(color);
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

/// Draw `name` (plus size if there is room), ellipsized to fit `area`. Always
/// shows *something* if even two characters fit — never a blank box.
fn draw_label(ctx: &Paint, area: ERect, name: &str, size: u64, color: Color32) {
    let inner = area.shrink(4.0);
    if ctx.char_w <= 0.0 || inner.width() < ctx.char_w * 2.0 || inner.height() < ctx.line_h {
        return;
    }
    let max_chars = (inner.width() / ctx.char_w).floor() as usize;
    let text = fit_text(name, &theme::format_size(size), max_chars);
    let galley = ctx
        .painter
        .layout_no_wrap(text, FontId::monospace(LABEL_FONT), color);
    ctx.painter.galley(inner.min, galley, color);
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

/// Camera transform: maps the `source` rect onto the full `area`, so a shrinking
/// `source` zooms the view in. Identity when `source == area`.
#[derive(Clone, Copy)]
struct Xform {
    source: ERect,
    area: ERect,
}

impl Xform {
    fn identity(area: ERect) -> Self {
        Xform { source: area, area }
    }

    fn apply(&self, r: ERect) -> ERect {
        if self.source.width() <= 0.0 || self.source.height() <= 0.0 {
            return r;
        }
        let sx = self.area.width() / self.source.width();
        let sy = self.area.height() / self.source.height();
        ERect::from_min_size(
            Pos2::new(
                self.area.min.x + (r.min.x - self.source.min.x) * sx,
                self.area.min.y + (r.min.y - self.source.min.y) * sy,
            ),
            Vec2::new(r.width() * sx, r.height() * sy),
        )
    }
}
