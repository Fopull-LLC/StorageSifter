//! Visual theme: a customizable palette plus *semantic* file-category colors and
//! human-readable size formatting.
//!
//! The default palette stays in a cohesive cool family — dark saturated backdrop,
//! near-white text, blues / teals / purples for structure and types, and a single
//! pink reserved for the one thing you're hunting: reclaimable junk. Red is used
//! only for destructive actions, never as decoration.
//!
//! Colors are user-customizable: the active [`Palette`] lives behind a global so
//! UI chrome can read it through the `theme::*()` accessors, while the hot treemap
//! path snapshots it once per frame (see [`palette`]) to stay lock-free.

use std::sync::{LazyLock, RwLock};

use eframe::egui::{self, Color32};
use scanner::{NodeId, NodeKind, Tree};
use serde::{Deserialize, Serialize};

/// Serialize a `Color32` as a compact `[r, g, b]` triple.
mod color_rgb {
    use eframe::egui::Color32;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
        [c.r(), c.g(), c.b()].serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
        let [r, g, b] = <[u8; 3]>::deserialize(d)?;
        Ok(Color32::from_rgb(r, g, b))
    }
}

macro_rules! palette {
    ($($field:ident),+ $(,)?) => {
        /// A full set of theme colors. Customizable and persisted in settings.
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Palette {
            $(#[serde(with = "color_rgb")] pub $field: Color32,)+
        }

        impl Palette {
            /// Number of colors in a palette.
            pub const FIELDS: usize = [$(stringify!($field)),+].len();

            /// Pack the colors into RGB bytes, in field order.
            fn to_bytes(self) -> Vec<u8> {
                let mut v = Vec::with_capacity(Self::FIELDS * 3);
                $( v.extend_from_slice(&[self.$field.r(), self.$field.g(), self.$field.b()]); )+
                v
            }

            /// Rebuild from exactly `FIELDS * 3` RGB bytes, in field order.
            fn from_bytes(b: &[u8]) -> Option<Palette> {
                if b.len() != Self::FIELDS * 3 {
                    return None;
                }
                let mut chunks = b.chunks_exact(3);
                $(
                    let $field = {
                        let c = chunks.next()?;
                        Color32::from_rgb(c[0], c[1], c[2])
                    };
                )+
                Some(Palette { $($field,)+ })
            }
        }
    };
}

palette!(
    bg, panel, border, text, text_dim, accent, mount, danger, // chrome
    junk, media, archive, app, code, document, folder, other, // categories
);

impl Palette {
    /// The default cool/dark theme.
    pub const COOL_DARK: Palette = Palette {
        bg: Color32::from_rgb(0x11, 0x13, 0x20),
        panel: Color32::from_rgb(0x19, 0x1c, 0x2b),
        border: Color32::from_rgb(0x0a, 0x0b, 0x12),
        text: Color32::from_rgb(0xea, 0xed, 0xf4),
        text_dim: Color32::from_rgb(0x88, 0x8f, 0xa6),
        accent: Color32::from_rgb(0xff, 0x7e, 0xb6),
        mount: Color32::from_rgb(0x58, 0xd2, 0xc2),
        danger: Color32::from_rgb(0xec, 0x5f, 0x78),
        junk: Color32::from_rgb(0xd9, 0x6a, 0x96),
        media: Color32::from_rgb(0x46, 0xc2, 0xa2),
        archive: Color32::from_rgb(0xc5, 0x6f, 0xd0),
        app: Color32::from_rgb(0x9b, 0x7b, 0xea),
        code: Color32::from_rgb(0x6f, 0x8b, 0xf2),
        document: Color32::from_rgb(0x54, 0xc2, 0xdd),
        folder: Color32::from_rgb(0x38, 0x45, 0x6a),
        other: Color32::from_rgb(0x52, 0x5b, 0x72),
    };

    /// Maximum-legibility theme: near-black backdrop, pure-white text, vivid and
    /// well-separated category hues.
    pub const HIGH_CONTRAST: Palette = Palette {
        bg: Color32::from_rgb(0x04, 0x05, 0x09),
        panel: Color32::from_rgb(0x12, 0x14, 0x1e),
        border: Color32::from_rgb(0x00, 0x00, 0x00),
        text: Color32::from_rgb(0xff, 0xff, 0xff),
        text_dim: Color32::from_rgb(0xc2, 0xc9, 0xdb),
        accent: Color32::from_rgb(0xff, 0x55, 0xc4),
        mount: Color32::from_rgb(0x2f, 0xe9, 0xd6),
        danger: Color32::from_rgb(0xff, 0x53, 0x68),
        junk: Color32::from_rgb(0xff, 0x77, 0xb4),
        media: Color32::from_rgb(0x33, 0xe2, 0xb0),
        archive: Color32::from_rgb(0xdc, 0x7c, 0xff),
        app: Color32::from_rgb(0xae, 0x8c, 0xff),
        code: Color32::from_rgb(0x66, 0x9b, 0xff),
        document: Color32::from_rgb(0x4c, 0xd6, 0xf4),
        folder: Color32::from_rgb(0x4a, 0x59, 0x8e),
        other: Color32::from_rgb(0x76, 0x80, 0xa0),
    };

    /// A light theme for bright environments.
    pub const LIGHT: Palette = Palette {
        bg: Color32::from_rgb(0xf3, 0xf4, 0xf8),
        panel: Color32::from_rgb(0xe6, 0xe9, 0xf1),
        border: Color32::from_rgb(0xbf, 0xc5, 0xd6),
        text: Color32::from_rgb(0x1b, 0x1e, 0x2c),
        text_dim: Color32::from_rgb(0x5a, 0x61, 0x78),
        accent: Color32::from_rgb(0xcb, 0x46, 0x92),
        mount: Color32::from_rgb(0x1f, 0x9d, 0x90),
        danger: Color32::from_rgb(0xcf, 0x3f, 0x57),
        junk: Color32::from_rgb(0xd9, 0x6a, 0x96),
        media: Color32::from_rgb(0x2f, 0xa6, 0x88),
        archive: Color32::from_rgb(0xab, 0x5f, 0xc0),
        app: Color32::from_rgb(0x7b, 0x66, 0xd6),
        code: Color32::from_rgb(0x4f, 0x73, 0xe0),
        document: Color32::from_rgb(0x3f, 0xa6, 0xc6),
        folder: Color32::from_rgb(0xb6, 0xbe, 0xd6),
        other: Color32::from_rgb(0x97, 0x9f, 0xb6),
    };

    /// Presets offered in the settings dialog: (name, palette).
    pub const PRESETS: [(&'static str, Palette); 3] = [
        ("Cool Dark", Palette::COOL_DARK),
        ("High Contrast", Palette::HIGH_CONTRAST),
        ("Light", Palette::LIGHT),
    ];
}

impl Default for Palette {
    fn default() -> Self {
        Palette::COOL_DARK
    }
}

/// Prefix marking a StorageSifter palette code (format version 1).
const CODE_PREFIX: &str = "SSP1-";

impl Palette {
    /// Encode this palette as a short, shareable code (e.g. to send a friend).
    pub fn to_code(self) -> String {
        let mut bytes = self.to_bytes();
        bytes.push(checksum(&bytes));
        format!("{CODE_PREFIX}{}", b64_encode(&bytes))
    }

    /// Decode a palette code. Returns `None` if it's malformed, the wrong
    /// length, or the checksum doesn't match (so typos are rejected cleanly).
    pub fn from_code(code: &str) -> Option<Palette> {
        let body = code.trim().strip_prefix(CODE_PREFIX)?;
        let bytes = b64_decode(body)?;
        let (&sum, data) = bytes.split_last()?;
        if sum != checksum(data) {
            return None;
        }
        Palette::from_bytes(data)
    }
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[(n & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    for chunk in s.as_bytes().chunks(4) {
        let mut acc = 0u32;
        for &c in chunk {
            acc = (acc << 6) | val(c)?;
        }
        match chunk.len() {
            2 => out.push((acc >> 4) as u8),
            3 => {
                out.push((acc >> 10) as u8);
                out.push((acc >> 2) as u8);
            }
            4 => {
                out.push((acc >> 16) as u8);
                out.push((acc >> 8) as u8);
                out.push(acc as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

static ACTIVE: LazyLock<RwLock<Palette>> = LazyLock::new(|| RwLock::new(Palette::COOL_DARK));

/// Set the process-wide active palette.
pub fn set_palette(p: Palette) {
    if let Ok(mut guard) = ACTIVE.write() {
        *guard = p;
    }
}

/// A copy of the active palette. Cheap (48 bytes); the hot path snapshots once
/// per frame and reads fields directly rather than re-locking per cell.
pub fn palette() -> Palette {
    ACTIVE.read().map(|g| *g).unwrap_or(Palette::COOL_DARK)
}

// Convenience accessors for UI chrome (toolbars, dialogs, status bar). These
// take the lock per call, which is fine off the per-cell hot path.
pub fn bg() -> Color32 {
    palette().bg
}
pub fn panel() -> Color32 {
    palette().panel
}
pub fn text() -> Color32 {
    palette().text
}
pub fn text_dim() -> Color32 {
    palette().text_dim
}
pub fn accent() -> Color32 {
    palette().accent
}
pub fn mount() -> Color32 {
    palette().mount
}
pub fn danger() -> Color32 {
    palette().danger
}

fn luma(c: Color32) -> f32 {
    0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
}

/// Install the active palette into the egui context (call again when it changes).
pub fn apply(ctx: &egui::Context) {
    let p = palette();
    let mut visuals = if luma(p.bg) > 140.0 {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.bg;
    visuals.override_text_color = Some(p.text);
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.45);
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

    /// This category's fill color, from the given palette.
    pub fn color(self, pal: &Palette) -> Color32 {
        match self {
            Category::Junk => pal.junk,
            Category::Media => pal.media,
            Category::Archive => pal.archive,
            Category::App => pal.app,
            Category::Code => pal.code,
            Category::Document => pal.document,
            Category::Folder => pal.folder,
            Category::Other => pal.other,
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

/// Pick black or white text for legibility against a filled cell. Independent of
/// the theme — it only depends on the cell's own brightness.
pub fn contrast_text(bg: Color32) -> Color32 {
    if luma(bg) > 150.0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_code_round_trips() {
        for pal in [Palette::COOL_DARK, Palette::HIGH_CONTRAST, Palette::LIGHT] {
            let code = pal.to_code();
            assert!(code.starts_with(CODE_PREFIX));
            assert_eq!(Palette::from_code(&code), Some(pal));
        }
    }

    #[test]
    fn code_tolerates_surrounding_whitespace() {
        let code = Palette::HIGH_CONTRAST.to_code();
        let padded = format!("  {code}\n");
        assert_eq!(Palette::from_code(&padded), Some(Palette::HIGH_CONTRAST));
    }

    #[test]
    fn bad_codes_are_rejected() {
        assert_eq!(Palette::from_code(""), None);
        assert_eq!(Palette::from_code("hello"), None);
        assert_eq!(Palette::from_code("SSP1-"), None);
        assert_eq!(Palette::from_code("SSP1-!!!!"), None); // invalid base64 chars
                                                           // A corrupted-but-well-formed code fails the checksum.
        let mut code = Palette::COOL_DARK.to_code();
        let last = code.pop().unwrap();
        code.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(Palette::from_code(&code), None);
    }

    #[test]
    fn base64_round_trips_all_lengths() {
        for len in 0..50usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 1) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&data)), Some(data));
        }
    }
}
