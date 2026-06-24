//! The application: navigation state, the per-frame UI, and the three-panel
//! shell (toolbar / treemap / status bar).
//!
//! Navigation is just a single `current` node id. The breadcrumb and "go up"
//! are derived from the scanner's parent pointers, and every change of `current`
//! kicks off a short eased zoom that grows the new view out of a focal cell.

use std::path::PathBuf;

use eframe::egui::{self, Pos2, Rect as ERect};
use scanner::{NodeId, NodeKind, Tree};

use crate::scan::Scan;
use crate::theme::{self, format_size};
use crate::treemap_view;

/// Zoom duration in seconds.
const ANIM_SECS: f64 = 0.22;

pub struct StorageSifterApp {
    /// The directory passed on the command line (the scan root).
    path: PathBuf,
    /// Background scan state.
    scan: Scan,
    /// The node currently drilled into (its children fill the view). Always a
    /// valid id because the root is 0 and we reset to 0 on every (re)scan.
    current: NodeId,
    /// In-progress zoom, if any.
    anim: Option<AnimState>,
    /// The treemap rect from last frame, used to locate cells for "go up" zooms.
    last_area: ERect,
    /// Node under the pointer, refreshed each frame by the treemap view.
    hovered: Option<NodeId>,
}

#[derive(Clone, Copy)]
struct AnimState {
    /// Camera `source` rect at the start of the zoom.
    from: ERect,
    /// Camera `source` rect at the end of the zoom.
    to: ERect,
    /// `egui` time when the first frame rendered; stamped lazily so the zoom
    /// always begins at t = 0 no matter when it was queued.
    start: Option<f64>,
    /// Node to switch to when the zoom finishes (drill-in). `None` for zoom-out,
    /// where `current` is already the destination.
    commit: Option<NodeId>,
}

impl StorageSifterApp {
    pub fn new(path: PathBuf) -> Self {
        let scan = Scan::start(&path);
        Self {
            path,
            scan,
            current: 0,
            anim: None,
            last_area: ERect::ZERO,
            hovered: None,
        }
    }

    fn rescan(&mut self) {
        self.scan = Scan::start(&self.path);
        self.current = 0;
        self.anim = None;
        self.hovered = None;
    }
}

impl eframe::App for StorageSifterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.scan.poll();
        if self.scan.is_running() {
            ui.ctx().request_repaint();
        }

        self.toolbar(ui);
        self.status_bar(ui);
        self.treemap(ui);
    }
}

impl StorageSifterApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("⟳ Rescan").clicked() {
                    self.rescan();
                }
                ui.separator();
                self.breadcrumb(ui);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &self.scan {
                        Scan::Done { tree, elapsed } => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}  ·  {} items  ·  {:.2}s",
                                    format_size(tree.node(tree.root).size),
                                    tree.len(),
                                    elapsed.as_secs_f64()
                                ))
                                .color(theme::TEXT_DIM),
                            );
                        }
                        Scan::Running { .. } => {
                            ui.label(egui::RichText::new("scanning…").color(theme::ACCENT));
                        }
                        Scan::Error(_) => {
                            ui.label(egui::RichText::new("error").color(theme::ACCENT));
                        }
                    }
                });
            });
            ui.add_space(2.0);
        });
    }

    /// Clickable breadcrumb from the scan root down to `current`, built by
    /// walking parent pointers.
    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let Scan::Done { tree, .. } = &self.scan else {
            ui.monospace(self.path.display().to_string());
            return;
        };

        let mut ids = Vec::new();
        let mut node = Some(self.current);
        while let Some(id) = node {
            ids.push(id);
            node = tree.node(id).parent;
        }
        ids.reverse();

        let mut jump = None;
        for (i, &id) in ids.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("›").color(theme::TEXT_DIM));
            }
            let label = segment_label(tree, id);
            if i == ids.len() - 1 {
                ui.label(egui::RichText::new(label).color(theme::TEXT).strong());
            } else if ui.link(label).clicked() {
                jump = Some(id);
            }
        }

        if let Some(target) = jump {
            // Zoom out of the cell we're leaving (inlined rather than a &mut self
            // method because `tree` borrows `self.scan`).
            let from = focal_up(tree, self.current, target, self.last_area)
                .unwrap_or_else(|| fallback_focal(self.last_area));
            self.current = target;
            self.anim = Some(AnimState {
                from,
                to: self.last_area,
                start: None,
                commit: None,
            });
            self.hovered = None;
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| match (&self.scan, self.hovered) {
                (Scan::Done { tree, .. }, Some(id)) => {
                    let node = tree.node(id);
                    ui.monospace(tree.path(id).display().to_string());
                    ui.label(
                        egui::RichText::new(format!("·  {}", format_size(node.size)))
                            .color(theme::TEXT_DIM),
                    );
                    let cat = theme::Category::of(tree, id);
                    ui.label(egui::RichText::new(format!("·  {}", cat.label())).color(cat.color()));
                    if node.is_hardlinked() {
                        ui.label(egui::RichText::new("·  hardlinked").color(theme::ACCENT));
                    }
                }
                (Scan::Done { tree, .. }, None) => {
                    let n = tree.unreadable.len();
                    let hint = if n == 0 {
                        "click a folder to drill in · Backspace/Esc to go up".to_owned()
                    } else {
                        format!("click to drill · Backspace to go up · {n} unreadable path(s)")
                    };
                    ui.label(egui::RichText::new(hint).color(theme::TEXT_DIM));
                }
                _ => {
                    ui.label("");
                }
            });
            ui.add_space(2.0);
        });
    }

    fn treemap(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let Scan::Done { tree, .. } = &self.scan else {
                let msg = match &self.scan {
                    Scan::Running { started, .. } => {
                        format!("Scanning…  {:.1}s", started.elapsed().as_secs_f64())
                    }
                    Scan::Error(e) => format!("Scan failed:\n{e}"),
                    Scan::Done { .. } => String::new(),
                };
                center_message(ui, &msg);
                return;
            };

            let now = ui.ctx().input(|i| i.time);

            // Keyboard: go up a level (zoom out of the child we leave).
            let go_up = ui
                .ctx()
                .input(|i| i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Escape));
            if go_up && self.anim.is_none() {
                if let Some(parent) = tree.node(self.current).parent {
                    let from = focal_up(tree, self.current, parent, self.last_area)
                        .unwrap_or_else(|| fallback_focal(self.last_area));
                    self.current = parent;
                    self.anim = Some(AnimState {
                        from,
                        to: self.last_area,
                        start: None,
                        commit: None,
                    });
                }
            }

            // Advance the camera; commit + clear when the zoom finishes.
            let anim = if let Some(a) = &mut self.anim {
                let start = *a.start.get_or_insert(now);
                let t = (now - start) / ANIM_SECS;
                if t >= 1.0 {
                    None
                } else {
                    let source = lerp_rect(a.from, a.to, ease_out_cubic(t as f32));
                    Some(treemap_view::Anim { source })
                }
            } else {
                None
            };
            if self.anim.is_some() && anim.is_none() {
                if let Some(commit) = self.anim.and_then(|a| a.commit) {
                    self.current = commit;
                }
                self.anim = None;
            }

            let it = treemap_view::show(ui, tree, self.current, anim);
            self.last_area = it.area;
            self.hovered = it.hovered.map(|h| h.id);

            // Drill into the clicked folder: zoom into it, then commit at the end.
            if let Some(hit) = it.clicked {
                let node = tree.node(hit.id);
                if node.kind == NodeKind::Dir && !node.children.is_empty() {
                    self.anim = Some(AnimState {
                        from: it.area,
                        to: hit.rect,
                        start: None,
                        commit: Some(hit.id),
                    });
                }
            }

            if self.anim.is_some() {
                ui.ctx().request_repaint();
            }
        });
    }
}

/// The screen rect of the child of `target` that lies on the path up from
/// `from` — i.e. where we should grow the parent view out of when zooming out.
fn focal_up(tree: &Tree, from: NodeId, target: NodeId, area: ERect) -> Option<ERect> {
    let mut node = from;
    while let Some(parent) = tree.node(node).parent {
        if parent == target {
            return treemap_view::child_rect(tree, target, node, area);
        }
        node = parent;
    }
    None
}

fn fallback_focal(area: ERect) -> ERect {
    ERect::from_center_size(area.center(), area.size() * 0.25)
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

fn segment_label(tree: &Tree, id: NodeId) -> String {
    if id == tree.root {
        let path = tree.path(id);
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    } else {
        tree.name(id).to_string()
    }
}

fn center_message(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(theme::TEXT_DIM).size(16.0));
    });
}
