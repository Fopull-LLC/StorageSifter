//! Visual theme: a dark, cool, sleek palette plus *semantic* file-category
//! colors and human-readable size formatting.
//!
//! The palette stays in a cohesive cool family — dark saturated backdrop, near
//! white text, blues / teals / purples for structure and types, and a single
//! pink reserved for the one thing you're hunting: reclaimable junk. Red is used
//! only for destructive actions, never as decoration.

use eframe::egui::{self, Color32};
use scanner::{NodeId, NodeKind, Tree};

// Core chrome. Dark, faintly indigo, flat — no gradients.
pub const BG: Color32 = Color32::from_rgb(0x11, 0x13, 0x20); // treemap backdrop
pub const PANEL: Color32 = Color32::from_rgb(0x19, 0x1c, 0x2b); // toolbars / dialogs
pub const BORDER: Color32 = Color32::from_rgb(0x0a, 0x0b, 0x12); // subtle cell gaps
pub const TEXT: Color32 = Color32::from_rgb(0xea, 0xed, 0xf4); // near-white
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x88, 0x8f, 0xa6); // muted
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x7e, 0xb6); // selection / highlight (pink)
pub const MOUNT: Color32 = Color32::from_rgb(0x58, 0xd2, 0xc2); // mount / subvolume edge (teal)
pub const DANGER: Color32 = Color32::from_rgb(0xec, 0x5f, 0x78); // destructive actions only

/// Install the dark theme into the egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    ctx.set_visuals(visuals);
}

/// A "what is this" classification, tuned to surface reclaimable space.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Junk,
    Media,
    Archive,
    App,
    Code,
    Document,
    Folder,
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
    /// too, so a whole `target/` or `node_modules/` reads as one solid block.
    pub fn of(tree: &Tree, id: NodeId) -> Category {
        let mut ancestor = Some(id);
        while let Some(node) = ancestor {
            // The scanner stores the full scanned path as the root node's name,
            // so fall back to its basename here — otherwise scanning a cache dir
            // directly (e.g. `/var/cache`) wouldn't read as junk. This only
            // allocates for the root, never for the per-cell common case.
            let junk = if node == tree.root {
                tree.path(node)
                    .file_name()
                    .is_some_and(|n| is_junk_dir(&n.to_string_lossy()))
            } else {
                is_junk_dir(tree.name(node))
            };
            if tree.node(node).kind == NodeKind::Dir && junk {
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
            Category::Junk => Color32::from_rgb(0xd9, 0x6a, 0x96), // rose — reclaimable
            Category::Media => Color32::from_rgb(0x46, 0xc2, 0xa2), // teal-green
            Category::Archive => Color32::from_rgb(0xc5, 0x6f, 0xd0), // purple-magenta
            Category::App => Color32::from_rgb(0x9b, 0x7b, 0xea),  // blue-purple
            Category::Code => Color32::from_rgb(0x6f, 0x8b, 0xf2), // blue
            Category::Document => Color32::from_rgb(0x54, 0xc2, 0xdd), // cyan
            Category::Folder => Color32::from_rgb(0x38, 0x45, 0x6a), // dark blue (structural)
            Category::Other => Color32::from_rgb(0x52, 0x5b, 0x72), // blue-gray
        }
    }

    /// Short, human label for the status bar / properties.
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
    if luma > 150.0 {
        Color32::from_rgb(0x11, 0x13, 0x20)
    } else {
        Color32::from_rgb(0xea, 0xed, 0xf4)
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
