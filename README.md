# StorageSifter

A fast, lightweight disk-usage visualizer for Linux. It maps a filesystem as a
**squarified treemap** — nested rectangles sized proportionally to what they
consume — so the biggest space hogs are obvious at a glance. Think SpaceSniffer,
but native, GPU-accelerated, and built for a single Linux machine.

A free and open-source project by **[Fopull LLC](https://fopull.com)**.

> **v0.1.0 — first public release.** A complete, daily-usable disk visualizer:
> scanning engine, squarified treemap (drill-down, breadcrumb, animated zoom), a
> device picker, file operations (multi-select, move to trash, permanent delete)
> behind safety guardrails, and "Safe to delete?" cleanup reports.

## Install

StorageSifter is a single, self-contained binary for **x86-64 Linux**. It needs
a Vulkan-capable GPU driver (standard on modern desktops) and a Wayland or X11
session.

### AppImage — any distro

Download `StorageSifter-*-x86_64.AppImage` from the
[latest release](https://github.com/fopull/StorageSifter/releases/latest), then:

```sh
chmod +x StorageSifter-*-x86_64.AppImage
./StorageSifter-*-x86_64.AppImage
```

### Arch / CachyOS — AUR

```sh
yay -S storagesifter-bin     # prebuilt binary, fast
# …or build from source:
yay -S storagesifter
```

### Prebuilt binary — tarball

Grab `storagesifter-*-x86_64-linux.tar.gz` from the
[latest release](https://github.com/fopull/StorageSifter/releases/latest),
then put the binary on your `PATH`:

```sh
tar xzf storagesifter-*-x86_64-linux.tar.gz
install -Dm755 storagesifter-*/storagesifter ~/.local/bin/storagesifter
```

(The bundled `INSTALL.txt` covers optional menu-entry and icon integration.)

### From source — cargo

```sh
cargo install --git https://github.com/fopull/StorageSifter --locked storagesifter
```

You can verify any download against the release's `SHA256SUMS.txt`:
`sha256sum -c SHA256SUMS.txt`.

## Architecture

A Cargo workspace with the engine and layout kept strictly UI-free, so each part
is testable and reusable on its own:

| Crate | Role |
|-------|------|
| [`crates/scanner`](crates/scanner) | Parallel, single-filesystem disk scanner. Arena tree, hard-link de-dup, safety classification. No UI dependencies. |
| [`crates/treemap`](crates/treemap) | Squarified treemap layout (Phase 2). Pure geometry: rectangles in, rectangles out. |
| [`crates/app`](crates/app)         | The desktop UI — `eframe`/`egui` on the `wgpu` (Vulkan) backend. Glue only. |

The scanner produces an arena of nodes (`Vec<Node>`, addressed by `u32` ids).
Every node stores its **parent** as well as its children, so deleting an item can
walk up the ancestor chain and subtract reclaimed bytes in `O(depth)` without
rebuilding the tree.

## How "size" is measured

StorageSifter reports **real on-disk consumption**, computed as `st_blocks * 512`
— the number of 512-byte blocks the filesystem has actually allocated to a file.
This is deliberate: it is the number that answers *"how much will I actually
reclaim if I delete this?"* It accounts for sparse files and filesystem slack,
and it matches `du` rather than a file manager's "size" column.

A few consequences worth knowing so the numbers never surprise you:

- **It can differ from apparent size (`st_size`).** A file manager usually shows
  apparent size — the logical length of the file. On-disk size can be smaller
  (sparse files, holes) or larger (block rounding).
- **Btrfs transparent compression.** On CachyOS the root filesystem is typically
  Btrfs, often with transparent compression (e.g. `zstd`). There, `st_blocks`
  reflects the **compressed** on-disk size — the real bytes occupied. So a 1 GiB
  log that compresses to 200 MiB shows as ~200 MiB here, while a file manager may
  still report ~1 GiB. That ~200 MiB is the correct "reclaimable" figure, and it
  is exactly what `du` reports too.
- **Hard links are counted once.** A file with multiple hard links inside the
  scan is counted a single time, matching `du`. Such entries are flagged so a
  future delete can warn that removing one link may not free the data until every
  link is gone.
- **Single filesystem.** A scan stays on the device the target path lives on; it
  does not cross into other mounts (`/proc`, `/sys`, external drives, etc.). This
  matches `du -x`.

## Building

Requires a Rust toolchain (install via [rustup](https://rustup.rs)).

```sh
cargo build --release          # build everything
cargo test  -p scanner         # run the scanner's correctness tests
```

## Running the visualizer

```sh
cargo run -p storagesifter             # pick a filesystem from the device list
cargo run -p storagesifter -- [PATH]   # …or scan PATH directly
```

A dark, GPU-rendered window opens and a background scan fills in a squarified
treemap: cells sized by on-disk usage and **colored by what they are** — caches
and build artifacts (`target/`, `node_modules/`, `.cache/`, `__pycache__`, …)
and everything inside them glow **amber** so reclaimable space stands out at a
glance, with media green, applications cyan, code blue, documents yellow, and
archives pink. One level of nested preview is drawn inside each folder.

- **Click** a folder to drill into it — the view zooms in, growing out of the
  cell you clicked.
- The **breadcrumb** (top bar) shows where you are; click any segment to jump
  back up. **Backspace** or **Esc** goes up one level.
- **Hover** a cell to read its full path, size, and category in the status bar.
- **Rescan** re-reads the directory; **Devices** (top-left) returns to the
  picker. The picker lists each filesystem with its used/free space; picking one
  scans the whole filesystem and **crosses btrfs subvolumes** (`/home`, `/var`,
  …) — which are outlined in cyan so the boundaries are obvious. (Scanning a path
  directly still stays on its single filesystem, matching `du -x`.)
- **Ctrl/Shift-click** cells to build a multi-selection; a bar shows the count
  and total reclaimable size, with **Move to Trash / Delete / Clear**.
- **Right-click** any cell for **Safe to delete?**, **Properties**, **Reveal in
  file manager**, or to delete. The trash is the default (reversible); every
  delete is confirmed, and permanent deletes — or anything outside your home
  directory — are flagged in the dialog. Protected system roots are refused
  outright.
- **Safe to delete?** opens an instant report that tells you *what a file or
  folder actually is* — a cache, build output, version-control history, personal
  media, installed software, a code project, credentials — and gives a plain
  verdict (*Safe to delete* → *Don't delete*) with the reasons behind it. A
  built-in knowledge catalog recognizes dozens of common tool directories
  (npm/yarn/pnpm/pip/cargo/go/gradle caches, Docker & Podman storage, browser
  profiles, the systemd journal, the pacman cache, Flatpak data, language
  version managers, …) and, when a tool offers a cleaner way to reclaim the
  space than a plain delete, shows the **recommended command** to run (e.g.
  `npm cache clean --force`, `docker system prune -a`, `paccache -r`,
  `journalctl --vacuum-time=2weeks`) with a one-click **Copy**. For
  system-managed caches it **detects your distro's package manager** (pacman,
  apt, dnf, zypper, apk, xbps, portage, or nix) and gives the matching clean
  command, so the advice is right wherever you run it. The analysis reads only
  the already-scanned, in-memory tree (no re-scan, no disk reads, no network)
  and is computed once, so it never slows the app down.
- **Keyboard**: `Delete` trashes the selection, `Shift+Delete` deletes it
  permanently, `Ctrl+A` selects everything in view, `Esc` clears the selection,
  `Backspace` goes up, `F5` rescans. All bindings are **configurable in ⚙
  Settings** (which also toggles zoom animations and the folder preview depth);
  settings persist to `~/.config/storagesifter/settings.json`.

The first build pulls in the GUI stack and takes a few minutes; after that,
rebuilds of the app are seconds. A `--release` build is available for maximum
scan speed but isn't needed for everyday use.

## Phase 1: the scanning engine

A throwaway CLI exercises the scanner and prints sizes in **raw bytes** so they
can be checked against `du` to the byte:

```sh
cargo run --release -p scanner --bin scan -- /path/to/dir
```

It prints the on-disk total, a summary line (node count, unreadable paths,
hard-link count), and the top 20 consumers.

### Verifying correctness against `du`

The scanner's total should match `du` exactly. Because the scan stays on one
filesystem, pin `du` to one filesystem too with `-x`:

```sh
du -xsB1 /path/to/dir          # one number — should equal our "on-disk total"
```

If they ever disagree, `du -xB1 /path/to/dir` gives a per-directory breakdown to
locate the discrepancy.

## Scope

Built for one machine (CachyOS / Linux). It uses Linux-specific APIs directly and
does not carry cross-platform abstractions.

## About

StorageSifter is a free and open-source product of **Fopull LLC** — a software
studio by Ty Johnston. More projects at **[fopull.com](https://fopull.com)**.

## License

[MIT](LICENSE) © 2026 Fopull LLC
