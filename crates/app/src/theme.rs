//! Visual theme: a dark, flat, programmer palette plus *semantic* file-category
//! colors and human-readable size formatting.
//!
//! The categories are chosen to answer the question "what is taking up my
//! space, and is it junk?" at a glance — caches and build artifacts get a loud
//! amber, media a calm green, applications cyan, and so on.

use eframe::egui::{self, Color32};
use scanner::{NodeId, NodeKind, Tree};

// Core chrome colors. Flat, dark, high-contrast — no gradients.
pub const BG: Color32 = Color32::from_rgb(0x14, 0x16, 0x1b); // treemap backdrop
pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x26); // toolbars
pub const BORDER: Color32 = Color32::from_rgb(0x0d, 0x0f, 0x13); // hard cell edges
pub const TEXT: Color32 = Color32::from_rgb(0xc8, 0xcd, 0xd6);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7a, 0x82, 0x90);
pub const ACCENT: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b); // hover / selection
pub const MOUNT: Color32 = Color32::from_rgb(0x6f, 0xd0, 0xe8); // mount / subvolume edge

/// Install the dark theme into the egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}

/// A "what is this" classification, tuned to surface reclaimable space.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Caches, build artifacts, temp files — the usual reclaimable garbage.
    Junk,
    /// Video / audio / images — usually large and usually wanted.
    Media,
    /// Archives and disk images.
    Archive,
    /// Applications, installers, libraries, executables.
    App,
    /// Source code and config.
    Code,
    /// Documents and text.
    Document,
    /// A plain folder (nothing more specific).
    Folder,
    /// Anything else.
    Other,
}

/// Directory names that are almost always reclaimable caches / build output.
const JUNK_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    ".cache",
    "cache",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".gradle",
    ".ccache",
    ".npm",
    ".yarn",
    ".venv",
    "venv",
    ".next",
    ".nuxt",
    "cmakefiles",
    ".tox",
    "gpucache",
    "shadercache",
];

impl Category {
    /// Classify a node by kind, then by name / extension.
    ///
    /// Junk propagates: anything *inside* a known cache/build directory is junk
    /// too, so a whole `target/` or `node_modules/` reads as one solid amber
    /// block — the reclaimable space you're hunting for.
    pub fn of(tree: &Tree, id: NodeId) -> Category {
        let mut ancestor = Some(id);
        while let Some(node) = ancestor {
            if tree.node(node).kind == NodeKind::Dir && is_junk_dir(tree.name(node)) {
                return Category::Junk;
            }
            ancestor = tree.node(node).parent;
        }

        if tree.node(id).kind == NodeKind::Dir {
            return Category::Folder;
        }
        let ext = tree
            .name(id)
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "tmp" | "temp" | "log" | "bak" | "swp" | "swo" | "old" | "part" | "crdownload"
            | "pyc" | "pyo" | "class" | "o" | "obj" => Category::Junk,
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "m4v" | "mp3" | "flac"
            | "wav" | "ogg" | "opus" | "m4a" | "aac" | "png" | "jpg" | "jpeg" | "gif" | "webp"
            | "bmp" | "tiff" | "svg" | "heic" | "raw" | "cr2" | "nef" | "psd" => Category::Media,
            "zip" | "tar" | "gz" | "xz" | "zst" | "bz2" | "7z" | "rar" | "iso" | "dmg" => {
                Category::Archive
            }
            "exe" | "msi" | "appimage" | "deb" | "rpm" | "pkg" | "flatpak" | "snap" | "so"
            | "dll" | "dylib" | "bin" | "elf" | "a" | "ko" | "wasm" | "rlib" => Category::App,
            "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "py" | "js" | "jsx" | "ts" | "tsx" | "go"
            | "java" | "kt" | "rb" | "php" | "html" | "css" | "scss" | "json" | "toml" | "yaml"
            | "yml" | "xml" | "sh" | "bash" | "lua" | "sql" | "vue" | "svelte" => Category::Code,
            "pdf" | "doc" | "docx" | "odt" | "ods" | "odp" | "txt" | "md" | "rtf" | "epub"
            | "mobi" | "csv" | "xlsx" | "pptx" | "tex" => Category::Document,
            _ => Category::Other,
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Category::Junk => Color32::from_rgb(0xd1, 0x88, 0x3c), // amber — reclaimable
            Category::Media => Color32::from_rgb(0x9e, 0xce, 0x6a), // green
            Category::Archive => Color32::from_rgb(0xd1, 0x7f, 0xb0), // pink
            Category::App => Color32::from_rgb(0x56, 0xb6, 0xc2),  // cyan
            Category::Code => Color32::from_rgb(0x61, 0xaf, 0xef), // blue
            Category::Document => Color32::from_rgb(0xe5, 0xc0, 0x7b), // yellow
            Category::Folder => Color32::from_rgb(0x49, 0x56, 0x6e), // slate
            Category::Other => Color32::from_rgb(0x5a, 0x62, 0x73), // gray
        }
    }

    /// Short, human label for the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Category::Junk => "cache / junk",
            Category::Media => "media",
            Category::Archive => "archive",
            Category::App => "application",
            Category::Code => "code",
            Category::Document => "document",
            Category::Folder => "folder",
            Category::Other => "other",
        }
    }
}

fn is_junk_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    JUNK_DIRS.contains(&lower.as_str())
}

/// Pick black or white text for legibility against a filled cell.
pub fn contrast_text(bg: Color32) -> Color32 {
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
