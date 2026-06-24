//! Arena-backed directory tree.
//!
//! Every scanned filesystem object becomes a [`Node`] stored in a single
//! `Vec<Node>` and addressed by a [`NodeId`] (a `u32` index). Each node keeps a
//! reference to its `parent` as well as its `children`, so a future delete can
//! walk *up* the ancestor chain to subtract reclaimed bytes in `O(depth)`
//! without rebuilding the tree.

use std::ops::Range;
use std::path::PathBuf;

/// Index into [`Tree::nodes`]. `u32` keeps each node small while still
/// addressing far more entries than any real filesystem contains.
pub type NodeId = u32;

/// What kind of filesystem object a node represents.
///
/// Symlinks are always recorded as leaves and never traversed (see
/// [`crate::scan`]); that is what structurally prevents symlink loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
    Symlink,
    /// Sockets, FIFOs, block/char devices — counted but never traversed.
    Other,
}

/// Per-node bit flags, packed into a single byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeFlags(u8);

impl NodeFlags {
    /// A regular file with `st_nlink > 1`: its data is shared with other hard
    /// links, so deleting this entry may not actually reclaim its bytes until
    /// every link is gone.
    pub const HARDLINKED: u8 = 1 << 0;
    /// A directory we could not read (e.g. permission denied). Its own size is
    /// counted, but its contents are missing from the tree. The scan continues.
    pub const UNREADABLE: u8 = 1 << 1;
    /// A hard link whose bytes were attributed to an earlier-seen link, so this
    /// node's `size` is 0 even though `own_size` is the real size.
    pub const DEDUPED: u8 = 1 << 2;
    /// This entry sits at a mount / subvolume boundary (its device differs from
    /// its parent's). The UI flags these so the separation is obvious.
    pub const MOUNTPOINT: u8 = 1 << 3;

    #[inline]
    pub fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    #[inline]
    pub fn insert(&mut self, bit: u8) {
        self.0 |= bit;
    }
}

/// A single filesystem object in the arena.
#[derive(Debug, Clone)]
pub struct Node {
    /// Parent node, or `None` for the scan root.
    pub parent: Option<NodeId>,
    pub kind: NodeKind,
    pub flags: NodeFlags,
    /// Hard-link count (`st_nlink`). For files, `> 1` means the data is shared.
    pub nlink: u64,
    /// This entry's *own* on-disk size in bytes (`st_blocks * 512`), before
    /// hard-link de-duplication. Always the real size, even for a duplicate link.
    pub own_size: u64,
    /// Aggregated on-disk size: this node's de-duplicated bytes plus those of
    /// all descendants. This is the number that matches `du -xsB1`.
    pub size: u64,
    /// Slice of [`Tree::names`] holding this node's file name (not the path).
    pub(crate) name: Range<u32>,
    /// Child node ids; empty for non-directories.
    pub children: Vec<NodeId>,
}

impl Node {
    /// `true` if this is a hard-linked regular file (`st_nlink > 1`).
    #[inline]
    pub fn is_hardlinked(&self) -> bool {
        self.flags.contains(NodeFlags::HARDLINKED)
    }

    /// `true` if this entry sits at a mount / subvolume boundary.
    #[inline]
    pub fn is_mountpoint(&self) -> bool {
        self.flags.contains(NodeFlags::MOUNTPOINT)
    }
}

/// The result of a scan: an arena of [`Node`]s plus lookup metadata.
#[derive(Debug)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Backing storage for all node names, concatenated. Indexed via each
    /// node's private `name` range — one allocation instead of one per node.
    names: String,
    /// The scan root, always node id 0.
    pub root: NodeId,
    /// `st_dev` of the scan root. Subtrees on other devices were skipped
    /// (single-filesystem scan, matching `du -x`).
    pub root_dev: u64,
    /// Paths that could not be read during the scan. Collecting these lets the
    /// UI report "N unreadable" without ever aborting the scan.
    pub unreadable: Vec<PathBuf>,
}

impl Tree {
    /// This node's file name (final path component). The root's name is the
    /// full path that was scanned.
    pub fn name(&self, id: NodeId) -> &str {
        let r = &self.nodes[id as usize].name;
        &self.names[r.start as usize..r.end as usize]
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Reconstruct the absolute path of a node by walking up the parent chain.
    pub fn path(&self, id: NodeId) -> PathBuf {
        path_of(&self.nodes, &self.names, id)
    }

    /// Detach `id` (and its subtree) after it has been deleted on disk: unlink
    /// it from its parent and subtract its size from every ancestor, in
    /// `O(depth)`. The node stays in the arena but is no longer reachable from
    /// the root, so it stops rendering. Returns the freed size; a no-op on the
    /// root.
    pub fn remove_subtree(&mut self, id: NodeId) -> u64 {
        let (size, parent) = {
            let node = &self.nodes[id as usize];
            (node.size, node.parent)
        };
        let Some(parent) = parent else {
            return 0;
        };
        let siblings = &mut self.nodes[parent as usize].children;
        if let Some(pos) = siblings.iter().position(|&c| c == id) {
            siblings.remove(pos);
        }
        let mut ancestor = Some(parent);
        while let Some(a) = ancestor {
            let node = &mut self.nodes[a as usize];
            node.size = node.size.saturating_sub(size);
            ancestor = node.parent;
        }
        size
    }

    /// Construct a [`Tree`] from already-built parts. Used by the walk module.
    pub(crate) fn new(
        nodes: Vec<Node>,
        names: String,
        root_dev: u64,
        unreadable: Vec<PathBuf>,
    ) -> Self {
        Tree {
            nodes,
            names,
            root: 0,
            root_dev,
            unreadable,
        }
    }
}

/// Shared path reconstruction, usable both via [`Tree::path`] and during the
/// build before a `Tree` value exists.
pub(crate) fn path_of(nodes: &[Node], names: &str, id: NodeId) -> PathBuf {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = Some(id);
    while let Some(c) = cur {
        let n = &nodes[c as usize];
        parts.push(&names[n.name.start as usize..n.name.end as usize]);
        cur = n.parent;
    }
    parts.reverse();
    // The root node holds the full scanned path; the rest are basenames.
    let mut path = PathBuf::from(parts[0]);
    for p in &parts[1..] {
        path.push(p);
    }
    path
}
