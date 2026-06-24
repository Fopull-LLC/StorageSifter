//! Parallel, single-filesystem directory walk.
//!
//! Scanning happens in two stages:
//!
//! 1. [`build`] walks the tree in parallel (via Rayon), calling `lstat` on every
//!    entry and reading directories concurrently. It produces an owned [`Raw`]
//!    tree. Symlinks are never followed; subtrees on a different device are
//!    pruned (matching `du -x`); unreadable directories are flagged, not fatal.
//! 2. [`flatten`] turns the `Raw` tree into the flat arena [`Tree`] in a single
//!    deterministic depth-first pass: it assigns ids and parent pointers,
//!    de-duplicates hard links by `(st_dev, st_ino)`, and aggregates sizes
//!    bottom-up.
//!
//! Doing de-duplication in the sequential pass — rather than during the parallel
//! walk — keeps the walk free of shared mutable state and makes *which* link
//! gets counted deterministic: the first one in depth-first order, like `du`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rustc_hash::FxHashSet;

use crate::tree::{path_of, Node, NodeFlags, NodeId, NodeKind, Tree};

/// Owned, intermediate tree produced by the parallel walk.
struct Raw {
    name: OsString,
    kind: NodeKind,
    dev: u64,
    ino: u64,
    nlink: u64,
    /// `st_blocks * 512` for this entry itself.
    own_size: u64,
    /// Set when this directory could not be read.
    unreadable: bool,
    /// Set when this entry's device differs from its parent's — a mount /
    /// subvolume boundary.
    is_mount: bool,
    children: Vec<Raw>,
}

/// Scan `root`, staying on the single filesystem `root` lives on (matches
/// `du -x`). Returns an error only if `root` itself cannot be `lstat`-ed.
pub fn scan(root: &Path) -> io::Result<Tree> {
    let meta = fs::symlink_metadata(root)?;
    let mut allowed = FxHashSet::default();
    allowed.insert(meta.dev());
    scan_with_allowed(root, allowed)
}

/// Like [`scan`], but follows the whole physical filesystem `root` lives on,
/// crossing into sibling subvolumes / bind mounts on the same source device
/// (e.g. btrfs `/home`, `/var`). Genuinely different drives are still pruned.
pub fn scan_filesystem(root: &Path) -> io::Result<Tree> {
    fs::symlink_metadata(root)?; // surface a clear error if `root` is unreadable
    let allowed = crate::mounts::same_filesystem_devices(root);
    scan_with_allowed(root, allowed)
}

/// Scan `root`, descending into any child whose device is in `allowed`. Exposed
/// within the crate so tests can control exactly which devices are crossed.
pub(crate) fn scan_with_allowed(root: &Path, allowed: FxHashSet<u64>) -> io::Result<Tree> {
    let meta = fs::symlink_metadata(root)?;
    let raw = build(root.to_path_buf(), &meta, &allowed);
    Ok(flatten(raw, root, meta.dev()))
}

fn kind_of(meta: &fs::Metadata) -> NodeKind {
    let ft = meta.file_type();
    if ft.is_dir() {
        NodeKind::Dir
    } else if ft.is_file() {
        NodeKind::File
    } else if ft.is_symlink() {
        NodeKind::Symlink
    } else {
        NodeKind::Other
    }
}

/// Recursively build the `Raw` subtree rooted at `path`. `meta` is the result of
/// `lstat(path)`, already obtained by the caller. Directory children are read
/// and `lstat`-ed; children on a device not in `allowed` are pruned, and the
/// survivors are recursed into in parallel. A child whose device differs from
/// its parent's just crossed a mount/subvolume boundary and is flagged.
fn build(path: PathBuf, meta: &fs::Metadata, allowed: &FxHashSet<u64>) -> Raw {
    let own_size = meta.blocks() * 512;
    let name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| path.as_os_str().to_os_string());

    let leaf = |unreadable: bool, kind: NodeKind, children: Vec<Raw>| Raw {
        name: name.clone(),
        kind,
        dev: meta.dev(),
        ino: meta.ino(),
        nlink: meta.nlink(),
        own_size,
        unreadable,
        is_mount: false,
        children,
    };

    if !meta.is_dir() {
        // File, symlink, or other special file: a leaf. We never follow links.
        return leaf(false, kind_of(meta), Vec::new());
    }

    // Directory: read its entries. A read failure (e.g. EACCES) is recorded but
    // never aborts the scan.
    let read_dir = match fs::read_dir(&path) {
        Ok(rd) => rd,
        Err(_) => return leaf(true, NodeKind::Dir, Vec::new()),
    };

    let mut unreadable = false;
    let mut kids: Vec<(PathBuf, fs::Metadata)> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                unreadable = true;
                continue;
            }
        };
        let child_path = entry.path();
        let child_meta = match fs::symlink_metadata(&child_path) {
            Ok(m) => m,
            Err(_) => {
                unreadable = true;
                continue;
            }
        };
        // Stay within the allowed device set (one filesystem, or one physical
        // filesystem including its subvolumes — see `scan` vs `scan_filesystem`).
        if !allowed.contains(&child_meta.dev()) {
            continue;
        }
        kids.push((child_path, child_meta));
    }

    let parent_dev = meta.dev();
    let children: Vec<Raw> = kids
        .into_par_iter()
        .map(|(p, m)| {
            let is_mount = m.dev() != parent_dev;
            let mut child = build(p, &m, allowed);
            child.is_mount = is_mount;
            child
        })
        .collect();

    leaf(unreadable, NodeKind::Dir, children)
}

/// Flatten the `Raw` tree into the arena: assign ids/parents, de-duplicate hard
/// links, then aggregate sizes bottom-up.
fn flatten(raw: Raw, root_path: &Path, root_dev: u64) -> Tree {
    let mut nodes: Vec<Node> = Vec::new();
    let mut names = String::new();
    let mut seen: FxHashSet<(u64, u64)> = FxHashSet::default();
    let mut unreadable: Vec<PathBuf> = Vec::new();

    // Iterative pre-order DFS. Each stack item carries its parent id. Children
    // are pushed in reverse so they pop back in original order, giving stable
    // ascending ids. A parent always gets a smaller id than any descendant.
    let mut stack: Vec<(Raw, Option<NodeId>, bool)> = vec![(raw, None, true)];
    while let Some((r, parent, is_root)) = stack.pop() {
        let id = nodes.len() as NodeId;

        // Name: the root stores the full scanned path; others store the basename.
        let name_str = if is_root {
            root_path.to_string_lossy()
        } else {
            r.name.to_string_lossy()
        };
        let start = names.len() as u32;
        names.push_str(&name_str);
        let end = names.len() as u32;

        // Flags + hard-link de-duplication.
        let mut flags = NodeFlags::default();
        let mut counted = r.own_size;
        if r.unreadable {
            flags.insert(NodeFlags::UNREADABLE);
        }
        if r.is_mount {
            flags.insert(NodeFlags::MOUNTPOINT);
        }
        let is_file_like = r.kind != NodeKind::Dir;
        if is_file_like && r.nlink > 1 {
            flags.insert(NodeFlags::HARDLINKED);
            if !seen.insert((r.dev, r.ino)) {
                // Already attributed to an earlier link: count zero here.
                flags.insert(NodeFlags::DEDUPED);
                counted = 0;
            }
        }

        nodes.push(Node {
            parent,
            kind: r.kind,
            flags,
            nlink: r.nlink,
            own_size: r.own_size,
            size: counted, // descendants are folded in below
            name: start..end,
            children: Vec::new(),
        });

        if let Some(p) = parent {
            nodes[p as usize].children.push(id);
        }
        if flags.contains(NodeFlags::UNREADABLE) {
            unreadable.push(path_of(&nodes, &names, id));
        }

        for child in r.children.into_iter().rev() {
            stack.push((child, Some(id), false));
        }
    }

    // Bottom-up aggregation: because every parent precedes its descendants,
    // iterating in reverse means each child's subtotal is final before its
    // parent consumes it.
    for i in (0..nodes.len()).rev() {
        if let Some(p) = nodes[i].parent {
            let s = nodes[i].size;
            nodes[p as usize].size += s;
        }
    }

    Tree::new(nodes, names, root_dev, unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_filesystem_guard_prunes_foreign_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/file.bin"), vec![0u8; 4096]).unwrap();

        let real_dev = fs::symlink_metadata(root).unwrap().dev();

        // Correct device allowed: children are present.
        let mut real = FxHashSet::default();
        real.insert(real_dev);
        let ok = scan_with_allowed(root, real).unwrap();
        assert!(ok.len() > 1, "real device should include children");

        // Only a foreign device allowed: every child crosses a (simulated)
        // boundary and is pruned, leaving just the root node — exactly what
        // `du -x` does at a mount point.
        let mut wrong = FxHashSet::default();
        wrong.insert(real_dev.wrapping_add(1));
        let pruned = scan_with_allowed(root, wrong).unwrap();
        assert_eq!(pruned.len(), 1, "all foreign-device children pruned");
        assert_eq!(pruned.root, 0);
    }
}
