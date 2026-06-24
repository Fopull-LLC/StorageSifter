//! The application: state, the per-frame UI, and the three-panel shell
//! (toolbar / treemap / status bar).

use std::path::PathBuf;

use eframe::egui;
use scanner::NodeId;

use crate::scan::Scan;
use crate::theme::{self, format_size};
use crate::treemap_view;

pub struct StorageSifterApp {
    /// The directory currently being visualized.
    path: PathBuf,
    /// Background scan state.
    scan: Scan,
    /// Node under the pointer, refreshed each frame by the treemap view.
    hovered: Option<NodeId>,
}

impl StorageSifterApp {
    pub fn new(path: PathBuf) -> Self {
        let scan = Scan::start(&path);
        Self {
            path,
            scan,
            hovered: None,
        }
    }
}

impl eframe::App for StorageSifterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Advance the scan; keep repainting while it runs so we poll the channel.
        self.scan.poll();
        if self.scan.is_running() {
            ui.ctx().request_repaint();
        }

        self.toolbar(ui);
        self.status_bar(ui);

        egui::CentralPanel::default().show_inside(ui, |ui| match &self.scan {
            Scan::Running { started, .. } => {
                center_message(ui, &format!("Scanning… {:.1}s", started.elapsed().as_secs_f64()));
            }
            Scan::Error(error) => center_message(ui, &format!("Scan failed:\n{error}")),
            Scan::Done { tree, .. } => {
                let root = tree.root;
                self.hovered = treemap_view::show(ui, tree, root);
            }
        });
    }
}

impl StorageSifterApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("⟳  Rescan").clicked() {
                    self.scan = Scan::start(&self.path);
                    self.hovered = None;
                }
                ui.separator();
                ui.monospace(self.path.display().to_string());

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
                    if node.is_hardlinked() {
                        ui.label(egui::RichText::new("·  hardlinked").color(theme::ACCENT));
                    }
                }
                (Scan::Done { tree, .. }, None) => {
                    let n = tree.unreadable.len();
                    let msg = if n == 0 {
                        "hover a cell to inspect it".to_owned()
                    } else {
                        format!("hover a cell to inspect it  ·  {n} unreadable path(s)")
                    };
                    ui.label(egui::RichText::new(msg).color(theme::TEXT_DIM));
                }
                _ => {
                    ui.label("");
                }
            });
            ui.add_space(2.0);
        });
    }
}

fn center_message(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(theme::TEXT_DIM).size(16.0));
    });
}
