//! StorageSifter — desktop disk-usage treemap.
//!
//! An interactive squarified treemap of a scanned filesystem, rendered on the
//! wgpu backend. Launch with no argument to pick a device; pass a `PATH` to scan
//! it directly. Scanning runs on a background thread and the view fills in when
//! it completes.
//!
//! Usage: `storagesifter [PATH]`

mod app;
mod assess;
mod ops;
mod scan;
mod settings;
mod theme;
mod treemap_view;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // A path argument scans directly; with none, the device picker is shown.
    let path = std::env::args_os().nth(1).map(PathBuf::from);

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
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::StorageSifterApp::new(path)))
        }),
    )
}
