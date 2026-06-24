//! User settings: configurable keybindings plus a couple of behavior toggles,
//! persisted to `$XDG_CONFIG_HOME/storagesifter/settings.json` (or
//! `~/.config/...`). Missing or unknown fields fall back to defaults, so the
//! config survives version changes.

use std::path::PathBuf;

use eframe::egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

/// A single key chord (a key plus modifier flags).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybind {
    /// egui key name, e.g. "Delete", "A", "F5", "Backspace".
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Keybind {
    fn new(key: &str, ctrl: bool, shift: bool, alt: bool) -> Self {
        Keybind {
            key: key.to_owned(),
            ctrl,
            shift,
            alt,
        }
    }

    /// Does this chord match a pressed-key event? (`command` mirrors `ctrl`.)
    pub fn matches(&self, key: Key, mods: Modifiers) -> bool {
        Key::from_name(&self.key) == Some(key)
            && (mods.ctrl || mods.command) == self.ctrl
            && mods.shift == self.shift
            && mods.alt == self.alt
    }

    /// Build a chord from a captured key event (for rebinding).
    pub fn from_event(key: Key, mods: Modifiers) -> Self {
        Keybind {
            key: key.name().to_owned(),
            ctrl: mods.ctrl || mods.command,
            shift: mods.shift,
            alt: mods.alt,
        }
    }

    /// Human-readable label like "Ctrl+Shift+Delete".
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        let key = Key::from_name(&self.key)
            .map(|k| k.symbol_or_name())
            .unwrap_or(self.key.as_str());
        let mut label = parts.join("+");
        if !label.is_empty() {
            label.push('+');
        }
        label.push_str(key);
        label
    }
}

/// A bindable action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    GoUp,
    ClearSelection,
    SelectAll,
    Trash,
    DeletePermanent,
    Rescan,
}

impl Action {
    /// All actions, in the order shown in the settings dialog.
    pub const ALL: [Action; 6] = [
        Action::GoUp,
        Action::ClearSelection,
        Action::SelectAll,
        Action::Trash,
        Action::DeletePermanent,
        Action::Rescan,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::GoUp => "Go up a level",
            Action::ClearSelection => "Clear selection",
            Action::SelectAll => "Select all in view",
            Action::Trash => "Move selection to Trash",
            Action::DeletePermanent => "Delete selection permanently",
            Action::Rescan => "Rescan",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keymap {
    pub go_up: Keybind,
    pub clear_selection: Keybind,
    pub select_all: Keybind,
    pub trash: Keybind,
    pub delete_permanent: Keybind,
    pub rescan: Keybind,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            go_up: Keybind::new("Backspace", false, false, false),
            clear_selection: Keybind::new("Escape", false, false, false),
            select_all: Keybind::new("A", true, false, false),
            trash: Keybind::new("Delete", false, false, false),
            delete_permanent: Keybind::new("Delete", false, true, false),
            rescan: Keybind::new("F5", false, false, false),
        }
    }
}

impl Keymap {
    pub fn get(&self, action: Action) -> &Keybind {
        match action {
            Action::GoUp => &self.go_up,
            Action::ClearSelection => &self.clear_selection,
            Action::SelectAll => &self.select_all,
            Action::Trash => &self.trash,
            Action::DeletePermanent => &self.delete_permanent,
            Action::Rescan => &self.rescan,
        }
    }

    pub fn set(&mut self, action: Action, bind: Keybind) {
        match action {
            Action::GoUp => self.go_up = bind,
            Action::ClearSelection => self.clear_selection = bind,
            Action::SelectAll => self.select_all = bind,
            Action::Trash => self.trash = bind,
            Action::DeletePermanent => self.delete_permanent = bind,
            Action::Rescan => self.rescan = bind,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub keys: Keymap,
    /// Animate zoom transitions when drilling in / out.
    pub animations: bool,
    /// Levels of nested preview drawn inside each folder (0–2).
    pub nesting_depth: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            keys: Keymap::default(),
            animations: true,
            nesting_depth: 1,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("storagesifter").join("settings.json"))
}

impl Settings {
    pub fn load() -> Settings {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
