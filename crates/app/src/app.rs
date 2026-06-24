//! The application: a device picker, navigation state, and the three-panel
//! treemap shell (toolbar / treemap / status bar).
//!
//! On launch with no path argument the app shows a **device picker** listing the
//! mounted filesystems with their used/total space; pick one to scan it. (Each
//! mount is its own filesystem — on btrfs your `/home`, `/var`, … are separate
//! subvolumes, which is why scanning `/` only shows the root subvolume.)
//!
//! While viewing, navigation is a single `current` node id; the breadcrumb and
//! "go up" are derived from the scanner's parent pointers, and every change of
//! `current` cross-fades a short zoom between the two views.

use std::path::PathBuf;

use eframe::egui::{self, Rect as ERect};
use scanner::{NodeId, NodeKind, Tree};

use crate::scan::Scan;
use crate::theme::{self, format_size};
use crate::treemap_view;

/// Zoom duration in seconds.
const ANIM_SECS: f64 = 0.22;

pub struct StorageSifterApp {
    /// Mounted filesystems for the picker.
    disks: Vec<DiskInfo>,
    /// The active scan, or `None` to show the device picker.
    scan: Option<Scan>,
    /// The path currently being visualized.
    path: PathBuf,
    /// The node currently drilled into (root is 0; reset on every scan).
    current: NodeId,
    /// In-progress zoom, if any.
    anim: Option<AnimState>,
    /// The treemap rect from last frame, used to locate cells for "go up" zooms.
    last_area: ERect,
    /// Node under the pointer, refreshed each frame by the treemap view.
    hovered: Option<NodeId>,
}

/// A mounted filesystem shown in the picker.
struct DiskInfo {
    name: String,
    mount: PathBuf,
    fs: String,
    total: u64,
    available: u64,
}

#[derive(Clone, Copy)]
struct AnimState {
    /// The two views being cross-faded, connected at `pivot` (the child's cell
    /// within the parent's layout). `current` is already the destination.
    parent: NodeId,
    child: NodeId,
    pivot: ERect,
    drilling_in: bool,
    /// `egui` time when the first frame rendered; stamped lazily so the zoom
    /// always begins at t = 0 no matter when it was queued.
    start: Option<f64>,
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
        }
    }

    fn open(&mut self, path: PathBuf) {
        self.scan = Some(Scan::start(&path));
        self.path = path;
        self.current = 0;
        self.anim = None;
        self.hovered = None;
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

        self.toolbar(ui);
        self.status_bar(ui);
        self.treemap(ui);
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
                    self.scan = None;
                    self.disks = list_disks();
                }
                if ui.button("⟳  Rescan").clicked() {
                    self.open(self.path.clone());
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

    /// Clickable breadcrumb from the scan root down to `current`.
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
                }
                (Some(Scan::Done { tree, .. }), None) => {
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
            let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
                let msg = match self.scan.as_ref() {
                    Some(Scan::Running { started, .. }) => {
                        format!("Scanning…  {:.1}s", started.elapsed().as_secs_f64())
                    }
                    Some(Scan::Error(e)) => format!("Scan failed:\n{e}"),
                    _ => String::new(),
                };
                center_message(ui, &msg);
                return;
            };

            let now = ui.ctx().input(|i| i.time);

            // Keyboard: go up a level (zoom out of the cell we leave).
            let go_up = ui
                .ctx()
                .input(|i| i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Escape));
            if go_up && self.anim.is_none() {
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

            // Advance the cross-fade; clear when it finishes.
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

            let it = treemap_view::show(ui, tree, self.current, anim);
            self.last_area = it.area;
            self.hovered = it.hovered.map(|h| h.id);

            // Drill into the clicked folder: cross-fade into it.
            if let Some(hit) = it.clicked {
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

            if self.anim.is_some() {
                ui.ctx().request_repaint();
            }
        });
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

/// Enumerate mounted filesystems with real capacity.
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
    // One entry per physical device: btrfs mounts every subvolume separately
    // (/, /home, /var, …) on the same device, so collapse to the shortest mount.
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.mount.as_os_str().len().cmp(&b.mount.as_os_str().len()))
    });
    out.dedup_by(|a, b| a.name == b.name);
    out.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.mount.cmp(&b.mount)));
    out
}

/// The child of `target` on the path up from `from`, plus its screen rect in
/// `target`'s layout — the pivot cell for a zoom-out.
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

fn center_message(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(theme::TEXT_DIM).size(16.0));
    });
}
