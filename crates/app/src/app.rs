//! The application: a device picker, navigation, multi-select, file operations,
//! and the panel shell (toolbar / selection bar / treemap / status bar).

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Rect as ERect};
use scanner::safety::{classify, Class};
use scanner::{NodeId, NodeKind, Tree};

use crate::assess::{self, Assessment};
use crate::ops;
use crate::player::{self, AudioPlayer, MediaKind};
use crate::scan::Scan;
use crate::settings::{Action, Keybind, SavedPalette, Settings};
use crate::theme::{self, format_size};
use crate::treemap_view;

pub struct StorageSifterApp {
    disks: Vec<DiskInfo>,
    scan: Option<Scan>,
    path: PathBuf,
    current: NodeId,
    anim: Option<AnimState>,
    last_area: ERect,
    hovered: Option<NodeId>,
    /// Multi-selected nodes (Ctrl/Shift-click).
    selection: HashSet<NodeId>,
    /// Node a context menu is open for.
    menu: Option<NodeId>,
    /// The active modal dialog, if any.
    dialog: Dialog,
    /// The user's home directory, for safety classification.
    home: PathBuf,
    /// Last operation result, shown in the status bar.
    status: Option<String>,
    /// User settings (keybindings + behavior toggles).
    settings: Settings,
    /// Action awaiting a key to rebind it (settings dialog).
    capturing: Option<Action>,
    /// System package managers detected once at startup, for cleanup advice.
    pkgs: Vec<assess::PkgManager>,
    /// Cached stipple texture marking the hovered drill target (built lazily).
    dither: Option<egui::TextureHandle>,
    /// Transient settings-dialog buffers for saving / importing palettes.
    save_name: String,
    import_name: String,
    import_code: String,
    palette_msg: Option<String>,
    /// Maximized media view (when a media file is opened), if any.
    media: Option<MediaView>,
    /// Expected used bytes for the current scan target (a picked disk), enabling
    /// a percentage + ETA on the scanning screen. `None` for path-arg scans.
    scan_target: Option<u64>,
}

/// The maximized in-app media view for a clicked audio/video file.
struct MediaView {
    name: String,
    path: PathBuf,
    kind: MediaKind,
    /// The audio player (audio files only, and only if it opened successfully).
    player: Option<AudioPlayer>,
    /// Why audio playback couldn't start, if it failed.
    error: Option<String>,
    /// While dragging the timeline: the scrubbed position in seconds.
    scrub: Option<f32>,
}

impl MediaView {
    fn open(name: String, path: PathBuf, kind: MediaKind) -> MediaView {
        let (player, error) = match kind {
            MediaKind::Audio => match AudioPlayer::open(&path) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(e)),
            },
            MediaKind::Video => (None, None),
        };
        MediaView {
            name,
            path,
            kind,
            player,
            error,
            scrub: None,
        }
    }
}

struct DiskInfo {
    name: String,
    mount: PathBuf,
    fs: String,
    total: u64,
    available: u64,
}

#[derive(Clone, Copy)]
struct AnimState {
    parent: NodeId,
    child: NodeId,
    pivot: ERect,
    drilling_in: bool,
    start: Option<f64>,
}

enum Dialog {
    None,
    Properties(NodeId),
    Assess { id: NodeId, report: Assessment },
    Confirm { ids: Vec<NodeId>, permanent: bool },
    Settings,
}

enum MenuAction {
    Assess(NodeId),
    Properties(NodeId),
    Reveal(NodeId),
    Trash(Vec<NodeId>),
    Delete(Vec<NodeId>),
}

/// What the "Safe to delete?" report dialog returned this frame.
#[derive(Clone, Copy)]
enum AssessOutcome {
    Open,
    Close,
    Trash,
    Delete,
}

#[derive(Clone, Copy)]
enum SelAction {
    Clear,
    Trash,
    Delete,
}

/// Result of showing the confirm dialog this frame.
enum Confirm {
    Open,
    Cancel,
    Go,
}

impl StorageSifterApp {
    pub fn new(path: Option<PathBuf>) -> Self {
        let (scan, path) = match path {
            Some(p) => (Some(Scan::start(&p)), p),
            None => (None, PathBuf::new()),
        };
        let settings = Settings::load();
        theme::set_palette(settings.palette);
        Self {
            disks: list_disks(),
            scan,
            path,
            current: 0,
            anim: None,
            last_area: ERect::ZERO,
            hovered: None,
            selection: HashSet::new(),
            menu: None,
            dialog: Dialog::None,
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            status: None,
            settings,
            capturing: None,
            pkgs: assess::detect_package_managers(),
            dither: None,
            save_name: String::new(),
            import_name: String::new(),
            import_code: String::new(),
            palette_msg: None,
            media: None,
            scan_target: None,
        }
    }

    fn open(&mut self, path: PathBuf) {
        if let Some(old) = &self.scan {
            old.cancel(); // don't leave a superseded scan churning in the background
        }
        self.scan = Some(Scan::start_with_target(&path, self.scan_target));
        self.path = path;
        self.current = 0;
        self.anim = None;
        self.hovered = None;
        self.selection.clear();
        self.dialog = Dialog::None;
        self.status = None;
        self.media = None;
    }
}

impl eframe::App for StorageSifterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Accessibility: scale the whole UI. Only set it when it actually
        // changes, so we don't force a relayout every frame.
        if (ui.ctx().zoom_factor() - self.settings.ui_scale).abs() > 0.001 {
            ui.ctx().set_zoom_factor(self.settings.ui_scale);
        }

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

        // Rescan shortcut (configurable).
        if matches!(self.scan, Some(Scan::Done { .. }))
            && ui.ctx().input(|i| {
                i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                        if self.settings.keys.rescan.matches(*key, *modifiers))
            })
            })
        {
            self.open(self.path.clone());
        }

        self.toolbar(ui);
        if self.media.is_some() {
            // Keep the timeline live, and close on Escape.
            ui.ctx().request_repaint();
            if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                self.media = None;
            } else {
                self.media_view(ui);
            }
        } else {
            self.selection_bar(ui);
            self.status_bar(ui);
            self.treemap(ui);
        }
        self.dialogs(ui.ctx());
    }
}

impl StorageSifterApp {
    fn device_picker(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let mut refresh = false;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.heading(egui::RichText::new("StorageSifter").color(theme::text()));
                ui.label(
                    egui::RichText::new("Choose a filesystem to scan").color(theme::text_dim()),
                );
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
                            // Used bytes → the scan's expected total, for a %/ETA.
                            let used = disk.total.saturating_sub(disk.available);
                            chosen = Some((disk.mount.clone(), used));
                        }
                        ui.add_space(6.0);
                    }
                });
            });
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("A Fopull LLC project  ·").color(theme::text_dim()),
                    );
                    ui.hyperlink_to("fopull.com", "https://fopull.com");
                });
            });
        });

        if refresh {
            self.disks = list_disks();
        }
        if let Some((mount, used)) = chosen {
            self.scan_target = Some(used);
            self.open(mount);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("≡  Devices").clicked() {
                    if let Some(scan) = &self.scan {
                        scan.cancel();
                    }
                    self.scan = None;
                    self.media = None;
                    self.disks = list_disks();
                }
                if ui.button("⟳  Rescan").clicked() {
                    self.open(self.path.clone());
                }
                if ui.button("⚙  Settings").clicked() {
                    self.dialog = Dialog::Settings;
                }
                ui.separator();
                self.breadcrumb(ui);

                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match self.scan.as_ref() {
                        Some(Scan::Done { tree, elapsed }) => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}  ·  {} items  ·  {:.2}s",
                                    format_size(tree.node(tree.root).size),
                                    tree.len(),
                                    elapsed.as_secs_f64()
                                ))
                                .color(theme::text_dim()),
                            );
                        }
                        Some(Scan::Running { .. }) => {
                            ui.label(egui::RichText::new("scanning…").color(theme::accent()));
                        }
                        _ => {}
                    },
                );
            });
            ui.add_space(2.0);
        });
    }

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
                ui.label(egui::RichText::new("›").color(theme::text_dim()));
            }
            let label = segment_label(tree, id);
            if i == ids.len() - 1 {
                ui.label(egui::RichText::new(label).color(theme::text()).strong());
            } else if ui.link(label).clicked() {
                jump = Some(id);
            }
        }

        if let Some(target) = jump {
            let next = if self.settings.animations {
                zoom_out_pivot(tree, self.current, target, self.last_area)
            } else {
                None
            };
            self.anim = next.map(|(child, pivot)| AnimState {
                parent: target,
                child,
                pivot,
                drilling_in: false,
                start: None,
            });
            self.current = target;
            self.hovered = None;
        }
    }

    fn selection_bar(&mut self, ui: &mut egui::Ui) {
        if self.selection.is_empty() {
            return;
        }
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return;
        };
        let count = self.selection.len();
        // De-duplicate nested selections so a folder + a child inside it isn't
        // counted twice in the reclaimable total.
        let independent = independent_nodes(tree, self.selection.iter().copied().collect());
        let total: u64 = independent.iter().map(|&id| tree.node(id).size).sum();

        let mut action = None;
        egui::Panel::top("selection").show_inside(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{count} selected"))
                        .color(theme::accent())
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("·  {}", format_size(total)))
                        .color(theme::text_dim()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").clicked() {
                        action = Some(SelAction::Clear);
                    }
                    let del = egui::Button::new(
                        egui::RichText::new("Delete…").color(egui::Color32::WHITE),
                    )
                    .fill(theme::danger());
                    if ui.add(del).clicked() {
                        action = Some(SelAction::Delete);
                    }
                    if ui.button("Move to Trash…").clicked() {
                        action = Some(SelAction::Trash);
                    }
                });
            });
            ui.add_space(3.0);
        });

        match action {
            Some(SelAction::Clear) => self.selection.clear(),
            Some(SelAction::Trash) | Some(SelAction::Delete) => {
                let permanent = matches!(action, Some(SelAction::Delete));
                let ids: Vec<NodeId> = self.selection.iter().copied().collect();
                match prepare_delete(tree, &self.home, ids, permanent) {
                    Ok(dialog) => self.dialog = dialog,
                    Err(msg) => self.status = Some(msg),
                }
            }
            None => {}
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
                            .color(theme::text_dim()),
                    );
                    let cat = theme::Category::of(tree, id);
                    ui.label(
                        egui::RichText::new(format!("·  {}", cat.label()))
                            .color(cat.color(&theme::palette())),
                    );
                    if node.is_mountpoint() {
                        ui.label(egui::RichText::new("·  mount point").color(theme::mount()));
                    }
                    if node.is_hardlinked() {
                        ui.label(egui::RichText::new("·  hardlinked").color(theme::accent()));
                    }
                    if let Some(warn) = safety_note(classify(&tree.path(id), &self.home)) {
                        ui.label(egui::RichText::new(warn).color(theme::danger()));
                    }
                }
                (Some(Scan::Done { .. }), None) => {
                    let hint = self.status.clone().unwrap_or_else(|| {
                        "click to drill · Ctrl/Shift-click to select · right-click for actions"
                            .to_owned()
                    });
                    ui.label(egui::RichText::new(hint).color(theme::text_dim()));
                }
                _ => {
                    ui.label("");
                }
            });
            ui.add_space(2.0);
        });
    }

    /// The maximized media view: an audio transport (or, for video, a launch
    /// button), filling the window over the treemap.
    fn media_view(&mut self, ui: &mut egui::Ui) {
        let mut close = false;
        let mut open_ext = false;
        let mut restart = false;

        // Bottom transport bar.
        egui::Panel::bottom("media_controls").show_inside(ui, |ui| {
            ui.add_space(6.0);
            if let Some(media) = self.media.as_mut() {
                ui.horizontal(|ui| match (media.player.as_ref(), media.kind) {
                    (Some(player), _) => {
                        let done = player.finished();
                        let playing = !player.is_paused() && !done;
                        if transport_button(ui, playing) {
                            if done {
                                restart = true;
                            } else {
                                player.toggle_pause();
                            }
                        }
                        let total = player.duration().map(|d| d.as_secs_f32()).unwrap_or(0.0);
                        let live = player.position().as_secs_f32();
                        let shown = media.scrub.unwrap_or(live).clamp(0.0, total.max(0.001));
                        ui.monospace(fmt_time(shown));
                        let mut v = shown;
                        let resp = ui.add_sized(
                            egui::vec2(ui.available_width() - 56.0, 16.0),
                            egui::Slider::new(&mut v, 0.0..=total.max(0.001)).show_value(false),
                        );
                        if resp.dragged() {
                            media.scrub = Some(v);
                        }
                        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                            player.seek(std::time::Duration::from_secs_f32(v));
                            media.scrub = None;
                        }
                        ui.monospace(fmt_time(total));
                    }
                    (None, MediaKind::Video) => {
                        if ui.button("▶  Open in external player").clicked() {
                            open_ext = true;
                        }
                        ui.label(
                            egui::RichText::new("In-app video preview is coming soon.")
                                .color(theme::text_dim()),
                        );
                    }
                    (None, MediaKind::Audio) => {
                        ui.label(
                            egui::RichText::new(
                                media
                                    .error
                                    .as_deref()
                                    .unwrap_or("Couldn't play this audio."),
                            )
                            .color(theme::danger()),
                        );
                    }
                });
            }
            ui.add_space(6.0);
        });

        // Central content: a close button, then a centered glyph + filename.
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg()))
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui.button("×  Close").clicked() {
                        close = true;
                    }
                });
                if let Some(media) = self.media.as_ref() {
                    let glyph = match media.kind {
                        MediaKind::Audio => "♪",
                        MediaKind::Video => "▶",
                    };
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.32);
                        ui.label(
                            egui::RichText::new(glyph)
                                .size(72.0)
                                .color(theme::text_dim()),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(&media.name)
                                .size(20.0)
                                .strong()
                                .color(theme::text()),
                        );
                    });
                }
            });

        if open_ext {
            if let Some(m) = self.media.as_ref() {
                ops::open_external(&m.path);
            }
        }
        if restart {
            if let Some(media) = self.media.as_mut() {
                media.player = AudioPlayer::open(&media.path).ok();
                media.scrub = None;
            }
        }
        if close {
            self.media = None;
        }
    }

    fn treemap(&mut self, ui: &mut egui::Ui) {
        let dither = treemap_view::Dither {
            tex: self
                .dither
                .get_or_insert_with(|| treemap_view::make_dither_texture(ui.ctx()))
                .id(),
            scale: self.settings.dither_scale,
            strength: self.settings.dither_strength,
        };
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
                let mut leave = false;
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.28);
                    match self.scan.as_ref() {
                        Some(Scan::Running {
                            started,
                            progress,
                            target,
                            ..
                        }) => {
                            let elapsed = started.elapsed().as_secs_f64();
                            let entries = progress.entries();
                            let bytes = progress.bytes();
                            let frac = target.and_then(|t| {
                                (t > 0).then(|| (bytes as f32 / t as f32).clamp(0.0, 1.0))
                            });

                            ui.label(
                                egui::RichText::new("Scanning…")
                                    .size(22.0)
                                    .strong()
                                    .color(theme::text()),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(self.path.display().to_string())
                                    .monospace()
                                    .color(theme::text_dim()),
                            );
                            ui.add_space(16.0);

                            match frac {
                                Some(f) => {
                                    ui.add(
                                        egui::ProgressBar::new(f)
                                            .desired_width(440.0)
                                            .text(format!("{:.0}%", f * 100.0)),
                                    );
                                }
                                None => {
                                    ui.add(egui::Spinner::new().size(28.0));
                                }
                            }
                            ui.add_space(16.0);

                            egui::Grid::new("scan_stats")
                                .num_columns(2)
                                .spacing([18.0, 6.0])
                                .show(ui, |ui| {
                                    stat_row(ui, "Items found", group_thousands(entries));
                                    stat_row(ui, "Size found", format_size(bytes));
                                    if let Some(t) = target {
                                        stat_row(ui, "of about", format_size(*t));
                                    }
                                    stat_row(ui, "Elapsed", format!("{elapsed:.1} s"));
                                    if elapsed > 0.3 {
                                        let rate = (entries as f64 / elapsed) as u64;
                                        stat_row(
                                            ui,
                                            "Rate",
                                            format!("{}/s", group_thousands(rate)),
                                        );
                                    }
                                    if let Some(f) = frac {
                                        if (0.01..0.999).contains(&f) && elapsed > 0.5 {
                                            let eta = elapsed * (1.0 - f as f64) / f as f64;
                                            stat_row(ui, "Est. remaining", format!("~{eta:.0} s"));
                                        }
                                    }
                                });
                            ui.add_space(18.0);
                            if ui.button("Cancel").clicked() {
                                leave = true;
                            }
                        }
                        Some(Scan::Error(e)) => {
                            ui.label(
                                egui::RichText::new("Scan failed")
                                    .size(18.0)
                                    .strong()
                                    .color(theme::danger()),
                            );
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(e).color(theme::text_dim()));
                            ui.add_space(14.0);
                            if ui.button("Back to devices").clicked() {
                                leave = true;
                            }
                        }
                        _ => {}
                    }
                });
                if leave {
                    if let Some(scan) = &self.scan {
                        scan.cancel();
                    }
                    self.scan = None;
                }
                return;
            };

            let now = ui.ctx().input(|i| i.time);

            // Keyboard shortcuts (suppressed while a dialog is open).
            if matches!(self.dialog, Dialog::None) {
                let pressed: Vec<(egui::Key, egui::Modifiers)> = ui.ctx().input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Key {
                                key,
                                pressed: true,
                                repeat: false,
                                modifiers,
                                ..
                            } => Some((*key, *modifiers)),
                            _ => None,
                        })
                        .collect()
                });
                let hit = |bind: &Keybind| pressed.iter().any(|&(k, m)| bind.matches(k, m));
                let keys = &self.settings.keys;
                let (clear, go_up, select_all, trash, delete_perm) = (
                    hit(&keys.clear_selection),
                    hit(&keys.go_up),
                    hit(&keys.select_all),
                    hit(&keys.trash),
                    hit(&keys.delete_permanent),
                );

                // The clear binding clears a selection first, else it goes up.
                if clear && !self.selection.is_empty() {
                    self.selection.clear();
                } else if (go_up || clear) && self.anim.is_none() {
                    if let Some(parent) = tree.node(self.current).parent {
                        let pivot = if self.settings.animations {
                            zoom_out_pivot(tree, self.current, parent, self.last_area)
                        } else {
                            None
                        };
                        self.anim = pivot.map(|(child, pivot)| AnimState {
                            parent,
                            child,
                            pivot,
                            drilling_in: false,
                            start: None,
                        });
                        self.current = parent;
                    }
                }

                if select_all {
                    self.selection
                        .extend(tree.node(self.current).children.iter().copied());
                }
                if (trash || delete_perm) && !self.selection.is_empty() {
                    let ids: Vec<NodeId> = self.selection.iter().copied().collect();
                    match prepare_delete(tree, &self.home, ids, delete_perm) {
                        Ok(dialog) => self.dialog = dialog,
                        Err(msg) => self.status = Some(msg),
                    }
                }
            }

            let anim = if let Some(a) = &mut self.anim {
                let start = *a.start.get_or_insert(now);
                let t = (now - start) / self.settings.anim_secs.max(0.01) as f64;
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

            let it = treemap_view::show(
                ui,
                tree,
                self.current,
                anim,
                &self.selection,
                self.settings.nesting_depth,
                dither,
            );
            self.last_area = it.area;
            self.hovered = it.hovered.map(|h| h.id);

            if let Some(hit) = it.clicked {
                if it.modified {
                    let target = it.hovered.map(|h| h.id).unwrap_or(hit.id);
                    if !self.selection.insert(target) {
                        self.selection.remove(&target);
                    }
                    self.status = None;
                } else {
                    let node = tree.node(hit.id);
                    if node.kind == NodeKind::Dir && !node.children.is_empty() {
                        if self.settings.animations {
                            self.anim = Some(AnimState {
                                parent: self.current,
                                child: hit.id,
                                pivot: hit.rect,
                                drilling_in: true,
                                start: None,
                            });
                        }
                        self.current = hit.id;
                    } else if node.kind != NodeKind::Dir {
                        // A media file: maximize into the in-app media view.
                        if let Some(kind) = player::media_kind(tree.name(hit.id)) {
                            let name = tree.name(hit.id).to_owned();
                            let path = tree.path(hit.id);
                            self.media = Some(MediaView::open(name, path, kind));
                            self.selection.clear();
                        }
                    }
                }
            }
            if let Some(hit) = it.secondary {
                self.menu = Some(hit.id);
            }

            let mut menu_action = None;
            it.response.context_menu(|ui| {
                if let Some(target) = self.menu {
                    menu_action = context_menu_items(ui, tree, target, &self.selection);
                }
            });
            match menu_action {
                Some(MenuAction::Assess(id)) => {
                    // Computed once, here — the report is then cached in the
                    // dialog and never recomputed per frame.
                    let report = assess::assess(tree, id, &self.home, &self.pkgs);
                    self.dialog = Dialog::Assess { id, report };
                }
                Some(MenuAction::Properties(id)) => self.dialog = Dialog::Properties(id),
                Some(MenuAction::Reveal(id)) => ops::reveal(&tree.path(id)),
                Some(MenuAction::Trash(ids)) => {
                    match prepare_delete(tree, &self.home, ids, false) {
                        Ok(dialog) => self.dialog = dialog,
                        Err(msg) => self.status = Some(msg),
                    }
                }
                Some(MenuAction::Delete(ids)) => {
                    match prepare_delete(tree, &self.home, ids, true) {
                        Ok(dialog) => self.dialog = dialog,
                        Err(msg) => self.status = Some(msg),
                    }
                }
                None => {}
            }

            if self.anim.is_some() {
                ui.ctx().request_repaint();
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        match dialog {
            Dialog::None => {}
            Dialog::Properties(id) => {
                if self.show_properties(ctx, id) {
                    self.dialog = Dialog::Properties(id);
                }
            }
            Dialog::Assess { id, report } => {
                let outcome = self.show_assess(ctx, id, &report);
                match outcome {
                    AssessOutcome::Open => self.dialog = Dialog::Assess { id, report },
                    AssessOutcome::Close => {}
                    AssessOutcome::Trash | AssessOutcome::Delete => {
                        let permanent = matches!(outcome, AssessOutcome::Delete);
                        if let Some(Scan::Done { tree, .. }) = self.scan.as_ref() {
                            match prepare_delete(tree, &self.home, vec![id], permanent) {
                                Ok(dialog) => self.dialog = dialog,
                                Err(msg) => self.status = Some(msg),
                            }
                        }
                    }
                }
            }
            Dialog::Confirm { ids, permanent } => match self.show_confirm(ctx, &ids, permanent) {
                Confirm::Open => self.dialog = Dialog::Confirm { ids, permanent },
                Confirm::Cancel => {}
                Confirm::Go => self.execute_delete(ids, permanent),
            },
            Dialog::Settings => {
                if self.show_settings(ctx) {
                    self.dialog = Dialog::Settings;
                } else {
                    self.settings.save();
                    self.capturing = None;
                }
            }
        }
    }

    fn show_properties(&self, ctx: &egui::Context, id: NodeId) -> bool {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return false;
        };
        let node = tree.node(id);
        let path = tree.path(id);
        let mut keep = true;
        let response = egui::Modal::new(egui::Id::new("properties")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading("Properties");
            ui.add_space(6.0);
            egui::Grid::new("props")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    prop_row(ui, "Path", path.display().to_string());
                    prop_row(ui, "On disk", format_size(node.size));
                    prop_row(
                        ui,
                        "Category",
                        theme::Category::of(tree, id).label().to_owned(),
                    );
                    prop_row(ui, "Kind", kind_label(node.kind).to_owned());
                    if node.kind == NodeKind::Dir {
                        prop_row(ui, "Items", node.children.len().to_string());
                    }
                    if node.is_hardlinked() {
                        prop_row(ui, "Hard links", node.nlink.to_string());
                    }
                    if node.is_mountpoint() {
                        prop_row(ui, "Mount", "subvolume / mount point".to_owned());
                    }
                    prop_row(
                        ui,
                        "Safety",
                        class_label(classify(&path, &self.home)).to_owned(),
                    );
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Reveal in file manager").clicked() {
                    ops::reveal(&path);
                }
                if ui.button("Close").clicked() {
                    keep = false;
                }
            });
        });
        if response.should_close() {
            keep = false;
        }
        keep
    }

    /// The "Safe to delete?" report. `report` is precomputed and merely
    /// rendered here, so this stays cheap on every frame the modal is open.
    fn show_assess(&self, ctx: &egui::Context, id: NodeId, report: &Assessment) -> AssessOutcome {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return AssessOutcome::Close;
        };
        let node = tree.node(id);
        let path = tree.path(id);
        let meta = if node.kind == NodeKind::Dir {
            format!(
                "{}  ·  {} items",
                format_size(node.size),
                node.children.len()
            )
        } else {
            format!("{}  ·  file", format_size(node.size))
        };

        let mut outcome = AssessOutcome::Open;
        let response = egui::Modal::new(egui::Id::new("assess")).show(ctx, |ui| {
            ui.set_width(540.0);

            // Verdict badge + what-it-is headline.
            ui.heading(egui::RichText::new(report.verdict.label()).color(report.verdict.color()));
            ui.label(
                egui::RichText::new(&report.headline)
                    .strong()
                    .color(theme::text()),
            );
            ui.add_space(6.0);
            ui.monospace(path.display().to_string());
            ui.label(egui::RichText::new(meta).color(theme::text_dim()));

            ui.add_space(10.0);
            ui.label(&report.detail);

            // Recommended cleanup command, when a tool offers a better way.
            if let Some(cmd) = &report.command {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("Recommended way to clear it")
                        .color(theme::mount())
                        .strong(),
                );
                ui.add_space(2.0);
                egui::Frame::new()
                    .fill(theme::bg())
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .corner_radius(4)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(cmd).monospace().color(theme::text()),
                                )
                                .wrap(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Copy").clicked() {
                                        ui.ctx().copy_text(cmd.clone());
                                    }
                                },
                            );
                        });
                    });
                ui.label(
                    egui::RichText::new(
                        "Run this in a terminal — StorageSifter won't run it for you.",
                    )
                    .color(theme::text_dim())
                    .small(),
                );
            }

            if !report.points.is_empty() {
                ui.add_space(10.0);
                for p in &report.points {
                    let (mark, color) = if p.good {
                        ("✔", assess::Verdict::Safe.color())
                    } else {
                        ("⚠", assess::Verdict::Caution.color())
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(egui::RichText::new(mark).color(color));
                        ui.label(egui::RichText::new(&p.text).color(theme::text()));
                    });
                }
            }

            ui.add_space(14.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Move to Trash").clicked() {
                    outcome = AssessOutcome::Trash;
                }
                let del = egui::Button::new(
                    egui::RichText::new("Delete permanently…").color(egui::Color32::WHITE),
                )
                .fill(theme::danger());
                if ui.add(del).clicked() {
                    outcome = AssessOutcome::Delete;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        outcome = AssessOutcome::Close;
                    }
                });
            });
        });
        if response.should_close() && matches!(outcome, AssessOutcome::Open) {
            outcome = AssessOutcome::Close;
        }
        outcome
    }

    fn show_confirm(&self, ctx: &egui::Context, ids: &[NodeId], permanent: bool) -> Confirm {
        let Some(Scan::Done { tree, .. }) = self.scan.as_ref() else {
            return Confirm::Cancel;
        };
        let total: u64 = ids.iter().map(|&id| tree.node(id).size).sum();
        let outside = ids
            .iter()
            .filter(|&&id| safety_note(classify(&tree.path(id), &self.home)).is_some())
            .count();

        let mut result = Confirm::Open;
        let response = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(520.0);
            let title = if permanent {
                "Delete permanently"
            } else {
                "Move to Trash"
            };
            ui.heading(egui::RichText::new(title).color(if permanent {
                theme::danger()
            } else {
                theme::text()
            }));
            ui.add_space(6.0);
            ui.label(format!(
                "{} item(s)  ·  {} total",
                ids.len(),
                format_size(total)
            ));
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(170.0)
                .show(ui, |ui| {
                    for &id in ids.iter().take(15) {
                        ui.monospace(tree.path(id).display().to_string());
                    }
                    if ids.len() > 15 {
                        ui.label(format!("… and {} more", ids.len() - 15));
                    }
                });
            ui.add_space(8.0);
            if permanent {
                ui.label(
                    egui::RichText::new("This cannot be undone.")
                        .color(theme::danger())
                        .strong(),
                );
            } else {
                ui.label(
                    egui::RichText::new("Items can be restored from the trash.")
                        .color(theme::text_dim()),
                );
            }
            if outside > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠  {outside} item(s) are outside your home directory."
                    ))
                    .color(theme::danger()),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = Confirm::Cancel;
                }
                let go = if permanent {
                    "Delete permanently"
                } else {
                    "Move to Trash"
                };
                let button = egui::Button::new(egui::RichText::new(go).color(egui::Color32::WHITE))
                    .fill(if permanent {
                        theme::danger()
                    } else {
                        theme::Category::App.color(&theme::palette())
                    });
                if ui.add(button).clicked() {
                    result = Confirm::Go;
                }
            });
        });
        if response.should_close() && matches!(result, Confirm::Open) {
            result = Confirm::Cancel;
        }
        result
    }

    fn execute_delete(&mut self, ids: Vec<NodeId>, permanent: bool) {
        let Some(Scan::Done { tree, .. }) = &mut self.scan else {
            return;
        };
        let targets: Vec<(PathBuf, u64)> = ids
            .iter()
            .map(|&id| (tree.path(id), tree.node(id).size))
            .collect();
        let mode = if permanent {
            ops::Mode::Delete
        } else {
            ops::Mode::Trash
        };
        let report = ops::perform(&targets, mode);
        let mut removed: HashSet<NodeId> = HashSet::new();
        for (&id, (path, _)) in ids.iter().zip(&targets) {
            if report.succeeded.contains(path) {
                tree.remove_subtree(id);
                removed.insert(id);
            }
        }

        // If we just deleted the folder being viewed (or an ancestor of it),
        // retreat to the nearest surviving ancestor — otherwise the treemap would
        // keep rendering a subtree that no longer exists on disk.
        let mut node = self.current;
        let mut landing = None;
        loop {
            if removed.contains(&node) {
                landing = tree.node(node).parent;
            }
            match tree.node(node).parent {
                Some(parent) => node = parent,
                None => break,
            }
        }
        if let Some(target) = landing {
            self.current = target;
            self.anim = None;
            self.hovered = None;
        }

        self.status = Some(report.summary());
        self.selection.clear();
    }

    /// The settings modal: rebindable keys plus behavior toggles. Returns
    /// whether it should stay open.
    fn show_settings(&mut self, ctx: &egui::Context) -> bool {
        // If we're listening for a rebind, capture the next key chord.
        let mut just_captured = false;
        if let Some(action) = self.capturing {
            let captured = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(Keybind::from_event(*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some(bind) = captured {
                self.settings.keys.set(action, bind);
                self.capturing = None;
                just_captured = true;
            }
        }

        let mut keep = true;
        // Set true by any color/preset change so we re-install the palette once,
        // after the dialog is built.
        let mut theme_dirty = false;
        let response = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
            ui.set_width(500.0);
            ui.heading("Settings");
            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .max_height(600.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    // ---- Keybindings ----
                    section_label(ui, "Keybindings");
                    ui.label(
                        egui::RichText::new("Click a binding, then press the keys.")
                            .small()
                            .color(theme::text_dim()),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("binds")
                        .num_columns(2)
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            for action in Action::ALL {
                                ui.label(action.label());
                                let listening = self.capturing == Some(action);
                                let text = if listening {
                                    "press keys…".to_owned()
                                } else {
                                    self.settings.keys.get(action).label()
                                };
                                let mut button =
                                    egui::Button::new(egui::RichText::new(text).monospace())
                                        .min_size(egui::vec2(160.0, 0.0));
                                if listening {
                                    button = button.fill(theme::accent().gamma_multiply(0.4));
                                }
                                if ui.add(button).clicked() {
                                    self.capturing = if listening { None } else { Some(action) };
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(12.0);

                    // ---- Behavior ----
                    section_label(ui, "Behavior");
                    ui.checkbox(&mut self.settings.animations, "Animate zoom transitions");
                    ui.add_enabled_ui(self.settings.animations, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Animation speed");
                            ui.add(
                                egui::Slider::new(
                                    &mut self.settings.anim_secs,
                                    crate::settings::ANIM_SECS_RANGE,
                                )
                                .suffix(" s"),
                            );
                            ui.label(
                                egui::RichText::new("lower = snappier")
                                    .small()
                                    .color(theme::text_dim()),
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Folder preview depth");
                        ui.add(egui::Slider::new(
                            &mut self.settings.nesting_depth,
                            0..=crate::settings::MAX_NESTING_DEPTH,
                        ));
                    });
                    if self.settings.nesting_depth > crate::settings::NESTING_ADVISED_MAX {
                        ui.label(
                            egui::RichText::new(
                                "⚠  Deep previews draw many more cells — may slow rendering on very large trees.",
                            )
                            .small()
                            .color(theme::danger()),
                        );
                    }
                    ui.horizontal(|ui| {
                        ui.label("Hover highlight size");
                        ui.add(egui::Slider::new(
                            &mut self.settings.dither_scale,
                            crate::settings::DITHER_SCALE_RANGE,
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Hover highlight strength");
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings.dither_strength,
                                crate::settings::DITHER_STRENGTH_RANGE,
                            )
                            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                        );
                    });
                    ui.add_space(12.0);

                    // ---- Accessibility ----
                    section_label(ui, "Accessibility");
                    ui.horizontal(|ui| {
                        ui.label("Text / UI size");
                        // A +/- stepper, not a slider: changing the UI scale moves
                        // a slider out from under the cursor (a feedback loop that
                        // runs straight to the end). Discrete 5% steps avoid that,
                        // and the buttons barely move per click so you can keep
                        // clicking the same one.
                        let range = crate::settings::UI_SCALE_RANGE;
                        let step = 0.05_f32;
                        let snapped = (self.settings.ui_scale / step).round() * step;
                        let bsize = egui::vec2(30.0, 0.0);
                        if ui
                            .add(egui::Button::new(egui::RichText::new("-").monospace()).min_size(bsize))
                            .clicked()
                        {
                            self.settings.ui_scale =
                                (snapped - step).clamp(*range.start(), *range.end());
                        }
                        ui.add_sized(
                            egui::vec2(46.0, 20.0),
                            egui::Label::new(format!("{:.0}%", self.settings.ui_scale * 100.0)),
                        );
                        if ui
                            .add(egui::Button::new(egui::RichText::new("+").monospace()).min_size(bsize))
                            .clicked()
                        {
                            self.settings.ui_scale =
                                (snapped + step).clamp(*range.start(), *range.end());
                        }
                    });
                    ui.add_space(12.0);

                    // ---- Appearance ----
                    section_label(ui, "Appearance");
                    let mut apply_palette: Option<theme::Palette> = None;
                    let mut delete_saved: Option<usize> = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Theme:");
                        for (name, pal) in theme::Palette::PRESETS {
                            if ui.button(name).clicked() {
                                apply_palette = Some(pal);
                            }
                        }
                        // Saved (custom) palettes — clickable, with a hover × to delete.
                        for (i, sp) in self.settings.saved_palettes.iter().enumerate() {
                            let (apply, delete) = palette_chip(ui, &sp.name);
                            if apply {
                                apply_palette = Some(sp.palette);
                            }
                            if delete {
                                delete_saved = Some(i);
                            }
                        }
                    });
                    if let Some(i) = delete_saved {
                        self.settings.saved_palettes.remove(i);
                    }
                    if let Some(pal) = apply_palette {
                        self.settings.palette = pal;
                        theme_dirty = true;
                    }
                    ui.collapsing("Customize colors", |ui| {
                        let p = &mut self.settings.palette;
                        ui.label(
                            egui::RichText::new("Interface")
                                .small()
                                .color(theme::text_dim()),
                        );
                        egui::Grid::new("colors_chrome")
                            .num_columns(2)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                theme_dirty |= color_swatch(ui, "Background", &mut p.bg);
                                theme_dirty |= color_swatch(ui, "Panels", &mut p.panel);
                                theme_dirty |= color_swatch(ui, "Text", &mut p.text);
                                theme_dirty |= color_swatch(ui, "Dim text", &mut p.text_dim);
                                theme_dirty |= color_swatch(ui, "Cell borders", &mut p.border);
                                theme_dirty |= color_swatch(ui, "Selection accent", &mut p.accent);
                                theme_dirty |= color_swatch(ui, "Mount edge", &mut p.mount);
                                theme_dirty |= color_swatch(ui, "Danger", &mut p.danger);
                            });
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("File categories")
                                .small()
                                .color(theme::text_dim()),
                        );
                        egui::Grid::new("colors_cats")
                            .num_columns(2)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                theme_dirty |= color_swatch(ui, "Cache / junk", &mut p.junk);
                                theme_dirty |= color_swatch(ui, "Media", &mut p.media);
                                theme_dirty |= color_swatch(ui, "Archive", &mut p.archive);
                                theme_dirty |= color_swatch(ui, "Application", &mut p.app);
                                theme_dirty |= color_swatch(ui, "Code", &mut p.code);
                                theme_dirty |= color_swatch(ui, "Document", &mut p.document);
                                theme_dirty |= color_swatch(ui, "Folder", &mut p.folder);
                                theme_dirty |= color_swatch(ui, "Other", &mut p.other);
                            });
                    });

                    ui.add_space(6.0);
                    ui.collapsing("Share & manage palettes", |ui| {
                        // Export: the current palette's shareable code.
                        let code = self.settings.palette.to_code();
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("This palette's code")
                                    .small()
                                    .color(theme::text_dim()),
                            );
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(code.clone());
                                self.palette_msg = Some("Code copied to clipboard.".to_owned());
                            }
                        });
                        ui.add(
                            egui::Label::new(egui::RichText::new(&code).monospace().small()).wrap(),
                        );

                        ui.add_space(8.0);
                        // Save the current colors as a named palette.
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.save_name)
                                    .hint_text("name")
                                    .desired_width(130.0),
                            );
                            if ui.button("Save current").clicked() {
                                let pal = self.settings.palette;
                                let name =
                                    palette_name(&self.save_name, self.settings.saved_palettes.len());
                                self.settings
                                    .saved_palettes
                                    .push(SavedPalette { name, palette: pal });
                                self.save_name.clear();
                                self.palette_msg = Some("Saved to your palettes.".to_owned());
                            }
                        });

                        ui.add_space(4.0);
                        // Import a palette from a shared code.
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.import_name)
                                    .hint_text("name")
                                    .desired_width(130.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.import_code)
                                    .hint_text("paste a code")
                                    .desired_width(190.0),
                            );
                            if ui.button("Add").clicked() {
                                match theme::Palette::from_code(&self.import_code) {
                                    Some(pal) => {
                                        let name = palette_name(
                                            &self.import_name,
                                            self.settings.saved_palettes.len(),
                                        );
                                        self.settings
                                            .saved_palettes
                                            .push(SavedPalette { name, palette: pal });
                                        self.settings.palette = pal;
                                        theme_dirty = true;
                                        self.import_code.clear();
                                        self.import_name.clear();
                                        self.palette_msg =
                                            Some("Palette added and applied.".to_owned());
                                    }
                                    None => {
                                        self.palette_msg =
                                            Some("That code isn't valid.".to_owned());
                                    }
                                }
                            }
                        });
                        if let Some(msg) = &self.palette_msg {
                            ui.label(egui::RichText::new(msg).small().color(theme::text_dim()));
                        }
                    });
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Reset to defaults").clicked() {
                    // Keep the user's saved palettes — a settings reset shouldn't
                    // throw away color profiles they imported or saved.
                    let saved = std::mem::take(&mut self.settings.saved_palettes);
                    self.settings = Settings::default();
                    self.settings.saved_palettes = saved;
                    self.capturing = None;
                    theme_dirty = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        keep = false;
                    }
                });
            });
        });

        // Re-install the palette once if any color changed this frame.
        if theme_dirty {
            theme::set_palette(self.settings.palette);
            theme::apply(ctx);
        }
        // A captured Esc is a rebind, not a request to close the dialog.
        if response.should_close() && !just_captured {
            keep = false;
        }
        keep
    }
}

/// A play/pause toggle drawn with the painter (no font-glyph dependency, so the
/// icon is always crisp). `playing` shows the pause bars; otherwise a play
/// triangle. Returns whether it was clicked.
fn transport_button(ui: &mut egui::Ui, playing: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 24.0), egui::Sense::click());
    let vis = ui.style().interact(&resp);
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, vis.weak_bg_fill);
    let c = rect.center();
    let col = vis.fg_stroke.color;
    if playing {
        let (bw, gap, h) = (3.0_f32, 4.0_f32, 11.0_f32);
        let off = gap / 2.0 + bw / 2.0;
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(c.x - off, c.y), egui::vec2(bw, h)),
            0.0,
            col,
        );
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(c.x + off, c.y), egui::vec2(bw, h)),
            0.0,
            col,
        );
    } else {
        let s = 6.0_f32;
        let tri = vec![
            egui::pos2(c.x - s * 0.5, c.y - s),
            egui::pos2(c.x - s * 0.5, c.y + s),
            egui::pos2(c.x + s * 0.9, c.y),
        ];
        painter.add(egui::Shape::convex_polygon(tri, col, egui::Stroke::NONE));
    }
    resp.clicked()
}

/// A two-column "label: value" row in the scanning-screen stats grid.
fn stat_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(egui::RichText::new(label).color(theme::text_dim()));
    ui.label(egui::RichText::new(value).strong().color(theme::text()));
    ui.end_row();
}

/// Format an integer with thousands separators, e.g. `1234567` → `1,234,567`.
fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Format a duration in seconds as `M:SS` (or `H:MM:SS` past an hour).
fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// An accent-colored section heading in the settings dialog.
fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::accent()).strong());
    ui.add_space(4.0);
}

/// Name a saved palette: the trimmed input, or a generated "Custom N".
fn palette_name(input: &str, existing: usize) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        format!("Custom {}", existing + 1)
    } else {
        trimmed.to_owned()
    }
}

/// A saved-palette chip: click it to apply, or click the × (revealed on hover,
/// at the top-right corner) to delete it. Returns `(apply, delete)`.
fn palette_chip(ui: &mut egui::Ui, name: &str) -> (bool, bool) {
    let b = ui.button(name);
    let mut apply = b.clicked();
    let mut delete = false;
    if b.hovered() {
        // The × sits inside the chip's top-right corner, so moving onto it keeps
        // the chip hovered (no flicker). `interact` doesn't disturb layout.
        let sz = 13.0;
        let x_rect = egui::Rect::from_min_size(
            egui::pos2(b.rect.right() - sz, b.rect.top()),
            egui::vec2(sz, sz),
        );
        let x = ui.interact(x_rect, b.id.with("del"), egui::Sense::click());
        let bg = if x.hovered() {
            theme::danger()
        } else {
            theme::danger().gamma_multiply(0.85)
        };
        ui.painter().rect_filled(x_rect, 3.0, bg);
        ui.painter().text(
            x_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
        if x.clicked() {
            delete = true;
            apply = false;
        }
    }
    (apply, delete)
}

/// A two-column color row: an opaque-RGB swatch picker plus its label. Returns
/// whether the color changed this frame.
fn color_swatch(ui: &mut egui::Ui, label: &str, c: &mut egui::Color32) -> bool {
    let mut rgb = [c.r(), c.g(), c.b()];
    let changed = ui.color_edit_button_srgb(&mut rgb).changed();
    if changed {
        *c = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    }
    ui.label(label);
    ui.end_row();
    changed
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
        .fill(theme::panel())
        .show(ui, |ui| {
            ui.set_width(520.0);
            ui.label(
                egui::RichText::new(disk.mount.display().to_string())
                    .monospace()
                    .strong()
                    .color(theme::text()),
            );
            ui.label(
                egui::RichText::new(format!("{}  ·  {}", disk.name, disk.fs))
                    .small()
                    .color(theme::text_dim()),
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

/// Build the right-click context menu; returns the action chosen, if any.
fn context_menu_items(
    ui: &mut egui::Ui,
    tree: &Tree,
    target: NodeId,
    selection: &HashSet<NodeId>,
) -> Option<MenuAction> {
    // Operate on the whole selection if the clicked cell is part of it.
    let ids: Vec<NodeId> = if selection.contains(&target) {
        selection.iter().copied().collect()
    } else {
        vec![target]
    };
    let n = ids.len();

    let mut action = None;
    ui.label(egui::RichText::new(tree.name(target)).strong());
    ui.separator();
    if ui
        .button(egui::RichText::new("Safe to delete?").color(theme::mount()))
        .clicked()
    {
        action = Some(MenuAction::Assess(target));
        ui.close();
    }
    if ui.button("Properties").clicked() {
        action = Some(MenuAction::Properties(target));
        ui.close();
    }
    if ui.button("Reveal in file manager").clicked() {
        action = Some(MenuAction::Reveal(target));
        ui.close();
    }
    ui.separator();
    let trash = if n > 1 {
        format!("Move {n} items to Trash")
    } else {
        "Move to Trash".to_owned()
    };
    if ui.button(trash).clicked() {
        action = Some(MenuAction::Trash(ids.clone()));
        ui.close();
    }
    let delete = if n > 1 {
        format!("Delete {n} items permanently…")
    } else {
        "Delete permanently…".to_owned()
    };
    if ui
        .button(egui::RichText::new(delete).color(theme::danger()))
        .clicked()
    {
        action = Some(MenuAction::Delete(ids));
        ui.close();
    }
    action
}

/// Filter `ids` down to those with no selected ancestor (so deleting a folder
/// and something inside it doesn't double-count or fail).
fn independent_nodes(tree: &Tree, ids: Vec<NodeId>) -> Vec<NodeId> {
    let set: HashSet<NodeId> = ids.iter().copied().collect();
    ids.into_iter()
        .filter(|&id| {
            let mut ancestor = tree.node(id).parent;
            while let Some(a) = ancestor {
                if set.contains(&a) {
                    return false;
                }
                ancestor = tree.node(a).parent;
            }
            true
        })
        .collect()
}

/// Prepare a delete: dedup nested selections and refuse critical paths.
fn prepare_delete(
    tree: &Tree,
    home: &std::path::Path,
    ids: Vec<NodeId>,
    permanent: bool,
) -> Result<Dialog, String> {
    let ids = independent_nodes(tree, ids);
    if ids.is_empty() {
        return Err("Nothing to delete".to_owned());
    }
    for &id in &ids {
        let path = tree.path(id);
        if classify(&path, home) == Class::Critical {
            return Err(format!(
                "Refused — {} is a protected system location",
                path.display()
            ));
        }
    }
    Ok(Dialog::Confirm { ids, permanent })
}

fn prop_row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.label(egui::RichText::new(key).color(theme::text_dim()));
    ui.monospace(value);
    ui.end_row();
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Dir => "Folder",
        NodeKind::File => "File",
        NodeKind::Symlink => "Symlink",
        NodeKind::Other => "Special file",
    }
}

fn class_label(class: Class) -> &'static str {
    match class {
        Class::Normal => "in home directory",
        Class::OutsideHome => "outside home directory",
        Class::System => "system location",
        Class::Critical => "protected (cannot delete)",
    }
}

/// A short status-bar warning for a non-normal safety class.
fn safety_note(class: Class) -> Option<&'static str> {
    match class {
        Class::Normal => None,
        Class::OutsideHome => Some("·  ⚠ outside home"),
        Class::System => Some("·  ⚠ system"),
        Class::Critical => Some("·  ⛔ protected"),
    }
}

/// Enumerate mounted filesystems with real capacity, one row per device.
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
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.mount.as_os_str().len().cmp(&b.mount.as_os_str().len()))
    });
    out.dedup_by(|a, b| a.name == b.name);
    out.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.mount.cmp(&b.mount)));
    out
}

/// The child of `target` on the path up from `from`, plus its screen rect.
fn zoom_out_pivot(
    tree: &Tree,
    from: NodeId,
    target: NodeId,
    area: ERect,
) -> Option<(NodeId, ERect)> {
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
