//! StorageSifter — desktop disk-usage treemap.
//!
//! Phase 0: an empty, dark, GPU-backed (wgpu) window that confirms the UI
//! toolchain is wired up end to end. The scanning UI, treemap rendering, and
//! drill-down interactions arrive in later phases.

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("StorageSifter")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "StorageSifter",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::<StorageSifter>::default())
        }),
    )
}

#[derive(Default)]
struct StorageSifter;

impl eframe::App for StorageSifter {
    // eframe 0.34 sets up the central panel for us and hands over its `Ui`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.4);
            ui.heading("StorageSifter");
            ui.label("Phase 0 — scaffold online. The treemap arrives in Phase 3.");
        });
    }
}
