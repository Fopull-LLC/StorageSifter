# StorageSifter

A fast, lightweight disk-usage visualizer for Linux. It maps a filesystem as a
**squarified treemap** — nested rectangles sized proportionally to what they
consume — so the biggest space hogs are obvious at a glance. Think SpaceSniffer,
but native, GPU-accelerated, and built for a single Linux machine.

> Status: early development. Phase 0 (scaffold) and Phase 1 (scanning engine)
> are in place. The treemap UI lands in later phases.

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

## License

[MIT](LICENSE)
