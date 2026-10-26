//! Update checks and in-app updates, from the GitHub releases of
//! `Fopull-LLC/StorageSifter` (the same feed fopull.com's download page reads).
//!
//! How an update is applied depends on how StorageSifter was installed:
//! an AppImage or a standalone binary (release tarball, `cargo install`) is
//! replaced in place after its SHA-256 is checked against the release's
//! `SHA256SUMS.txt`; a copy owned by a system package manager is left to that
//! package manager, and the user is shown the command to run.
//!
//! All network and disk work runs on background threads through `curl`, `tar`
//! and `sha256sum`, so nothing here ever blocks a frame.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use eframe::egui;
use serde::Deserialize;

const LATEST_URL: &str = "https://api.github.com/repos/Fopull-LLC/StorageSifter/releases/latest";
pub const RELEASES_URL: &str = "https://github.com/Fopull-LLC/StorageSifter/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How this copy was installed, which decides how it can be updated.
#[derive(Clone)]
pub enum Method {
    /// Running from an AppImage at this path (`$APPIMAGE`).
    AppImage(PathBuf),
    /// A standalone binary the user can write to, replaced from the tarball.
    Binary(PathBuf),
    /// Installed by a system package manager; it should do the update.
    Package { manager: &'static str, pkg: String },
    /// A development build or a read-only location: download page only.
    Manual,
}

impl Method {
    /// Whether "Update now" can install the new version itself.
    pub fn can_self_update(&self) -> bool {
        matches!(self, Method::AppImage(_) | Method::Binary(_))
    }

    /// For package-manager installs, the command that updates it.
    pub fn command(&self) -> Option<String> {
        let Method::Package { manager, pkg } = self else {
            return None;
        };
        Some(match *manager {
            "pacman" => format!("yay -Syu {pkg}"),
            "dpkg" => format!("sudo apt update && sudo apt install --only-upgrade {pkg}"),
            _ => format!("sudo dnf upgrade {pkg}"),
        })
    }
}

#[derive(Clone)]
pub struct Release {
    /// The release tag, e.g. `v0.4.0`.
    pub tag: String,
    /// Its page on GitHub (release notes).
    pub page: String,
    assets: Vec<GhAsset>,
}

#[derive(Clone)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing(Release),
    /// The new version is on disk; relaunching runs it.
    Installed(Release),
    Failed(String),
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Clone, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub struct Updater {
    pub method: Method,
    state: Arc<Mutex<State>>,
    ctx: egui::Context,
}

impl Updater {
    pub fn new(ctx: egui::Context) -> Self {
        Updater {
            method: detect_method(),
            state: Arc::new(Mutex::new(State::Idle)),
            ctx,
        }
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Idle)
    }

    fn set(state: &Mutex<State>, ctx: &egui::Context, new: State) {
        if let Ok(mut s) = state.lock() {
            *s = new;
        }
        ctx.request_repaint();
    }

    /// Ask GitHub for the latest release, in the background.
    pub fn check(&self) {
        if matches!(self.state(), State::Checking | State::Installing(_)) {
            return;
        }
        Self::set(&self.state, &self.ctx, State::Checking);
        let (state, ctx) = (self.state.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let next = match fetch_latest() {
                Ok(r) if is_newer(&r.tag, CURRENT) => State::Available(r),
                Ok(_) => State::UpToDate,
                Err(e) => State::Failed(e),
            };
            Self::set(&state, &ctx, next);
        });
    }

    /// Download, verify and install the available release, in the background.
    pub fn install(&self) {
        let State::Available(release) = self.state() else {
            return;
        };
        let method = self.method.clone();
        Self::set(&self.state, &self.ctx, State::Installing(release.clone()));
        let (state, ctx) = (self.state.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let next = match install(&method, &release) {
                Ok(()) => State::Installed(release),
                Err(e) => State::Failed(e),
            };
            Self::set(&state, &ctx, next);
        });
    }

    /// Start the freshly installed version and close this one.
    pub fn relaunch(&self, ctx: &egui::Context) -> Result<(), String> {
        let target = match &self.method {
            Method::AppImage(p) | Method::Binary(p) => p,
            _ => return Err("This copy can't be relaunched automatically.".into()),
        };
        Command::new(target)
            .args(std::env::args_os().skip(1))
            .spawn()
            .map_err(|e| format!("Couldn't start the new version: {e}"))?;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }
}

fn detect_method() -> Method {
    if let Some(p) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        if p.is_file() {
            return Method::AppImage(p);
        }
    }
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return Method::Manual;
    };
    if let Some((manager, pkg)) = owning_package(&exe) {
        return Method::Package { manager, pkg };
    }
    let dev_build = exe.components().any(|c| c.as_os_str() == "target");
    if dev_build || exe.starts_with("/usr") || exe.starts_with("/opt") {
        return Method::Manual;
    }
    Method::Binary(exe)
}

/// Which installed package owns `exe`, if a package manager knows it.
fn owning_package(exe: &Path) -> Option<(&'static str, String)> {
    let query = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = Command::new(cmd).args(args).arg(exe).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    if let Some(pkg) = query("pacman", &["-Qqo"]) {
        return Some(("pacman", pkg));
    }
    // `dpkg -S` prints "pkg[:arch]: /path".
    if let Some(line) = query("dpkg", &["-S"]) {
        let pkg = line.split(':').next()?.trim().to_owned();
        return Some(("dpkg", pkg));
    }
    query("rpm", &["-qf", "--qf", "%{NAME}"]).map(|pkg| ("rpm", pkg))
}

fn curl() -> Command {
    let mut c = Command::new("curl");
    c.args(["-fsSL", "-H"])
        .arg(format!("User-Agent: StorageSifter/{CURRENT}"));
    c
}

fn fetch_latest() -> Result<Release, String> {
    let out = curl()
        .args([
            "--max-time",
            "15",
            "-H",
            "Accept: application/vnd.github+json",
        ])
        .arg(LATEST_URL)
        .output()
        .map_err(|_| "Checking for updates needs curl installed.".to_owned())?;
    if !out.status.success() {
        return Err("Couldn't reach GitHub to check for updates.".into());
    }
    let gh: GhRelease = serde_json::from_slice(&out.stdout)
        .map_err(|_| "GitHub sent an unexpected reply.".to_owned())?;
    Ok(Release {
        tag: gh.tag_name,
        page: gh.html_url,
        assets: gh.assets,
    })
}

/// `"v1.2.3-rc1"` → `(1, 2, 3)`; pre-release/build suffixes are ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn install(method: &Method, release: &Release) -> Result<(), String> {
    let arch = std::env::consts::ARCH;
    let (target, suffix) = match method {
        Method::AppImage(p) => (p, format!("-{arch}.AppImage")),
        Method::Binary(p) => (p, format!("-{arch}-linux.tar.gz")),
        _ => return Err("This copy has to be updated outside the app.".into()),
    };
    let find = |pred: &dyn Fn(&str) -> bool| release.assets.iter().find(|a| pred(&a.name));
    let asset = find(&|n| n.ends_with(&suffix))
        .ok_or_else(|| format!("{} has no {arch} build to install.", release.tag))?;
    let sums = find(&|n| n == "SHA256SUMS.txt")
        .ok_or_else(|| format!("{} has no checksums to verify against.", release.tag))?;

    // Stage beside the target so the final rename stays on one filesystem
    // (atomic, and safe while the old binary is still running).
    let dir = target.parent().ok_or("Unknown install location.")?;
    let work = dir.join(format!(".storagesifter-update-{}", std::process::id()));
    std::fs::create_dir(&work).map_err(|e| format!("Can't write to {}: {e}", dir.display()))?;
    let result = stage_and_replace(&work, target, asset, sums, method);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn stage_and_replace(
    work: &Path,
    target: &Path,
    asset: &GhAsset,
    sums: &GhAsset,
    method: &Method,
) -> Result<(), String> {
    let download = work.join(&asset.name);
    let sums_file = work.join("SHA256SUMS.txt");
    fetch_to(&asset.browser_download_url, &download)?;
    fetch_to(&sums.browser_download_url, &sums_file)?;

    let listed = std::fs::read_to_string(&sums_file)
        .ok()
        .and_then(|s| expected_sum(&s, &asset.name))
        .ok_or("The release checksums don't list this download.")?;
    if sha256(&download)? != listed {
        return Err("The download didn't match its checksum, so it wasn't installed.".into());
    }

    let new = match method {
        Method::AppImage(_) => download,
        _ => {
            let ok = Command::new("tar")
                .arg("-xzf")
                .arg(&download)
                .arg("-C")
                .arg(work)
                .status()
                .is_ok_and(|s| s.success());
            let root = asset.name.trim_end_matches(".tar.gz");
            let bin = work.join(root).join("storagesifter");
            if !ok || !bin.is_file() {
                return Err("Couldn't unpack the download.".into());
            }
            bin
        }
    };

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
        .and_then(|()| std::fs::rename(&new, target))
        .map_err(|e| format!("Couldn't replace {}: {e}", target.display()))
}

fn fetch_to(url: &str, dest: &Path) -> Result<(), String> {
    let ok = curl()
        .args(["--max-time", "900", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(())
        .ok_or_else(|| "The download failed. Check your connection and try again.".into())
}

/// The hash `SHA256SUMS.txt` lists for `name` (`<hex>  <name>` lines).
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256(path: &Path) -> Result<String, String> {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|_| "Verifying the download needs sha256sum installed.".to_owned())?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "Couldn't verify the download.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v0.4.0", "0.3.0"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.5", "0.3.0"));
        assert!(!is_newer("nightly", "0.3.0"));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
    }

    #[test]
    fn finds_the_listed_checksum() {
        let sums = "abc123  storagesifter-v0.4.0-x86_64-linux.tar.gz\n\
                    DEF456  StorageSifter-v0.4.0-x86_64.AppImage\n";
        assert_eq!(
            expected_sum(sums, "StorageSifter-v0.4.0-x86_64.AppImage").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_sum(sums, "other"), None);
    }

    /// Live: installs the latest release over scratch files in
    /// `$STORAGESIFTER_TEST_DIR`. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_install_replaces_scratch_targets() {
        let dir = PathBuf::from(std::env::var("STORAGESIFTER_TEST_DIR").unwrap());
        let release = fetch_latest().unwrap();
        for (name, method) in [
            ("bin", Method::Binary(dir.join("storagesifter"))),
            (
                "appimage",
                Method::AppImage(dir.join("StorageSifter.AppImage")),
            ),
        ] {
            let (Method::Binary(target) | Method::AppImage(target)) = &method else {
                unreachable!()
            };
            std::fs::write(target, b"old").unwrap();
            install(&method, &release).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                std::fs::metadata(target).unwrap().len() > 1_000_000,
                "{name}"
            );
        }
    }
}
