# Changelog

All notable changes to StorageSifter are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[Semantic Versioning](https://semver.org).

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
