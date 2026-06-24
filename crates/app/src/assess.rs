//! "Safe to delete?" — a fast, local heuristic that judges how risky deleting a
//! given node is and explains what it is.
//!
//! It reads only the already-scanned in-memory tree (path, category, immediate
//! children, flags) plus a few name/location rules. No filesystem access, no
//! network, no recursion beyond one level of children — so it's effectively
//! instant and is computed once when the report opens, never per frame.

use std::path::Path;

use eframe::egui::Color32;
use scanner::safety::{classify, Class};
use scanner::{NodeId, NodeKind, Tree};

use crate::theme::Category;

/// How risky deleting this node is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Safe,
    Likely,
    Caution,
    Keep,
    Danger,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Safe => "Safe to delete",
            Verdict::Likely => "Likely safe to delete",
            Verdict::Caution => "Review before deleting",
            Verdict::Keep => "Probably keep",
            Verdict::Danger => "Don't delete",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Verdict::Safe => Color32::from_rgb(0x7e, 0xc9, 0x8a),
            Verdict::Likely => Color32::from_rgb(0x58, 0xd2, 0xc2),
            Verdict::Caution => Color32::from_rgb(0xd8, 0xa6, 0x57),
            Verdict::Keep => Color32::from_rgb(0x6f, 0x8b, 0xf2),
            Verdict::Danger => Color32::from_rgb(0xec, 0x5f, 0x78),
        }
    }
}

/// One reason for / against deleting (`good` = reassuring).
pub struct Point {
    pub good: bool,
    pub text: String,
}

impl Point {
    fn good(text: impl Into<String>) -> Self {
        Point {
            good: true,
            text: text.into(),
        }
    }
    fn bad(text: impl Into<String>) -> Self {
        Point {
            good: false,
            text: text.into(),
        }
    }
}

/// The full report shown in the "Safe to delete?" dialog.
pub struct Assessment {
    pub verdict: Verdict,
    pub headline: String,
    pub detail: String,
    pub points: Vec<Point>,
}

const CRED_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".password-store",
    ".aws",
    ".kube",
    "keyrings",
];
const MEDIA_DIRS: &[&str] = &["pictures", "photos", "videos", "music", "dcim"];
const DOC_DIRS: &[&str] = &["documents", "desktop"];
const PROJECT_MARKERS: &[&str] = &[
    "cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "cmakelists.txt",
    "makefile",
    ".git",
];

/// Analyze whether `id` is safe to delete. Pure, in-memory, bounded.
pub fn assess(tree: &Tree, id: NodeId, home: &Path) -> Assessment {
    let node = tree.node(id);
    let path = tree.path(id);
    let lower = tree.name(id).to_ascii_lowercase();
    let class = classify(&path, home);
    let category = Category::of(tree, id);
    let is_dir = node.kind == NodeKind::Dir;

    let mut points = Vec::new();
    points.push(match class {
        Class::Normal => Point::good("Inside your home directory"),
        Class::OutsideHome => Point::bad("Outside your home directory"),
        Class::System => Point::bad("In a system location"),
        Class::Critical => Point::bad("A protected system location"),
    });
    if node.is_hardlinked() {
        points.push(Point::bad(
            "Hard-linked — other copies share this data, so deleting it may not free the space",
        ));
    }

    // --- Most dangerous cases first.
    if class == Class::Critical {
        return finish(
            Verdict::Danger,
            "Protected system location",
            "This is essential to your operating system. StorageSifter refuses to delete it.",
            points,
        );
    }
    if CRED_DIRS.iter().any(|d| lower == *d) {
        points.push(Point::bad("Holds keys or credentials"));
        return finish(
            Verdict::Danger,
            "Keys & credentials",
            "Deleting this could lock you out of servers, encrypted data, or accounts. Keep it.",
            points,
        );
    }
    if class == Class::System {
        points.push(Point::bad("Part of the OS or an installed program"));
        return finish(
            Verdict::Caution,
            "System / installed software",
            "Likely belongs to installed software. Remove programs through your package manager rather than deleting files here.",
            points,
        );
    }

    // --- Clearly reclaimable: caches, build output, temp.
    if category == Category::Junk {
        let (headline, detail) = junk_kind(&lower);
        points.push(Point::good("Regenerated automatically when it's next needed"));
        return finish(Verdict::Safe, headline, detail, points);
    }

    // --- Things people usually want to keep.
    if matches!(lower.as_str(), ".git" | ".svn" | ".hg") {
        points.push(Point::bad("Holds your project's entire version history"));
        return finish(
            Verdict::Caution,
            "Version-control history",
            "Deleting this erases all commit history for the project (branches, past versions). The working files remain, but the history is gone — unless it's pushed to a remote.",
            points,
        );
    }
    if category == Category::Media || MEDIA_DIRS.contains(&lower.as_str()) {
        points.push(Point::bad("Personal media is usually irreplaceable"));
        return finish(
            Verdict::Keep,
            "Personal media",
            "Photos, video, or audio — often one-of-a-kind. Back up anything you care about before deleting.",
            points,
        );
    }
    if category == Category::Document || DOC_DIRS.contains(&lower.as_str()) {
        points.push(Point::bad("Documents are usually irreplaceable"));
        return finish(
            Verdict::Keep,
            "Documents",
            "Personal documents. Make sure you have a copy elsewhere before deleting.",
            points,
        );
    }
    if lower == ".config" || starts_within(&path, home, &[".config"]) {
        return finish(
            Verdict::Caution,
            "Application settings",
            "Configuration for your programs. Deleting resets them to defaults — you won't lose documents, but you'll lose customizations.",
            points,
        );
    }
    if starts_within(&path, home, &[".local", "share"]) || starts_within(&path, home, &[".var"]) {
        points.push(Point::bad("May hold app data or saved files"));
        return finish(
            Verdict::Caution,
            "Application data",
            "Data saved by an installed app (profiles, saves, databases). Deleting it can lose app state — check what app it belongs to.",
            points,
        );
    }

    // --- Code projects.
    if is_dir && has_project_marker(tree, id) {
        let versioned = has_child(tree, id, ".git");
        points.push(if versioned {
            Point::good("Under version control")
        } else {
            Point::bad("No version control detected here")
        });
        let detail = if versioned {
            "A code project under version control. If it's pushed to a remote it's recoverable, but check for uncommitted changes first. Tip: its build output (target/, node_modules/, …) is the safe part to clear."
        } else {
            "A code project with no version history here — it could contain work you can't get back. Review before deleting."
        };
        return finish(Verdict::Caution, "Code project", detail, points);
    }

    // --- Re-obtainable but worth a glance.
    if starts_within(&path, home, &["downloads"]) {
        points.push(Point::good("Downloaded files are usually re-downloadable"));
        return finish(
            Verdict::Likely,
            "Downloads",
            "Downloaded files — usually re-downloadable, but skim for anything you deliberately saved here.",
            points,
        );
    }
    if category == Category::Archive {
        points.push(Point::good("Archives/installers are often re-downloadable"));
        return finish(
            Verdict::Likely,
            "Archive / installer",
            "A compressed archive or installer. Often re-downloadable — just confirm it isn't your only copy of something.",
            points,
        );
    }
    if category == Category::App {
        points.push(Point::bad("An executable or library"));
        return finish(
            Verdict::Caution,
            "Application / library",
            "An executable or library — possibly installed software. Removing it can break programs that rely on it.",
            points,
        );
    }

    // --- Empty.
    if node.size == 0 {
        return finish(
            Verdict::Safe,
            "Empty",
            "There's nothing in here — safe to remove.",
            points,
        );
    }

    // --- Otherwise, characterize a folder by what's inside.
    if is_dir {
        if let Some((dominant, share)) = dominant_category(tree, id) {
            let pct = (share * 100.0).round() as u32;
            match dominant {
                Category::Junk => {
                    points.push(Point::good(format!("{pct}% caches / build artifacts inside")));
                    return finish(
                        Verdict::Likely,
                        "Mostly reclaimable",
                        "Most of what's in here is cache or build output that regenerates. Worth clearing, but glance inside first.",
                        points,
                    );
                }
                Category::Media => {
                    points.push(Point::bad(format!("{pct}% media inside")));
                    return finish(
                        Verdict::Keep,
                        "Mostly media",
                        "Mostly photos/video/audio — usually irreplaceable. Keep unless you've backed it up.",
                        points,
                    );
                }
                Category::Document => {
                    points.push(Point::bad(format!("{pct}% documents inside")));
                    return finish(
                        Verdict::Keep,
                        "Mostly documents",
                        "Mostly documents — likely irreplaceable. Back up before deleting.",
                        points,
                    );
                }
                Category::Code => {
                    points.push(Point::bad(format!("{pct}% source code inside")));
                    return finish(
                        Verdict::Caution,
                        "Mostly source code",
                        "Mostly source files — could be work you care about. Check for version control / uncommitted changes.",
                        points,
                    );
                }
                Category::App => {
                    points.push(Point::bad("Contains applications / libraries"));
                    return finish(
                        Verdict::Caution,
                        "Mostly applications",
                        "Contains executables or libraries — possibly installed software. Review before deleting.",
                        points,
                    );
                }
                _ => {}
            }
        }
        return finish(
            Verdict::Caution,
            "A folder of mixed contents",
            "StorageSifter couldn't pin down one kind of content. Drill in to see what's inside before deleting.",
            points,
        );
    }

    // --- Unknown file.
    finish(
        Verdict::Caution,
        "Unrecognized file",
        "An unfamiliar file type. Check what it is before deleting.",
        points,
    )
}

fn finish(verdict: Verdict, headline: &str, detail: &str, points: Vec<Point>) -> Assessment {
    Assessment {
        verdict,
        headline: headline.to_owned(),
        detail: detail.to_owned(),
        points,
    }
}

/// A more specific headline/detail for a junk node by name.
fn junk_kind(lower: &str) -> (&'static str, &'static str) {
    match lower {
        "target" | "build" | "dist" | "out" => (
            "Build output",
            "Compiled output that's recreated whenever you build the project. Safe to clear.",
        ),
        "node_modules" => (
            "Installed packages",
            "Downloaded JavaScript packages, restored by `npm install`. Safe to clear.",
        ),
        ".cache" | "cache" | "gpucache" | "shadercache" => (
            "Cache",
            "Temporary cached data that apps rebuild on demand. Safe to clear.",
        ),
        ".npm" | ".yarn" | ".gradle" | ".ccache" => (
            "Package-manager cache",
            "Cached downloads/build artifacts that are re-fetched when needed. Safe to clear.",
        ),
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".tox" => (
            "Python cache",
            "Generated Python caches, recreated automatically. Safe to clear.",
        ),
        _ => (
            "Cache / build artifacts",
            "Generated data that's recreated automatically when needed. Safe to clear.",
        ),
    }
}

/// True if `path` is at or under `home` joined with `segments`.
fn starts_within(path: &Path, home: &Path, segments: &[&str]) -> bool {
    if home.as_os_str().is_empty() {
        return false;
    }
    let mut base = home.to_path_buf();
    for s in segments {
        base.push(s);
    }
    path.starts_with(&base)
}

fn has_child(tree: &Tree, id: NodeId, name: &str) -> bool {
    tree.node(id)
        .children
        .iter()
        .any(|&c| tree.name(c).eq_ignore_ascii_case(name))
}

fn has_project_marker(tree: &Tree, id: NodeId) -> bool {
    tree.node(id).children.iter().any(|&c| {
        let n = tree.name(c).to_ascii_lowercase();
        PROJECT_MARKERS.contains(&n.as_str())
    })
}

/// The category that accounts for the most bytes among `id`'s immediate
/// children, and its share of the total. `None` if there are no children.
fn dominant_category(tree: &Tree, id: NodeId) -> Option<(Category, f32)> {
    let children = &tree.node(id).children;
    if children.is_empty() {
        return None;
    }
    // 8 categories; index by discriminant order.
    let mut totals = [0u64; 8];
    let mut grand = 0u64;
    for &c in children {
        let size = tree.node(c).size;
        totals[cat_index(Category::of(tree, c))] += size;
        grand += size;
    }
    if grand == 0 {
        return None;
    }
    let (idx, &best) = totals
        .iter()
        .enumerate()
        .max_by_key(|(_, &v)| v)
        .unwrap();
    Some((cat_from_index(idx), best as f32 / grand as f32))
}

fn cat_index(c: Category) -> usize {
    match c {
        Category::Junk => 0,
        Category::Media => 1,
        Category::Archive => 2,
        Category::App => 3,
        Category::Code => 4,
        Category::Document => 5,
        Category::Folder => 6,
        Category::Other => 7,
    }
}

fn cat_from_index(i: usize) -> Category {
    match i {
        0 => Category::Junk,
        1 => Category::Media,
        2 => Category::Archive,
        3 => Category::App,
        4 => Category::Code,
        5 => Category::Document,
        6 => Category::Folder,
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scanner::scan;
    use std::fs;
    use tempfile::tempdir;

    // A throwaway home that none of the temp paths live under, so `classify`
    // reports OutsideHome and the name/category heuristics drive the verdict.
    const NOWHERE: &str = "/home/nobody";

    fn child(tree: &Tree, name: &str) -> NodeId {
        let root = tree.root;
        *tree
            .node(root)
            .children
            .iter()
            .find(|&&c| tree.name(c) == name)
            .expect("child should exist")
    }

    fn verdict_for(make: impl FnOnce(&std::path::Path), name: &str) -> Verdict {
        let dir = tempdir().unwrap();
        make(dir.path());
        let tree = scan(dir.path()).unwrap();
        let id = child(&tree, name);
        assess(&tree, id, Path::new(NOWHERE)).verdict
    }

    #[test]
    fn cache_is_safe() {
        let v = verdict_for(
            |root| {
                let c = root.join(".cache");
                fs::create_dir(&c).unwrap();
                fs::write(c.join("blob"), b"junk-data").unwrap();
            },
            ".cache",
        );
        assert_eq!(v, Verdict::Safe);
    }

    #[test]
    fn node_modules_is_safe() {
        let v = verdict_for(
            |root| {
                let c = root.join("node_modules");
                fs::create_dir(&c).unwrap();
                fs::write(c.join("index.js"), b"x").unwrap();
            },
            "node_modules",
        );
        assert_eq!(v, Verdict::Safe);
    }

    #[test]
    fn ssh_dir_is_danger() {
        let v = verdict_for(
            |root| {
                let c = root.join(".ssh");
                fs::create_dir(&c).unwrap();
                fs::write(c.join("id_ed25519"), b"secret").unwrap();
            },
            ".ssh",
        );
        assert_eq!(v, Verdict::Danger);
    }

    #[test]
    fn git_dir_warns() {
        let v = verdict_for(
            |root| {
                let c = root.join(".git");
                fs::create_dir(&c).unwrap();
                fs::write(c.join("HEAD"), b"ref: refs/heads/main").unwrap();
            },
            ".git",
        );
        assert_eq!(v, Verdict::Caution);
    }

    #[test]
    fn code_project_warns() {
        let v = verdict_for(
            |root| {
                let c = root.join("proj");
                fs::create_dir(&c).unwrap();
                fs::write(c.join("Cargo.toml"), b"[package]").unwrap();
            },
            "proj",
        );
        assert_eq!(v, Verdict::Caution);
    }
}
