//! Throwaway CLI for validating the scanner against `du`.
//!
//! Prints the on-disk total in **raw bytes** so it can be compared directly to:
//!
//! ```text
//! du -xsB1 <path>      # one number: should match our total exactly
//! du -xB1  <path>      # per-directory breakdown, for drilling into mismatches
//! ```
//!
//! The `-x` flag matters: our scanner stays on one filesystem, so the comparison
//! has to pin `du` to one filesystem too.

use std::cmp::Reverse;
use std::path::PathBuf;
use std::time::Instant;

use scanner::{scan, NodeFlags, NodeId};

fn main() {
    let path = match std::env::args_os().nth(1) {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: scan <path>");
            eprintln!("verify with: du -xsB1 <path>");
            std::process::exit(2);
        }
    };

    let started = Instant::now();
    let tree = match scan(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("scan failed for {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();

    let total = tree.node(tree.root).size;
    let hardlinked = tree
        .nodes
        .iter()
        .filter(|n| n.flags.contains(NodeFlags::HARDLINKED))
        .count();

    println!("scanned:       {}", tree.path(tree.root).display());
    println!("on-disk total: {total} bytes");
    println!(
        "nodes: {}   unreadable: {}   hardlinked files: {}   elapsed: {:.3}s",
        tree.len(),
        tree.unreadable.len(),
        hardlinked,
        elapsed.as_secs_f64()
    );
    println!("compare:       du -xsB1 {}", path.display());

    // Top 20 consumers by on-disk size (excluding the root itself).
    let mut ids: Vec<NodeId> = (0..tree.len() as NodeId)
        .filter(|&i| i != tree.root)
        .collect();
    ids.sort_unstable_by_key(|&i| Reverse(tree.node(i).size));

    println!("\ntop 20 (on-disk bytes):");
    for &id in ids.iter().take(20) {
        let n = tree.node(id);
        let tag = if n.flags.contains(NodeFlags::DEDUPED) {
            "  [hardlink — already counted elsewhere]"
        } else if n.is_hardlinked() {
            "  [hardlink]"
        } else {
            ""
        };
        println!("{:>16}  {}{}", n.size, tree.path(id).display(), tag);
    }

    if !tree.unreadable.is_empty() {
        println!("\nunreadable paths ({}):", tree.unreadable.len());
        for p in tree.unreadable.iter().take(10) {
            println!("  {}", p.display());
        }
        if tree.unreadable.len() > 10 {
            println!("  … and {} more", tree.unreadable.len() - 10);
        }
    }
}
