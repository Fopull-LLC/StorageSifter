//! Visual theme: a dark, flat, programmer palette plus file-category colors and
//! human-readable size formatting.

use eframe::egui::{self, Color32};
use scanner::{NodeId, NodeKind, Tree};

// Core chrome colors. Flat, dark, high-contrast — no gradients.
pub const BG: Color32 = Color32::from_rgb(0x14, 0x16, 0x1b); // treemap backdrop
pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x26); // toolbars
pub const BORDER: Color32 = Color32::from_rgb(0x0d, 0x0f, 0x13); // hard cell edges
pub const TEXT: Color32 = Color32::from_rgb(0xc8, 0xcd, 0xd6);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7a, 0x82, 0x90);
pub const ACCENT: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b); // hover / selection

/// Install the dark theme into the egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}

/// Broad file categories, colored so the disk is readable at a glance.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Dir,
    Code,
    Image,
    Video,
    Audio,
    Archive,
    Document,
    Binary,
    Other,
}

impl Category {
    /// Classify a node by kind, then by file extension.
    pub fn of(tree: &Tree, id: NodeId) -> Category {
        if tree.node(id).kind == NodeKind::Dir {
            return Category::Dir;
        }
        let ext = tree
            .name(id)
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "go" | "java" | "sh"
            | "toml" | "json" | "lock" | "yaml" | "yml" | "md" | "html" | "css" => Category::Code,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "tiff" => {
                Category::Image
            }
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" => Category::Video,
            "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" => Category::Audio,
            "zip" | "gz" | "xz" | "zst" | "bz2" | "tar" | "7z" | "rar" | "pkg" => Category::Archive,
            "pdf" | "txt" | "doc" | "docx" | "odt" | "epub" | "csv" | "xlsx" => Category::Document,
            "so" | "o" | "a" | "bin" | "exe" | "dll" | "wasm" | "rlib" => Category::Binary,
            _ => Category::Other,
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Category::Dir => Color32::from_rgb(0x39, 0x46, 0x5e),
            Category::Code => Color32::from_rgb(0x61, 0xaf, 0xef),
            Category::Image => Color32::from_rgb(0x98, 0xc3, 0x79),
            Category::Video => Color32::from_rgb(0xc6, 0x78, 0xdd),
            Category::Audio => Color32::from_rgb(0x56, 0xb6, 0xc2),
            Category::Archive => Color32::from_rgb(0xe0, 0x6c, 0x75),
            Category::Document => Color32::from_rgb(0xe5, 0xc0, 0x7b),
            Category::Binary => Color32::from_rgb(0xd1, 0x9a, 0x66),
            Category::Other => Color32::from_rgb(0x55, 0x5c, 0x69),
        }
    }
}

/// Pick black or white text for legibility against a filled cell.
pub fn contrast_text(bg: Color32) -> Color32 {
    // Rec. 601 luma.
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma > 140.0 {
        Color32::from_rgb(0x14, 0x16, 0x1b)
    } else {
        Color32::from_rgb(0xe8, 0xeb, 0xf0)
    }
}

/// Format a byte count with binary units (matches the on-disk numbers).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
