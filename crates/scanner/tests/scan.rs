//! Integration tests for the scanner against real temp-dir fixtures.
//!
//! These cover the four correctness properties the engine must get right from
//! day one: bottom-up size aggregation, symlinks-as-leaves, hard-link
//! de-duplication, and permission-error capture. The single-filesystem guard is
//! unit-tested inside `walk.rs` (it needs crate-internal access to inject a
//! mismatching device without root privileges).

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;

use scanner::{scan, NodeFlags, NodeKind, Tree};

/// `st_blocks * 512` for a single path (no recursion).
fn on_disk(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

/// Independent reference: recursive sum of on-disk sizes on a single device,
/// treating symlinks as leaves and WITHOUT hard-link de-duplication. Mirrors the
/// scanner's accounting closely enough to validate aggregation for trees that
/// contain no hard links.
fn reference_total(root: &Path) -> u64 {
    fn rec(path: &Path, root_dev: u64) -> u64 {
        let meta = fs::symlink_metadata(path).unwrap();
        let mut total = meta.blocks() * 512;
        if meta.is_dir() {
            if let Ok(rd) = fs::read_dir(path) {
                for entry in rd.flatten() {
                    let cpath = entry.path();
                    let cmeta = match fs::symlink_metadata(&cpath) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if cmeta.dev() != root_dev {
                        continue;
                    }
                    total += rec(&cpath, root_dev);
                }
            }
        }
        total
    }
    let root_dev = fs::symlink_metadata(root).unwrap().dev();
    rec(root, root_dev)
}

fn write_file(path: &Path, bytes: usize) {
    let mut f = File::create(path).unwrap();
    f.write_all(&vec![0u8; bytes]).unwrap();
    f.sync_all().unwrap();
}

/// Id of the first node with the given file name.
fn find(tree: &Tree, name: &str) -> scanner::NodeId {
    (0..tree.len() as u32)
        .find(|&id| tree.name(id) == name)
        .unwrap_or_else(|| panic!("no node named {name}"))
}

#[test]
fn aggregates_sizes_bottom_up() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_file(&root.join("a.bin"), 5000);
    fs::create_dir(root.join("sub")).unwrap();
    write_file(&root.join("sub/b.bin"), 3000);
    write_file(&root.join("sub/c.bin"), 0);

    let tree = scan(root).unwrap();

    // Root total matches an independent recursive sum.
    assert_eq!(tree.node(tree.root).size, reference_total(root));

    // The "sub" directory equals its own size plus its two files.
    let sub = find(&tree, "sub");
    let expected_sub = on_disk(&root.join("sub"))
        + on_disk(&root.join("sub/b.bin"))
        + on_disk(&root.join("sub/c.bin"));
    assert_eq!(tree.node(sub).size, expected_sub);

    // Parent pointers exist and point the right way.
    let b = find(&tree, "b.bin");
    assert_eq!(tree.node(b).parent, Some(sub));
    assert_eq!(tree.node(sub).parent, Some(tree.root));
}

#[test]
fn symlinks_are_leaves_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_file(&root.join("target.bin"), 40_000);
    symlink("target.bin", root.join("link.bin")).unwrap();
    fs::create_dir(root.join("realdir")).unwrap();
    write_file(&root.join("realdir/inner.bin"), 10_000);
    symlink("realdir", root.join("dirlink")).unwrap(); // symlink to a directory

    let tree = scan(root).unwrap();

    let link = find(&tree, "link.bin");
    assert_eq!(tree.node(link).kind, NodeKind::Symlink);
    assert!(tree.node(link).children.is_empty());
    // The link counts its own tiny size, NOT the 40 KB target.
    assert_eq!(tree.node(link).size, on_disk(&root.join("link.bin")));
    assert!(tree.node(link).size < 40_000);

    // A symlink to a directory is also a leaf: never traversed.
    let dirlink = find(&tree, "dirlink");
    assert_eq!(tree.node(dirlink).kind, NodeKind::Symlink);
    assert!(tree.node(dirlink).children.is_empty());

    // Total equals the no-dedup reference (which also treats symlinks as leaves).
    assert_eq!(tree.node(tree.root).size, reference_total(root));
}

#[test]
fn hardlinks_counted_once_but_flagged() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_file(&root.join("original.bin"), 50_000);
    fs::hard_link(root.join("original.bin"), root.join("clone.bin")).unwrap();
    write_file(&root.join("other.bin"), 7000);

    let tree = scan(root).unwrap();

    let original = find(&tree, "original.bin");
    let clone = find(&tree, "clone.bin");

    // Both are flagged hard-linked with the right link count.
    assert!(tree.node(original).is_hardlinked());
    assert!(tree.node(clone).is_hardlinked());
    assert_eq!(tree.node(original).nlink, 2);
    assert_eq!(tree.node(clone).nlink, 2);

    // Exactly one of the two carries the bytes; the other is deduped to zero.
    let counted = [original, clone]
        .iter()
        .filter(|&&id| tree.node(id).size > 0)
        .count();
    assert_eq!(counted, 1, "exactly one link should carry the size");
    let deduped = [original, clone]
        .iter()
        .filter(|&&id| tree.node(id).flags.contains(NodeFlags::DEDUPED))
        .count();
    assert_eq!(deduped, 1, "exactly one link should be marked deduped");

    // own_size is preserved on the deduped link even though its size is 0.
    for &id in &[original, clone] {
        assert_eq!(tree.node(id).own_size, on_disk(&root.join("original.bin")));
    }

    // The shared inode's bytes are counted exactly once in the total.
    let expected = on_disk(root)
        + on_disk(&root.join("original.bin")) // counted once
        + on_disk(&root.join("other.bin"));
    assert_eq!(tree.node(tree.root).size, expected);
}

#[test]
fn permission_errors_are_captured_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_file(&root.join("readable.bin"), 1000);
    let locked = root.join("locked");
    fs::create_dir(&locked).unwrap();
    write_file(&locked.join("secret.bin"), 9999);

    // Strip all permissions so the directory's contents can't be read.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    // If we're root (or the chmod didn't take), the guard can't be exercised;
    // skip rather than fail spuriously. Always restore perms so tempdir cleans up.
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipping: locked dir still readable (running as root?)");
        return;
    }

    let result = scan(root); // must not panic
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap(); // restore for cleanup

    let tree = result.expect("scan should succeed despite an unreadable dir");

    let locked_id = find(&tree, "locked");
    assert!(tree.node(locked_id).flags.contains(NodeFlags::UNREADABLE));
    assert!(tree.node(locked_id).children.is_empty());
    assert!(
        tree.unreadable.iter().any(|p| p.ends_with("locked")),
        "the unreadable path should be recorded"
    );

    // The locked dir's own size is counted; its hidden child is not.
    assert_eq!(tree.node(locked_id).size, on_disk(&locked));
}
