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
    /// The recommended way to clear this, when a tool offers a better command
    /// than a plain delete (e.g. `npm cache clean --force`). Shown verbatim.
    pub command: Option<String>,
    pub points: Vec<Point>,
}

/// One entry in the knowledge catalog: a recognizable kind of file/dir and how
/// best to clean it up. Matched by path substring (and/or a marker child file),
/// so adding coverage is just adding a row.
struct Tool {
    /// Lowercased path fragments; a match on any one identifies this tool.
    needles: &'static [&'static str],
    /// …or recognize it by an immediate child file (e.g. `pyvenv.cfg`).
    marker: Option<&'static str>,
    kind: &'static str,
    verdict: Verdict,
    detail: &'static str,
    /// The recommended cleanup command, if one is better than deleting by hand.
    command: Option<&'static str>,
    /// Lives in a system location and typically needs root to clear.
    root: bool,
}

/// The catalog, ordered most-specific path first so generic entries (e.g. a bare
/// `~/.cache`) only match when nothing more precise does.
static CATALOG: &[Tool] = &[
    // ---- Containers & VMs: deleting the files by hand can corrupt state. ----
    Tool {
        needles: &["/var/lib/docker", "/.local/share/docker"],
        marker: None,
        kind: "Docker data (images, containers, volumes)",
        verdict: Verdict::Caution,
        detail: "Docker's images, containers, and volumes. Deleting these by hand can corrupt Docker and wipe volumes you still need — use Docker's own pruning, which reclaims space safely.",
        command: Some("docker system prune -a --volumes"),
        root: true,
    },
    Tool {
        needles: &["/.local/share/containers", "/var/lib/containers"],
        marker: None,
        kind: "Container storage (Podman)",
        verdict: Verdict::Caution,
        detail: "Podman's image and container storage. Prune it with Podman rather than deleting files directly.",
        command: Some("podman system prune -a --volumes"),
        root: false,
    },
    Tool {
        needles: &["/var/lib/libvirt/images"],
        marker: None,
        kind: "Virtual machine disk images",
        verdict: Verdict::Keep,
        detail: "These are your VMs' virtual disks — deleting one erases that machine's entire disk. Remove VMs through virt-manager / virsh instead.",
        command: Some("virsh vol-list --pool default"),
        root: true,
    },
    // ---- System caches & logs: reclaimable, but root + a proper tool. ----
    Tool {
        needles: &["/var/cache/pacman"],
        marker: None,
        kind: "Pacman package cache",
        verdict: Verdict::Likely,
        detail: "Downloaded packages kept so you can reinstall or roll back. The space is reclaimable — trim it safely with paccache (which keeps the most recent versions) instead of deleting everything.",
        command: Some("sudo paccache -r        # or, to remove all: sudo pacman -Sc"),
        root: true,
    },
    Tool {
        needles: &["/var/cache/apt"],
        marker: None,
        kind: "APT package cache",
        verdict: Verdict::Likely,
        detail: "Downloaded .deb packages kept by APT. The space is reclaimable — clear it with apt rather than deleting the files by hand.",
        command: Some("sudo apt clean"),
        root: true,
    },
    Tool {
        needles: &["/var/cache/dnf", "/var/cache/yum"],
        marker: None,
        kind: "DNF / YUM package cache",
        verdict: Verdict::Likely,
        detail: "Downloaded packages and metadata kept by DNF/YUM. Reclaimable — clear it with the package manager.",
        command: Some("sudo dnf clean all        # or: sudo yum clean all"),
        root: true,
    },
    Tool {
        needles: &["/var/cache/zypp"],
        marker: None,
        kind: "Zypper package cache",
        verdict: Verdict::Likely,
        detail: "Downloaded packages kept by zypper. Reclaimable — clear it with zypper.",
        command: Some("sudo zypper clean --all"),
        root: true,
    },
    Tool {
        needles: &["/var/cache/apk"],
        marker: None,
        kind: "apk package cache",
        verdict: Verdict::Likely,
        detail: "Downloaded packages kept by apk (Alpine). Reclaimable — clear it with apk.",
        command: Some("sudo apk cache clean"),
        root: true,
    },
    Tool {
        needles: &["/var/cache/distfiles", "/var/cache/binpkgs"],
        marker: None,
        kind: "Portage cache (Gentoo)",
        verdict: Verdict::Likely,
        detail: "Source tarballs and binary packages cached by Portage. Reclaimable — trim with eclean.",
        command: Some("sudo eclean-dist        # from gentoolkit"),
        root: true,
    },
    Tool {
        needles: &["/nix/store"],
        marker: None,
        kind: "Nix store",
        verdict: Verdict::Caution,
        detail: "The Nix package store. Never delete entries by hand — it will break installed software. Reclaim space by collecting garbage, which removes only paths nothing references.",
        command: Some("nix-collect-garbage -d"),
        root: true,
    },
    Tool {
        needles: &["/var/log/journal"],
        marker: None,
        kind: "systemd journal logs",
        verdict: Verdict::Likely,
        detail: "System logs. Useful for debugging, but you can cap them by age or size rather than deleting them outright.",
        command: Some("sudo journalctl --vacuum-time=2weeks    # or --vacuum-size=200M"),
        root: true,
    },
    Tool {
        needles: &["/var/lib/systemd/coredump"],
        marker: None,
        kind: "Crash core dumps",
        verdict: Verdict::Safe,
        detail: "Memory dumps saved when programs crashed — only useful for post-mortem debugging. Safe to clear.",
        command: None,
        root: true,
    },
    Tool {
        needles: &["/var/lib/flatpak"],
        marker: None,
        kind: "Flatpak apps & runtimes (system)",
        verdict: Verdict::Caution,
        detail: "System-wide Flatpak apps and shared runtimes. Remove apps you don't use, then clear orphaned runtimes — don't delete the files by hand.",
        command: Some("flatpak uninstall --unused        # then: flatpak uninstall <app-id>"),
        root: true,
    },
    Tool {
        needles: &["/var/lib/snapd", "/var/cache/snapd"],
        marker: None,
        kind: "Snap data",
        verdict: Verdict::Caution,
        detail: "Snap packages and their retained old revisions. Remove snaps you don't use and reduce how many old revisions are kept.",
        command: Some("sudo snap set system refresh.retain=2"),
        root: true,
    },
    Tool {
        needles: &["/var/tmp/"],
        marker: None,
        kind: "Temporary files",
        verdict: Verdict::Likely,
        detail: "Scratch space programs leave under /var/tmp. Usually safe to remove — though a running program could be using some of it right now.",
        command: None,
        root: false,
    },
    // ---- JavaScript / web toolchains ----
    Tool {
        needles: &["/.npm/_cacache", "/.npm"],
        marker: None,
        kind: "npm cache",
        verdict: Verdict::Safe,
        detail: "npm's package download cache. Safe to remove — npm refills it on the next install — but its own command verifies and clears it cleanly.",
        command: Some("npm cache clean --force"),
        root: false,
    },
    Tool {
        needles: &["/.cache/yarn", "/.yarn/cache", "/.yarn-cache"],
        marker: None,
        kind: "Yarn cache",
        verdict: Verdict::Safe,
        detail: "Yarn's package cache. Re-populated on the next install.",
        command: Some("yarn cache clean"),
        root: false,
    },
    Tool {
        needles: &["/.local/share/pnpm/store", "/.pnpm-store"],
        marker: None,
        kind: "pnpm store",
        verdict: Verdict::Safe,
        detail: "pnpm's global content-addressable package store. Prune unreferenced packages rather than wiping it, so your other projects keep their links.",
        command: Some("pnpm store prune"),
        root: false,
    },
    Tool {
        needles: &["/.bun/install/cache"],
        marker: None,
        kind: "Bun cache",
        verdict: Verdict::Safe,
        detail: "Bun's package cache. Re-downloaded as needed.",
        command: Some("bun pm cache rm"),
        root: false,
    },
    Tool {
        needles: &["/.cache/ms-playwright"],
        marker: None,
        kind: "Playwright browsers",
        verdict: Verdict::Safe,
        detail: "Browser binaries Playwright downloaded for testing. Re-installable with `npx playwright install`.",
        command: Some("npx playwright uninstall --all"),
        root: false,
    },
    Tool {
        needles: &["/.cache/cypress"],
        marker: None,
        kind: "Cypress cache",
        verdict: Verdict::Safe,
        detail: "Cypress's downloaded test-runner binaries. Re-downloaded on the next run.",
        command: Some("cypress cache clear"),
        root: false,
    },
    Tool {
        needles: &["/.cache/electron", "/.electron"],
        marker: None,
        kind: "Electron download cache",
        verdict: Verdict::Safe,
        detail: "Cached Electron binaries downloaded by builds. Re-fetched when needed.",
        command: None,
        root: false,
    },
    // ---- Python ----
    Tool {
        needles: &["/.cache/pip"],
        marker: None,
        kind: "pip cache",
        verdict: Verdict::Safe,
        detail: "pip's wheel and download cache. Re-downloaded on the next install.",
        command: Some("pip cache purge"),
        root: false,
    },
    Tool {
        needles: &["/.cache/huggingface", "/.cache/torch"],
        marker: None,
        kind: "ML model cache",
        verdict: Verdict::Likely,
        detail: "Downloaded machine-learning models and datasets. Reclaimable, but can be many gigabytes and slow (or gated) to re-download — make sure you won't need them soon.",
        command: None,
        root: false,
    },
    Tool {
        needles: &["/conda/pkgs", "/.conda/pkgs", "/miniconda3/pkgs", "/anaconda3/pkgs"],
        marker: None,
        kind: "Conda package cache",
        verdict: Verdict::Safe,
        detail: "Conda's package and tarball cache. Clear it (and other caches) with conda's own command.",
        command: Some("conda clean --all"),
        root: false,
    },
    Tool {
        needles: &[],
        marker: Some("pyvenv.cfg"),
        kind: "Python virtual environment",
        verdict: Verdict::Likely,
        detail: "A self-contained Python environment. Safe to delete and recreate from your requirements — but it is not a cache, so you'll need to rebuild it before running the project again.",
        command: Some("python -m venv .venv && pip install -r requirements.txt"),
        root: false,
    },
    // ---- Rust / Go / C / JVM / .NET / Ruby / PHP ----
    Tool {
        needles: &["/.cargo/registry", "/.cargo/git"],
        marker: None,
        kind: "Cargo download cache",
        verdict: Verdict::Safe,
        detail: "Downloaded crate sources and the registry index. Re-fetched on the next build. (The cargo-cache tool can trim it while keeping what's in use.)",
        command: Some("cargo cache --autoclean    # needs: cargo install cargo-cache"),
        root: false,
    },
    Tool {
        needles: &["/.cache/go-build"],
        marker: None,
        kind: "Go build cache",
        verdict: Verdict::Safe,
        detail: "Go's compiled build cache. Rebuilt automatically on the next `go build`.",
        command: Some("go clean -cache"),
        root: false,
    },
    Tool {
        needles: &["/go/pkg/mod", "/.cache/go/pkg/mod"],
        marker: None,
        kind: "Go module cache",
        verdict: Verdict::Safe,
        detail: "Downloaded Go modules. Re-fetched on the next build. The cache is read-only, so use Go's own command to remove it.",
        command: Some("go clean -modcache"),
        root: false,
    },
    Tool {
        needles: &["/.gradle/caches", "/.gradle"],
        marker: None,
        kind: "Gradle cache",
        verdict: Verdict::Safe,
        detail: "Gradle's downloaded dependencies and build cache. Re-downloaded when needed — stop the Gradle daemon first if one is running.",
        command: Some("gradle --stop"),
        root: false,
    },
    Tool {
        needles: &["/.m2/repository"],
        marker: None,
        kind: "Maven repository cache",
        verdict: Verdict::Safe,
        detail: "Maven's downloaded dependencies. Re-downloaded on the next build.",
        command: None,
        root: false,
    },
    Tool {
        needles: &["/.nuget/packages", "/.cache/nuget"],
        marker: None,
        kind: ".NET NuGet cache",
        verdict: Verdict::Safe,
        detail: "NuGet's global package cache. Cleared with the dotnet CLI.",
        command: Some("dotnet nuget locals all --clear"),
        root: false,
    },
    Tool {
        needles: &["/.ccache", "/.cache/ccache"],
        marker: None,
        kind: "Compiler cache (ccache)",
        verdict: Verdict::Safe,
        detail: "Cached C/C++ compiler output. Rebuilt as you compile; clear it with ccache itself.",
        command: Some("ccache -C"),
        root: false,
    },
    Tool {
        needles: &["/.cache/sccache"],
        marker: None,
        kind: "Compiler cache (sccache)",
        verdict: Verdict::Safe,
        detail: "Cached compiler output (sccache). Rebuilt as you compile.",
        command: None,
        root: false,
    },
    Tool {
        needles: &["/.gem", "/.cache/gem"],
        marker: None,
        kind: "RubyGems cache",
        verdict: Verdict::Safe,
        detail: "Cached Ruby gems. Re-downloaded as needed; gem's own cleanup removes old versions.",
        command: Some("gem cleanup"),
        root: false,
    },
    Tool {
        needles: &["/.cache/composer", "/.composer/cache"],
        marker: None,
        kind: "Composer cache",
        verdict: Verdict::Safe,
        detail: "PHP Composer's download cache. Re-downloaded on the next install.",
        command: Some("composer clear-cache"),
        root: false,
    },
    // ---- Language version managers: installed toolchains, not caches. ----
    Tool {
        needles: &["/.rustup/toolchains", "/.rustup"],
        marker: None,
        kind: "Rust toolchains (rustup)",
        verdict: Verdict::Caution,
        detail: "Installed Rust toolchains and components — not a cache. Remove specific ones with rustup so it stays consistent.",
        command: Some("rustup toolchain list    # then: rustup toolchain uninstall <name>"),
        root: false,
    },
    Tool {
        needles: &["/.nvm/versions", "/.nvm"],
        marker: None,
        kind: "Node versions (nvm)",
        verdict: Verdict::Caution,
        detail: "Node.js versions installed by nvm. Remove versions you no longer use via nvm.",
        command: Some("nvm uninstall <version>"),
        root: false,
    },
    Tool {
        needles: &["/.pyenv/versions", "/.pyenv"],
        marker: None,
        kind: "Python versions (pyenv)",
        verdict: Verdict::Caution,
        detail: "Python versions installed by pyenv. Remove ones you don't use via pyenv.",
        command: Some("pyenv uninstall <version>"),
        root: false,
    },
    Tool {
        needles: &["/.asdf/installs", "/.asdf"],
        marker: None,
        kind: "asdf toolchains",
        verdict: Verdict::Caution,
        detail: "Tool versions installed by asdf. Remove ones you don't use via asdf.",
        command: Some("asdf uninstall <tool> <version>"),
        root: false,
    },
    Tool {
        needles: &["/.sdkman"],
        marker: None,
        kind: "SDKMAN toolchains",
        verdict: Verdict::Caution,
        detail: "JVM SDKs installed by SDKMAN. Remove ones you don't use via sdk.",
        command: Some("sdk uninstall <candidate> <version>"),
        root: false,
    },
    // ---- Browsers ----
    Tool {
        needles: &[
            "/.cache/mozilla",
            "/.cache/google-chrome",
            "/.cache/chromium",
            "/.cache/bravesoftware",
            "/.cache/vivaldi",
            "/.cache/microsoft-edge",
            "/.cache/opera",
        ],
        marker: None,
        kind: "Browser cache",
        verdict: Verdict::Safe,
        detail: "Cached web pages and assets. Rebuilt as you browse — clearing this won't log you out or remove bookmarks.",
        command: None,
        root: false,
    },
    Tool {
        needles: &[
            "/.mozilla/firefox",
            "/.config/google-chrome",
            "/.config/chromium",
            "/.config/bravesoftware",
            "/.config/microsoft-edge",
            "/.config/vivaldi",
        ],
        marker: None,
        kind: "Browser profile & data",
        verdict: Verdict::Keep,
        detail: "Your browser profile: history, bookmarks, saved passwords, cookies, and extensions. Deleting it wipes all of that — clear browsing data from inside the browser instead.",
        command: None,
        root: false,
    },
    // ---- Flatpak / per-user app data ----
    Tool {
        needles: &["/.var/app/"],
        marker: None,
        kind: "Flatpak app data",
        verdict: Verdict::Caution,
        detail: "A Flatpak app's per-user data (settings, saves, and its own caches). Deleting loses that app's state — uninstalling the app is the clean way to remove it.",
        command: Some("flatpak uninstall <app-id>"),
        root: false,
    },
    Tool {
        needles: &["/.local/share/flatpak"],
        marker: None,
        kind: "Flatpak apps & runtimes (user)",
        verdict: Verdict::Caution,
        detail: "Per-user Flatpak apps and runtimes. Remove unused runtimes and apps via flatpak.",
        command: Some("flatpak uninstall --unused"),
        root: false,
    },
    // ---- Trash / thumbnails / generic cache (most generic last) ----
    Tool {
        needles: &["/.local/share/trash", "/.trash"],
        marker: None,
        kind: "Trash",
        verdict: Verdict::Safe,
        detail: "Files already moved to the trash. Emptying it frees the space for good — these won't be recoverable afterward.",
        command: Some("gio trash --empty"),
        root: false,
    },
    Tool {
        needles: &["/.cache/thumbnails", "/.thumbnails"],
        marker: None,
        kind: "Thumbnail cache",
        verdict: Verdict::Safe,
        detail: "Cached image and video thumbnails. Regenerated automatically as you browse folders.",
        command: None,
        root: false,
    },
    Tool {
        needles: &["/.cache"],
        marker: None,
        kind: "Application cache",
        verdict: Verdict::Safe,
        detail: "Per-user cache shared by many apps. They rebuild whatever they need, so clearing it is safe and frees space.",
        command: None,
        root: false,
    },
];

/// First catalog entry that recognizes this node, if any.
fn recognize(tree: &Tree, id: NodeId, path_lower: &str) -> Option<&'static Tool> {
    CATALOG.iter().find(|t| {
        t.needles.iter().any(|n| path_lower.contains(n))
            || t.marker.is_some_and(|m| has_child(tree, id, m))
    })
}

/// A system package manager we can detect and recommend a clean command for, so
/// the report's advice fits the user's actual distro rather than assuming one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PkgManager {
    Pacman,
    Apt,
    Dnf,
    Zypper,
    Apk,
    Xbps,
    Portage,
    Nix,
}

impl PkgManager {
    fn binary(self) -> &'static str {
        match self {
            PkgManager::Pacman => "pacman",
            PkgManager::Apt => "apt",
            PkgManager::Dnf => "dnf",
            PkgManager::Zypper => "zypper",
            PkgManager::Apk => "apk",
            PkgManager::Xbps => "xbps-install",
            PkgManager::Portage => "emerge",
            PkgManager::Nix => "nix-env",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PkgManager::Pacman => "pacman",
            PkgManager::Apt => "apt",
            PkgManager::Dnf => "dnf",
            PkgManager::Zypper => "zypper",
            PkgManager::Apk => "apk",
            PkgManager::Xbps => "xbps",
            PkgManager::Portage => "portage",
            PkgManager::Nix => "nix",
        }
    }

    /// The command that frees package-cache space cleanly.
    fn clean_cmd(self) -> &'static str {
        match self {
            PkgManager::Pacman => {
                "sudo pacman -Sc        # or, keeping recent versions: sudo paccache -r"
            }
            PkgManager::Apt => "sudo apt clean",
            PkgManager::Dnf => "sudo dnf clean all",
            PkgManager::Zypper => "sudo zypper clean --all",
            PkgManager::Apk => "sudo apk cache clean",
            PkgManager::Xbps => "sudo xbps-remove -O",
            PkgManager::Portage => "sudo eclean-dist        # from gentoolkit",
            PkgManager::Nix => "nix-collect-garbage -d",
        }
    }
}

const PKG_BIN_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin", "/sbin", "/usr/sbin"];
const ALL_PKG_MANAGERS: &[PkgManager] = &[
    PkgManager::Pacman,
    PkgManager::Apt,
    PkgManager::Dnf,
    PkgManager::Zypper,
    PkgManager::Apk,
    PkgManager::Xbps,
    PkgManager::Portage,
    PkgManager::Nix,
];

/// Detect installed system package managers by probing for their binaries.
/// A one-time handful of `stat`s — call once at startup and reuse the result;
/// it is intentionally *not* called from [`assess`], which stays fs-free.
pub fn detect_package_managers() -> Vec<PkgManager> {
    ALL_PKG_MANAGERS
        .iter()
        .copied()
        .filter(|pm| {
            PKG_BIN_DIRS
                .iter()
                .any(|d| Path::new(d).join(pm.binary()).exists())
        })
        .collect()
}

/// A human label for the detected managers, e.g. "pacman" or "apt, nix".
fn pm_labels(pkgs: &[PkgManager]) -> String {
    pkgs.iter()
        .map(|p| p.label())
        .collect::<Vec<_>>()
        .join(", ")
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

/// Analyze whether `id` is safe to delete. Pure, in-memory, bounded. `pkgs` is
/// the once-detected list of system package managers (see
/// [`detect_package_managers`]) used to tailor system-cache cleanup advice.
pub fn assess(tree: &Tree, id: NodeId, home: &Path, pkgs: &[PkgManager]) -> Assessment {
    let node = tree.node(id);
    let path = tree.path(id);
    // Use the path's basename, not the raw node name: the scanner stores the
    // full scanned path as the *root* node's name, which would defeat the
    // name-based rules below if the root itself is ever assessed.
    let lower = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
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

    // --- Knowledge catalog: a recognized tool/cache/data directory. This is
    // where most of the "what is this and how should I clear it" intelligence
    // lives — it can attach a recommended command and the right caveats.
    let path_lower = path.to_string_lossy().to_ascii_lowercase();
    if let Some(t) = recognize(tree, id, &path_lower) {
        if t.root {
            points.push(Point::bad(
                "System-managed — clearing it usually needs root",
            ));
        }
        if t.command.is_some() {
            points.push(Point::good("Has a dedicated cleanup command (below)"));
        }
        let mut report = finish(t.verdict, t.kind, t.detail, points);
        report.command = t.command.map(str::to_owned);
        return report;
    }

    // --- Caches / build output / temp: the space is reclaimable. This is
    // checked *before* the bare system-location case below, because a
    // system-managed cache (e.g. /var/cache) is still a cache — but it should
    // be cleared through the package manager, not deleted by hand, so it gets
    // its own framing rather than being mislabeled "installed software".
    if category == Category::Junk {
        if class == Class::System {
            points.push(Point::good(
                "Cache / regenerated data — the space is reclaimable",
            ));
            points.push(Point::bad(
                "System-managed — clearing it usually needs root",
            ));
            // Tailor the advice to whatever package manager is actually
            // installed, so the guidance is right on any distro.
            if let Some(pm) = pkgs.first().copied() {
                points.push(Point::good(format!(
                    "Your package manager: {}",
                    pm_labels(pkgs)
                )));
                let mut report = finish(
                    Verdict::Likely,
                    "System cache",
                    "Cache the system rebuilds on demand, so the space is reclaimable. It lives in a system location, so clear it with your package manager rather than deleting the files by hand.",
                    points,
                );
                report.command = Some(pm.clean_cmd().to_owned());
                return report;
            }
            return finish(
                Verdict::Likely,
                "System cache",
                "Cache the system rebuilds on demand, so the space is reclaimable — but it sits in a system location and is best cleared through the owning tool (e.g. your package manager) rather than by hand.",
                points,
            );
        }
        let (headline, detail) = junk_kind(&lower);
        points.push(Point::good(
            "Regenerated automatically when it's next needed",
        ));
        return finish(Verdict::Safe, headline, detail, points);
    }

    // --- Other system files: the OS itself or installed software.
    if class == Class::System {
        points.push(Point::bad("Part of the OS or an installed program"));
        return finish(
            Verdict::Caution,
            "System / installed software",
            "Likely belongs to installed software. Remove programs through your package manager rather than deleting files here.",
            points,
        );
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
                    points.push(Point::good(format!(
                        "{pct}% caches / build artifacts inside"
                    )));
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
        command: None,
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
    let (idx, &best) = totals.iter().enumerate().max_by_key(|(_, &v)| v).unwrap();
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
        assess(&tree, id, Path::new(NOWHERE), &[]).verdict
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

    // The first tree node whose path ends with `suffix` (so we can target a
    // nested directory, not just a direct child of the scan root).
    fn find(tree: &Tree, suffix: &str) -> NodeId {
        (0..tree.len() as u32)
            .find(|&id| tree.path(id).to_string_lossy().ends_with(suffix))
            .expect("a node with that path suffix should exist")
    }

    fn report_at(make: impl FnOnce(&std::path::Path), suffix: &str) -> Assessment {
        let dir = tempdir().unwrap();
        make(dir.path());
        let tree = scan(dir.path()).unwrap();
        let id = find(&tree, suffix);
        assess(&tree, id, Path::new(NOWHERE), &[])
    }

    #[test]
    fn npm_cache_recommends_command() {
        let a = report_at(
            |root| {
                let c = root.join(".npm").join("_cacache");
                fs::create_dir_all(&c).unwrap();
                fs::write(c.join("blob"), b"x").unwrap();
            },
            "/.npm",
        );
        assert_eq!(a.verdict, Verdict::Safe);
        assert!(a.command.as_deref().unwrap().contains("npm cache clean"));
    }

    #[test]
    fn pip_cache_recommends_command() {
        let a = report_at(
            |root| {
                let c = root.join(".cache").join("pip");
                fs::create_dir_all(&c).unwrap();
                fs::write(c.join("wheel"), b"x").unwrap();
            },
            "/.cache/pip",
        );
        assert_eq!(a.verdict, Verdict::Safe);
        assert!(a.command.as_deref().unwrap().contains("pip cache purge"));
    }

    #[test]
    fn go_build_cache_recommends_command() {
        let a = report_at(
            |root| {
                let c = root.join(".cache").join("go-build");
                fs::create_dir_all(&c).unwrap();
                fs::write(c.join("ab"), b"x").unwrap();
            },
            "/.cache/go-build",
        );
        assert_eq!(a.verdict, Verdict::Safe);
        assert!(a.command.as_deref().unwrap().contains("go clean -cache"));
    }

    #[test]
    fn virtualenv_recognized_by_marker() {
        let a = report_at(
            |root| {
                let v = root.join("myenv");
                fs::create_dir_all(v.join("bin")).unwrap();
                fs::write(v.join("pyvenv.cfg"), b"home = /usr").unwrap();
            },
            "/myenv",
        );
        assert_eq!(a.verdict, Verdict::Likely);
        assert!(a.command.as_deref().unwrap().contains("venv"));
    }

    #[test]
    fn browser_cache_is_safe_but_profile_is_kept() {
        let cache = report_at(
            |root| {
                let c = root.join(".cache").join("mozilla");
                fs::create_dir_all(&c).unwrap();
                fs::write(c.join("c"), b"x").unwrap();
            },
            "/.cache/mozilla",
        );
        assert_eq!(cache.verdict, Verdict::Safe);

        let profile = report_at(
            |root| {
                let p = root.join(".mozilla").join("firefox").join("ab.default");
                fs::create_dir_all(&p).unwrap();
                fs::write(p.join("places.sqlite"), b"x").unwrap();
            },
            "/.mozilla/firefox/ab.default",
        );
        assert_eq!(profile.verdict, Verdict::Keep);
    }

    #[test]
    fn rustup_toolchains_warn() {
        let a = report_at(
            |root| {
                let c = root.join(".rustup").join("toolchains").join("stable");
                fs::create_dir_all(&c).unwrap();
                fs::write(c.join("marker"), b"x").unwrap();
            },
            "/.rustup/toolchains/stable",
        );
        assert_eq!(a.verdict, Verdict::Caution);
        assert!(a.command.as_deref().unwrap().contains("rustup toolchain"));
    }

    #[test]
    fn every_package_manager_has_metadata() {
        for pm in ALL_PKG_MANAGERS {
            assert!(!pm.binary().is_empty());
            assert!(!pm.label().is_empty());
            assert!(!pm.clean_cmd().is_empty());
        }
    }

    #[test]
    fn detection_returns_only_known_managers() {
        // Environment-dependent, but must never panic and only yield known kinds.
        for pm in detect_package_managers() {
            assert!(ALL_PKG_MANAGERS.contains(&pm));
        }
    }

    #[test]
    fn system_cache_uses_detected_package_manager() {
        // Simulate the system-cache branch's command/labels selection.
        let pkgs = [PkgManager::Apt, PkgManager::Nix];
        assert_eq!(pm_labels(&pkgs), "apt, nix");
        assert_eq!(pkgs.first().copied(), Some(PkgManager::Apt));
        assert!(PkgManager::Apt.clean_cmd().contains("apt clean"));
    }
}
