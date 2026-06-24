//! StorageSifter scanning engine.
//!
//! A small, UI-free library that walks a single filesystem and returns an arena
//! [`Tree`] of on-disk sizes. It has no knowledge of the GUI: data in (a path),
//! data out (a tree), so it can be unit-tested and reused on its own.
//!
//! # What "size" means
//!
//! Every size is **real on-disk consumption**: `st_blocks * 512`, the number of
//! 512-byte blocks the filesystem actually allocated. This accounts for sparse
//! files and filesystem slack, and is the right number for "what will I actually
//! reclaim". It can disagree with a file manager's apparent size (`st_size`),
//! especially on Btrfs with transparent compression, where `st_blocks` reflects
//! the *compressed* on-disk size. See the project README for details.
//!
//! The total reported by [`scan`] matches `du -xsB1 <path>` byte-for-byte.

mod mounts;
mod tree;
mod walk;

pub mod safety;

pub use tree::{Node, NodeFlags, NodeId, NodeKind, Tree};
pub use walk::{scan, scan_filesystem, scan_filesystem_cancellable};
