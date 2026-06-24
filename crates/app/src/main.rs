//! StorageSifter — desktop disk-usage treemap.
//!
//! Phase 3: a static squarified treemap of a scanned directory, rendered on the
//! wgpu backend with the dark theme. Scanning runs on a background thread and
//! the view fills in when it completes. Drill-down and animation come in Phase 4.
//!
//! Usage: `storagesifter [PATH]` (defaults to $HOME).

mod app;
mod scan;
mod theme;
mod treemap_view;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

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
