//! The application: a device picker, navigation, multi-select, file operations,
//! and the panel shell (toolbar / selection bar / treemap / status bar).

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Rect as ERect};
use scanner::safety::{classify, Class};
use scanner::{NodeId, NodeKind, Tree};

use crate::ops;
use crate::scan::Scan;
use crate::theme::{self, format_size};
use crate::treemap_view;

/// Zoom duration in seconds.
const ANIM_SECS: f64 = 0.22;

pub struct StorageSifterApp {
    disks: Vec<DiskInfo>,
    scan: Option<Scan>,
    path: PathBuf,
    current: NodeId,
    anim: Option<AnimState>,
    last_area: ERect,
    hovered: Option<NodeId>,
    /// Multi-selected nodes (Ctrl/Shift-click).
    selection: HashSet<NodeId>,
    /// Node a context menu is open for.
    menu: Option<NodeId>,
    /// The active modal dialog, if any.
    dialog: Dialog,
    /// The user's home directory, for safety classification.
    home: PathBuf,
    /// Last operation result, shown in the status bar.
    status: Option<String>,
}

struct DiskInfo {
    name: String,
    mount: PathBuf,
    fs: String,
    total: u64,
    available: u64,
}

#[derive(Clone, Copy)]
struct AnimState {
    parent: NodeId,
    child: NodeId,
    pivot: ERect,
    drilling_in: bool,
    start: Option<f64>,
}

enum Dialog {
    None,
    Properties(NodeId),
    Confirm { ids: Vec<NodeId>, permanent: bool },
    Shortcuts,
}

enum MenuAction {
    Properties(NodeId),
    Reveal(NodeId),
    Trash(Vec<NodeId>),
    Delete(Vec<NodeId>),
}

#[derive(Clone, Copy)]
enum SelAction {
    Clear,
    Trash,
    Delete,
}

/// Result of showing the confirm dialog this frame.
enum Confirm {
    Open,
    Cancel,
    Go,
}

impl StorageSifterApp {
    pub fn new(path: Option<PathBuf>) -> Self {
        let (scan, path) = match path {
            Some(p) => (Some(Scan::start(&p)), p),
            None => (None, PathBuf::new()),
        };
        Self {
            disks: list_disks(),
            scan,
            path,
            current: 0,
            anim: None,
            last_area: ERect::ZERO,
            hovered: None,
            selection: HashSet::new(),
            menu: None,
            dialog: Dialog::None,
            home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default(),
            status: None,
        }
    }

    fn open(&mut self, path: PathBuf) {
        if let Some(old) = &self.scan {
            old.cancel(); // don't leave a superseded scan churning in the background
        }
        self.scan = Some(Scan::start(&path));
        self.path = path;
        self.current = 0;
        self.anim = None;
        self.hovered = None;
        self.selection.clear();
        self.dialog = Dialog::None;
        self.status = None;
    }
}

impl eframe::App for StorageSifterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.scan.is_none() {
            self.device_picker(ui);
            return;
        }

        if let Some(scan) = &mut self.scan {
            scan.poll();
            if scan.is_running() {
                ui.ctx().request_repaint();
            }
        }

        // F5 / Ctrl+R rescans the current path.
        if matches!(self.scan, Some(Scan::Done { .. }))
            && ui.ctx().input(|i| {
                i.key_pressed(egui::Key::F5) || (i.modifiers.ctrl && i.key_pressed(egui::Key::R))
            })
        {
            self.open(self.path.clone());
        }

        self.toolbar(ui);
        self.selection_bar(ui);
        self.status_bar(ui);
        self.treemap(ui);
        self.dialogs(ui.ctx());
    }
}

impl StorageSifterApp {
    fn device_picker(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let mut refresh = false;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.heading(egui::RichText::new("StorageSifter").color(theme::TEXT));
                ui.label(egui::RichText::new("Choose a filesystem to scan").color(theme::TEXT_DIM));
                ui.add_space(8.0);
                if ui.button("⟳  Refresh").clicked() {
                    refresh = true;
                }
                ui.add_space(12.0);
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for disk in &self.disks {
                        if disk_row(ui, disk).clicked() {
                            chosen = Some(disk.mount.clone());
                        }
                        ui.add_space(6.0);
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("A Fopull LLC project  ·").color(theme::TEXT_DIM));
                    ui.hyperlink_to("fopull.com", "https://fopull.com");
                });
            });
        });

        if refresh {
            self.disks = list_disks();
        }
        if let Some(mount) = chosen {
            self.open(mount);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("≡  Devices").clicked() {
                    if let Some(scan) = &self.scan {
                        scan.cancel();
                    }
                    self.scan = None;
                    self.disks = list_disks();
                }
                if ui.button("⟳  Rescan").clicked() {
                    self.open(self.path.clone());
                }
                if ui.button("?  Keys").clicked() {
                    self.dialog = Dialog::Shortcuts;
                }
                ui.separator();
                self.breadcrumb(ui);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match self.scan.as_ref() {
                        Some(Scan::Done { tree, elapsed }) => {
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
                        Some(Scan::Running { .. }) => {
                            ui.label(egui::RichText::new("scanning…").color(theme::ACCENT));
                        }
                        _ => {}
                    }
                });
            });
            ui.add_space(2.0);
        });
    }

    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
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
            self.anim = zoom_out_pivot(tree, self.current, target, self.last_area).map(
                |(child, pivot)| AnimState {
                    parent: target,
                    child,
                    pivot,
                    drilling_in: false,
                    start: None,
                },
            );
            self.current = target;
            self.hovered = None;
        }
    }

    fn selection_bar(&mut self, ui: &mut egui::Ui) {
        if self.selection.is_empty() {
            return;
        }
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return;
        };
        let count = self.selection.len();
        // De-duplicate nested selections so a folder + a child inside it isn't
        // counted twice in the reclaimable total.
        let independent = independent_nodes(tree, self.selection.iter().copied().collect());
        let total: u64 = independent.iter().map(|&id| tree.node(id).size).sum();

        let mut action = None;
        egui::Panel::top("selection").show_inside(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{count} selected")).color(theme::ACCENT).strong());
                ui.label(egui::RichText::new(format!("·  {}", format_size(total))).color(theme::TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").clicked() {
                        action = Some(SelAction::Clear);
                    }
                    let del = egui::Button::new(
                        egui::RichText::new("Delete…").color(egui::Color32::WHITE),
                    )
                    .fill(theme::DANGER);
                    if ui.add(del).clicked() {
                        action = Some(SelAction::Delete);
                    }
                    if ui.button("Move to Trash…").clicked() {
                        action = Some(SelAction::Trash);
                    }
                });
            });
            ui.add_space(3.0);
        });

        match action {
            Some(SelAction::Clear) => self.selection.clear(),
            Some(SelAction::Trash) | Some(SelAction::Delete) => {
                let permanent = matches!(action, Some(SelAction::Delete));
                let ids: Vec<NodeId> = self.selection.iter().copied().collect();
                match prepare_delete(tree, &self.home, ids, permanent) {
                    Ok(dialog) => self.dialog = dialog,
                    Err(msg) => self.status = Some(msg),
                }
            }
            None => {}
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| match (self.scan.as_ref(), self.hovered) {
                (Some(Scan::Done { tree, .. }), Some(id)) => {
                    let node = tree.node(id);
                    ui.monospace(tree.path(id).display().to_string());
                    ui.label(
                        egui::RichText::new(format!("·  {}", format_size(node.size)))
                            .color(theme::TEXT_DIM),
                    );
                    let cat = theme::Category::of(tree, id);
                    ui.label(egui::RichText::new(format!("·  {}", cat.label())).color(cat.color()));
                    if node.is_mountpoint() {
                        ui.label(egui::RichText::new("·  mount point").color(theme::MOUNT));
                    }
                    if node.is_hardlinked() {
                        ui.label(egui::RichText::new("·  hardlinked").color(theme::ACCENT));
                    }
                    if let Some(warn) = safety_note(classify(&tree.path(id), &self.home)) {
                        ui.label(egui::RichText::new(warn).color(theme::DANGER));
                    }
                }
                (Some(Scan::Done { .. }), None) => {
                    let hint = self.status.clone().unwrap_or_else(|| {
                        "click to drill · Ctrl/Shift-click to select · right-click for actions"
                            .to_owned()
                    });
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
            let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
                let running = matches!(self.scan.as_ref(), Some(Scan::Running { .. }));
                let msg = match self.scan.as_ref() {
                    Some(Scan::Running { started, .. }) => {
                        format!("Scanning…  {:.1}s", started.elapsed().as_secs_f64())
                    }
                    Some(Scan::Error(e)) => format!("Scan failed:\n{e}"),
                    _ => String::new(),
                };
                let mut back = false;
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    let color = if running { theme::TEXT_DIM } else { theme::DANGER };
                    ui.label(egui::RichText::new(msg).color(color).size(16.0));
                    ui.add_space(12.0);
                    let label = if running { "Cancel" } else { "Back to devices" };
                    if ui.button(label).clicked() {
                        back = true;
                    }
                });
                if back {
                    if let Some(scan) = &self.scan {
                        scan.cancel();
                    }
                    self.scan = None;
                }
                return;
            };

            let now = ui.ctx().input(|i| i.time);

            // Keyboard shortcuts (suppressed while a dialog is open).
            if matches!(self.dialog, Dialog::None) {
                let (backspace, escape, delete, shift, select_all) = ui.ctx().input(|i| {
                    (
                        i.key_pressed(egui::Key::Backspace),
                        i.key_pressed(egui::Key::Escape),
                        i.key_pressed(egui::Key::Delete),
                        i.modifiers.shift,
                        (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::A),
                    )
                });

                // Esc clears a selection first; otherwise Esc/Backspace go up.
                if escape && !self.selection.is_empty() {
                    self.selection.clear();
                } else if (backspace || escape) && self.anim.is_none() {
                    if let Some(parent) = tree.node(self.current).parent {
                        self.anim = zoom_out_pivot(tree, self.current, parent, self.last_area).map(
                            |(child, pivot)| AnimState {
                                parent,
                                child,
                                pivot,
                                drilling_in: false,
                                start: None,
                            },
                        );
                        self.current = parent;
                    }
                }

                if select_all {
                    self.selection
                        .extend(tree.node(self.current).children.iter().copied());
                }
                if delete && !self.selection.is_empty() {
                    let ids: Vec<NodeId> = self.selection.iter().copied().collect();
                    match prepare_delete(tree, &self.home, ids, shift) {
                        Ok(dialog) => self.dialog = dialog,
                        Err(msg) => self.status = Some(msg),
                    }
                }
            }

            let anim = if let Some(a) = &mut self.anim {
                let start = *a.start.get_or_insert(now);
                let t = (now - start) / ANIM_SECS;
                if t >= 1.0 {
                    None
                } else {
                    Some(treemap_view::Anim {
                        parent: a.parent,
                        child: a.child,
                        pivot: a.pivot,
                        e: ease_out_cubic(t as f32),
                        drilling_in: a.drilling_in,
                    })
                }
            } else {
                None
            };
            if self.anim.is_some() && anim.is_none() {
                self.anim = None;
            }

            let it = treemap_view::show(ui, tree, self.current, anim, &self.selection);
            self.last_area = it.area;
            self.hovered = it.hovered.map(|h| h.id);

            if let Some(hit) = it.clicked {
                if it.modified {
                    let target = it.hovered.map(|h| h.id).unwrap_or(hit.id);
                    if !self.selection.insert(target) {
                        self.selection.remove(&target);
                    }
                    self.status = None;
                } else {
                    let node = tree.node(hit.id);
                    if node.kind == NodeKind::Dir && !node.children.is_empty() {
                        self.anim = Some(AnimState {
                            parent: self.current,
                            child: hit.id,
                            pivot: hit.rect,
                            drilling_in: true,
                            start: None,
                        });
                        self.current = hit.id;
                    }
                }
            }
            if let Some(hit) = it.secondary {
                self.menu = Some(hit.id);
            }

            let mut menu_action = None;
            it.response.context_menu(|ui| {
                if let Some(target) = self.menu {
                    menu_action = context_menu_items(ui, tree, target, &self.selection);
                }
            });
            match menu_action {
                Some(MenuAction::Properties(id)) => self.dialog = Dialog::Properties(id),
                Some(MenuAction::Reveal(id)) => ops::reveal(&tree.path(id)),
                Some(MenuAction::Trash(ids)) => match prepare_delete(tree, &self.home, ids, false) {
                    Ok(dialog) => self.dialog = dialog,
                    Err(msg) => self.status = Some(msg),
                },
                Some(MenuAction::Delete(ids)) => match prepare_delete(tree, &self.home, ids, true) {
                    Ok(dialog) => self.dialog = dialog,
                    Err(msg) => self.status = Some(msg),
                },
                None => {}
            }

            if self.anim.is_some() {
                ui.ctx().request_repaint();
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        match dialog {
            Dialog::None => {}
            Dialog::Properties(id) => {
                if self.show_properties(ctx, id) {
                    self.dialog = Dialog::Properties(id);
                }
            }
            Dialog::Confirm { ids, permanent } => match self.show_confirm(ctx, &ids, permanent) {
                Confirm::Open => self.dialog = Dialog::Confirm { ids, permanent },
                Confirm::Cancel => {}
                Confirm::Go => self.execute_delete(ids, permanent),
            },
            Dialog::Shortcuts => {
                if show_shortcuts(ctx) {
                    self.dialog = Dialog::Shortcuts;
                }
            }
        }
    }

    fn show_properties(&self, ctx: &egui::Context, id: NodeId) -> bool {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return false;
        };
        let node = tree.node(id);
        let path = tree.path(id);
        let mut keep = true;
        let response = egui::Modal::new(egui::Id::new("properties")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading("Properties");
            ui.add_space(6.0);
            egui::Grid::new("props")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    prop_row(ui, "Path", path.display().to_string());
                    prop_row(ui, "On disk", format_size(node.size));
                    prop_row(ui, "Category", theme::Category::of(tree, id).label().to_owned());
                    prop_row(ui, "Kind", kind_label(node.kind).to_owned());
                    if node.kind == NodeKind::Dir {
                        prop_row(ui, "Items", node.children.len().to_string());
                    }
                    if node.is_hardlinked() {
                        prop_row(ui, "Hard links", node.nlink.to_string());
                    }
                    if node.is_mountpoint() {
                        prop_row(ui, "Mount", "subvolume / mount point".to_owned());
                    }
                    prop_row(ui, "Safety", class_label(classify(&path, &self.home)).to_owned());
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Reveal in file manager").clicked() {
                    ops::reveal(&path);
                }
                if ui.button("Close").clicked() {
                    keep = false;
                }
            });
        });
        if response.should_close() {
            keep = false;
        }
        keep
    }

    fn show_confirm(&self, ctx: &egui::Context, ids: &[NodeId], permanent: bool) -> Confirm {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return Confirm::Cancel;
        };
        let total: u64 = ids.iter().map(|&id| tree.node(id).size).sum();
        let outside = ids
            .iter()
            .filter(|&&id| safety_note(classify(&tree.path(id), &self.home)).is_some())
            .count();

        let mut result = Confirm::Open;
        let response = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(520.0);
            let title = if permanent {
                "Delete permanently"
            } else {
                "Move to Trash"
            };
            ui.heading(
                egui::RichText::new(title)
                    .color(if permanent { theme::DANGER } else { theme::TEXT }),
            );
            ui.add_space(6.0);
            ui.label(format!("{} item(s)  ·  {} total", ids.len(), format_size(total)));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                for &id in ids.iter().take(15) {
                    ui.monospace(tree.path(id).display().to_string());
                }
                if ids.len() > 15 {
                    ui.label(format!("… and {} more", ids.len() - 15));
                }
            });
            ui.add_space(8.0);
            if permanent {
                ui.label(egui::RichText::new("This cannot be undone.").color(theme::DANGER).strong());
            } else {
                ui.label(
                    egui::RichText::new("Items can be restored from the trash.")
                        .color(theme::TEXT_DIM),
                );
            }
            if outside > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠  {outside} item(s) are outside your home directory."
                    ))
                    .color(theme::DANGER),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = Confirm::Cancel;
                }
                let go = if permanent {
                    "Delete permanently"
                } else {
                    "Move to Trash"
                };
                let button = egui::Button::new(egui::RichText::new(go).color(egui::Color32::WHITE))
                    .fill(if permanent { theme::DANGER } else { theme::Category::App.color() });
                if ui.add(button).clicked() {
                    result = Confirm::Go;
                }
            });
        });
        if response.should_close() && matches!(result, Confirm::Open) {
            result = Confirm::Cancel;
        }
        result
    }

    fn execute_delete(&mut self, ids: Vec<NodeId>, permanent: bool) {
        let Some(Scan::Done { tree, .. }) = &mut self.scan else {
            return;
        };
        let targets: Vec<(PathBuf, u64)> = ids
            .iter()
            .map(|&id| (tree.path(id), tree.node(id).size))
            .collect();
        let mode = if permanent {
            ops::Mode::Delete
        } else {
            ops::Mode::Trash
        };
        let report = ops::perform(&targets, mode);
        let mut removed: HashSet<NodeId> = HashSet::new();
        for (&id, (path, _)) in ids.iter().zip(&targets) {
            if report.succeeded.contains(path) {
                tree.remove_subtree(id);
                removed.insert(id);
            }
        }

        // If we just deleted the folder being viewed (or an ancestor of it),
        // retreat to the nearest surviving ancestor — otherwise the treemap would
        // keep rendering a subtree that no longer exists on disk.
        let mut node = self.current;
        let mut landing = None;
        loop {
            if removed.contains(&node) {
                landing = tree.node(node).parent;
            }
            match tree.node(node).parent {
                Some(parent) => node = parent,
                None => break,
            }
        }
        if let Some(target) = landing {
            self.current = target;
            self.anim = None;
            self.hovered = None;
        }

        self.status = Some(report.summary());
        self.selection.clear();
    }
}

/// One clickable filesystem row in the device picker.
fn disk_row(ui: &mut egui::Ui, disk: &DiskInfo) -> egui::Response {
    let used = disk.total.saturating_sub(disk.available);
    let frac = if disk.total > 0 {
        (used as f64 / disk.total as f64) as f32
    } else {
        0.0
    };
    let inner = egui::Frame::group(ui.style())
        .fill(theme::PANEL)
        .show(ui, |ui| {
            ui.set_width(520.0);
            ui.label(
                egui::RichText::new(disk.mount.display().to_string())
                    .monospace()
                    .strong()
                    .color(theme::TEXT),
            );
            ui.label(
                egui::RichText::new(format!("{}  ·  {}", disk.name, disk.fs))
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.add(egui::ProgressBar::new(frac).text(format!(
                "{} used · {} free · {} total",
                format_size(used),
                format_size(disk.available),
                format_size(disk.total),
            )));
        });
    inner
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Build the right-click context menu; returns the action chosen, if any.
fn context_menu_items(
    ui: &mut egui::Ui,
    tree: &Tree,
    target: NodeId,
    selection: &HashSet<NodeId>,
) -> Option<MenuAction> {
    // Operate on the whole selection if the clicked cell is part of it.
    let ids: Vec<NodeId> = if selection.contains(&target) {
        selection.iter().copied().collect()
    } else {
        vec![target]
    };
    let n = ids.len();

    let mut action = None;
    ui.label(egui::RichText::new(tree.name(target)).strong());
    ui.separator();
    if ui.button("Properties").clicked() {
        action = Some(MenuAction::Properties(target));
        ui.close();
    }
    if ui.button("Reveal in file manager").clicked() {
        action = Some(MenuAction::Reveal(target));
        ui.close();
    }
    ui.separator();
    let trash = if n > 1 {
        format!("Move {n} items to Trash")
    } else {
        "Move to Trash".to_owned()
    };
    if ui.button(trash).clicked() {
        action = Some(MenuAction::Trash(ids.clone()));
        ui.close();
    }
    let delete = if n > 1 {
        format!("Delete {n} items permanently…")
    } else {
        "Delete permanently…".to_owned()
    };
    if ui
        .button(egui::RichText::new(delete).color(theme::DANGER))
        .clicked()
    {
        action = Some(MenuAction::Delete(ids));
        ui.close();
    }
    action
}

/// Filter `ids` down to those with no selected ancestor (so deleting a folder
/// and something inside it doesn't double-count or fail).
fn independent_nodes(tree: &Tree, ids: Vec<NodeId>) -> Vec<NodeId> {
    let set: HashSet<NodeId> = ids.iter().copied().collect();
    ids.into_iter()
        .filter(|&id| {
            let mut ancestor = tree.node(id).parent;
            while let Some(a) = ancestor {
                if set.contains(&a) {
                    return false;
                }
                ancestor = tree.node(a).parent;
            }
            true
        })
        .collect()
}

/// Prepare a delete: dedup nested selections and refuse critical paths.
fn prepare_delete(
    tree: &Tree,
    home: &std::path::Path,
    ids: Vec<NodeId>,
    permanent: bool,
) -> Result<Dialog, String> {
    let ids = independent_nodes(tree, ids);
    if ids.is_empty() {
        return Err("Nothing to delete".to_owned());
    }
    for &id in &ids {
        let path = tree.path(id);
        if classify(&path, home) == Class::Critical {
            return Err(format!(
                "Refused — {} is a protected system location",
                path.display()
            ));
        }
    }
    Ok(Dialog::Confirm { ids, permanent })
}

fn prop_row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.label(egui::RichText::new(key).color(theme::TEXT_DIM));
    ui.monospace(value);
    ui.end_row();
}

/// The keyboard / mouse reference modal. Returns whether it should stay open.
fn show_shortcuts(ctx: &egui::Context) -> bool {
    let mut keep = true;
    let response = egui::Modal::new(egui::Id::new("shortcuts")).show(ctx, |ui| {
        ui.set_width(450.0);
        ui.heading("Keyboard & mouse");
        ui.add_space(6.0);
        egui::Grid::new("keys")
            .num_columns(2)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                key_row(ui, "Click", "Open a folder (drill in)");
                key_row(ui, "Ctrl / Shift-click", "Add a cell to the selection");
                key_row(ui, "Right-click", "Context menu (properties, delete, …)");
                key_row(ui, "Backspace", "Go up a level");
                key_row(ui, "Esc", "Clear the selection, or go up");
                key_row(ui, "Ctrl+A", "Select everything in view");
                key_row(ui, "Delete", "Move the selection to Trash");
                key_row(ui, "Shift+Delete", "Delete the selection permanently");
                key_row(ui, "F5", "Rescan");
            });
        ui.add_space(10.0);
        if ui.button("Close").clicked() {
            keep = false;
        }
    });
    if response.should_close() {
        keep = false;
    }
    keep
}

fn key_row(ui: &mut egui::Ui, key: &str, desc: &str) {
    ui.label(egui::RichText::new(key).monospace().strong().color(theme::ACCENT));
    ui.label(egui::RichText::new(desc).color(theme::TEXT));
    ui.end_row();
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Dir => "Folder",
        NodeKind::File => "File",
        NodeKind::Symlink => "Symlink",
        NodeKind::Other => "Special file",
    }
}

fn class_label(class: Class) -> &'static str {
    match class {
        Class::Normal => "in home directory",
        Class::OutsideHome => "outside home directory",
        Class::System => "system location",
        Class::Critical => "protected (cannot delete)",
    }
}

/// A short status-bar warning for a non-normal safety class.
fn safety_note(class: Class) -> Option<&'static str> {
    match class {
        Class::Normal => None,
        Class::OutsideHome => Some("·  ⚠ outside home"),
        Class::System => Some("·  ⚠ system"),
        Class::Critical => Some("·  ⛔ protected"),
    }
}

/// Enumerate mounted filesystems with real capacity, one row per device.
fn list_disks() -> Vec<DiskInfo> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<DiskInfo> = disks
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            mount: d.mount_point().to_path_buf(),
            fs: d.file_system().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.mount.as_os_str().len().cmp(&b.mount.as_os_str().len()))
    });
    out.dedup_by(|a, b| a.name == b.name);
    out.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.mount.cmp(&b.mount)));
    out
}

/// The child of `target` on the path up from `from`, plus its screen rect.
fn zoom_out_pivot(tree: &Tree, from: NodeId, target: NodeId, area: ERect) -> Option<(NodeId, ERect)> {
    let mut node = from;
    while let Some(parent) = tree.node(node).parent {
        if parent == target {
            let rect = treemap_view::child_rect(tree, target, node, area)?;
            return Some((node, rect));
        }
        node = parent;
    }
    None
}

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
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
