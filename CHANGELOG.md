# Changelog

All notable changes to StorageSifter are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- **Customizable color palette** in Settings: pick a preset (Cool Dark, High
  Contrast, Light) or fine-tune any of the 16 interface/category colors. The
  light preset also switches the chrome to a light theme.
- **Shareable palette codes**: export the current palette to a short `SSP1-…`
  code (one-click copy) and import codes from others. Imported or saved palettes
  appear as your own chips alongside the presets, and custom ones can be removed
  with a hover **×**. Save the current colors as a named palette too.
- **Text / UI size** control for accessibility — scales the whole interface.
- **Animation speed** control (default a touch smoother at 0.375 s), and the
  ability to raise **folder preview depth** beyond 2 (with a performance caution).
- **Hover drill-target highlight**: the top-level cell a click would drill into
  is marked with a subtle accent-tinted stipple, so it's clear where you're
  about to zoom. Its **size and strength are adjustable** in Settings (or set
  the strength to 0 to turn it off).

### Changed

- New default color theme, and tuned out-of-the-box defaults: folder preview
  depth 2, UI size 115%, and a calmer hover highlight (size 10, strength 30%).

All of these persist to `settings.json` and apply live. The treemap snapshots
the active palette once per frame, so customization adds no per-cell cost.

## [0.1.0] — 2026-06-25

First public release.

### Added

- Squarified treemap of real on-disk usage, GPU-rendered on the wgpu (Vulkan)
  backend, with drill-down navigation and an animated cross-fade zoom.
- Background, cancellable scanning. Sizes use `st_blocks * 512` (byte-exact with
  `du`) and de-duplicate hard links. Stays on one filesystem by default, but
  crosses Btrfs subvolumes when scanning a whole device — with the boundaries
  outlined.
- Device picker, breadcrumb navigation, and folder names shown in cell headers.
- Cells colored by what they are: caches/build output, media, code, documents,
  archives, applications.
- Multi-select with move-to-trash (default, reversible) and permanent delete,
  behind tiered safety confirmation; protected system roots are refused.
- **Safe to delete?** reports that identify what a path is and recommend the
  proper cleanup command, including detection of your distro's package manager
  for system caches.
- Configurable keyboard shortcuts and behavior settings, persisted to
  `~/.config/storagesifter/settings.json`.
- Distribution: AppImage, prebuilt binary tarball, AUR packages
  (`storagesifter`, `storagesifter-bin`), and `cargo install`.

[0.1.0]: https://github.com/Fopull-LLC/StorageSifter/releases/tag/v0.1.0
