//! File operations: move to trash (reversible, the default) or delete
//! permanently, plus "reveal in file manager". Path-based and UI-agnostic.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use scanner::safety::{classify, Class};

/// What a batch operation does to each target.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Move to the XDG trash (reversible).
    Trash,
    /// Delete permanently, skipping the trash.
    Delete,
}

/// The outcome of a batch operation.
pub struct Report {
    pub mode: Mode,
    pub succeeded: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    /// Subset of `failed` that failed because we lack permission — i.e. the item
    /// is owned by another user (usually root). These can be retried with
    /// elevated privileges.
    pub denied: Vec<PathBuf>,
    pub freed: u64,
}

impl Report {
    /// A one-line summary for the status bar.
    pub fn summary(&self) -> String {
        let verb = match self.mode {
            Mode::Trash => "Trashed",
            Mode::Delete => "Deleted",
        };
        if self.failed.is_empty() {
            format!(
                "{verb} {} item(s) · {} freed",
                self.succeeded.len(),
                crate::theme::format_size(self.freed)
            )
        } else {
            format!(
                "{verb} {} item(s) · {} failed ({})",
                self.succeeded.len(),
                self.failed.len(),
                self.failed
                    .first()
                    .map(|(_, e)| e.as_str())
                    .unwrap_or_default()
            )
        }
    }
}

/// Run `mode` on each `(path, on-disk size)` target.
pub fn perform(targets: &[(PathBuf, u64)], mode: Mode) -> Report {
    let mut report = Report {
        mode,
        succeeded: Vec::new(),
        failed: Vec::new(),
        denied: Vec::new(),
        freed: 0,
    };
    for (path, size) in targets {
        let outcome = match mode {
            Mode::Trash => trash::delete(path).map_err(reason_for_trash),
            Mode::Delete => remove(path).map_err(reason_for_io),
        };
        match outcome {
            Ok(()) => {
                report.succeeded.push(path.clone());
                report.freed += size;
            }
            Err((message, denied)) => {
                if denied {
                    report.denied.push(path.clone());
                }
                report.failed.push((path.clone(), message));
            }
        }
    }
    report
}

/// Permanently remove a path. A symlink is removed as a link (never followed).
fn remove(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Turn an I/O error into `(human message, is-permission-denied)`.
fn reason_for_io(e: std::io::Error) -> (String, bool) {
    match e.kind() {
        ErrorKind::PermissionDenied => ("Permission denied".to_owned(), true),
        ErrorKind::NotFound => ("No longer exists".to_owned(), false),
        _ => (e.to_string(), false),
    }
}

/// Turn a `trash` error into `(human message, is-permission-denied)`. On Linux,
/// moving an item owned by another user fails inside the filesystem step, which
/// we surface as a permission problem so the caller can offer to elevate.
fn reason_for_trash(e: trash::Error) -> (String, bool) {
    match &e {
        trash::Error::FileSystem { source, .. } if source.kind() == ErrorKind::PermissionDenied => {
            ("Permission denied".to_owned(), true)
        }
        // EACCES (13) / EPERM (1) surfaced directly by the platform.
        trash::Error::Os { code, .. } if *code == 13 || *code == 1 => {
            ("Permission denied".to_owned(), true)
        }
        trash::Error::TargetedRoot => ("Can't trash a filesystem root".to_owned(), false),
        _ => ("Couldn't move it to the trash".to_owned(), false),
    }
}

/// Open the system file manager at `path` (or its parent, for a file). Does
/// nothing if the target no longer exists (e.g. the path was just deleted).
pub fn reveal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    if target.exists() {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

/// Open `path` itself in the system's default application (e.g. a video in the
/// default media player). Does nothing if the file no longer exists.
pub fn open_external(path: &Path) {
    if path.exists() {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Permanently delete `paths` with elevated privileges, prompting for
/// authorization through the desktop's polkit agent (`pkexec`). Returns as soon
/// as the prompt has been launched — the deletion then runs as root in the
/// background — so callers should rescan afterwards. This is only ever reached
/// for items the user already tried and failed to delete as themselves.
pub fn spawn_elevated_delete(paths: &[PathBuf]) -> Result<(), String> {
    let argv = elevated_delete_argv(paths)?;
    std::process::Command::new("pkexec")
        .args(&argv)
        .spawn()
        .map(|_| ())
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => {
                "pkexec isn't available — install polkit, or run the copied command in a terminal."
                    .to_owned()
            }
            _ => e.to_string(),
        })
}

/// Build the argument vector handed to `pkexec` (`rm -rf -- <paths>`), refusing
/// empty input and any path that is relative or a protected system root. Keeping
/// this pure and guarded makes the one destructive line easy to review and test.
fn elevated_delete_argv(paths: &[PathBuf]) -> Result<Vec<OsString>, String> {
    if paths.is_empty() {
        return Err("Nothing to delete".to_owned());
    }
    for p in paths {
        if !p.is_absolute() {
            return Err(format!("Refusing a non-absolute path: {}", p.display()));
        }
        if classify(p, Path::new("")) == Class::Critical {
            return Err(format!(
                "Refusing a protected system location: {}",
                p.display()
            ));
        }
    }
    let mut argv: Vec<OsString> = vec!["rm".into(), "-rf".into(), "--".into()];
    argv.extend(paths.iter().map(|p| p.as_os_str().to_os_string()));
    Ok(argv)
}

/// The equivalent `sudo` one-liner, for users to copy when they'd rather run it
/// in a terminal (or have no polkit agent). Each path is single-quoted so spaces
/// and shell metacharacters are safe.
pub fn sudo_delete_command(paths: &[PathBuf]) -> String {
    let joined = paths
        .iter()
        .map(|p| sh_single_quote(&p.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    format!("sudo rm -rf -- {joined}")
}

/// Wrap `s` in single quotes for a POSIX shell, escaping any embedded quotes.
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn delete_removes_files_and_dirs_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f.bin");
        std::fs::write(&file, b"hi").unwrap();
        let dir = tmp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("inner.bin"), b"x").unwrap();

        let report = perform(&[(file.clone(), 2), (dir.clone(), 4096)], Mode::Delete);

        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(report.denied.is_empty());
        assert_eq!(report.succeeded.len(), 2);
        assert_eq!(report.freed, 4098);
        assert!(!file.exists());
        assert!(!dir.exists());
    }

    #[test]
    fn delete_symlink_never_follows_to_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.bin");
        std::fs::write(&target, b"important").unwrap();
        let link = tmp.path().join("link");
        symlink(&target, &link).unwrap();

        let report = perform(&[(link.clone(), 0)], Mode::Delete);

        assert_eq!(report.succeeded.len(), 1);
        assert!(!link.exists(), "the symlink itself should be gone");
        assert!(target.exists(), "the symlink target must NOT be deleted");
        assert_eq!(std::fs::read(&target).unwrap(), b"important");
    }

    #[test]
    fn delete_reports_failure_for_missing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        let report = perform(&[(missing, 0)], Mode::Delete);
        assert_eq!(report.succeeded.len(), 0);
        assert_eq!(report.failed.len(), 1);
        // A missing path is not a permission problem, so no elevation is offered.
        assert!(report.denied.is_empty());
    }

    #[test]
    fn elevated_argv_is_rm_rf_with_paths_after_a_separator() {
        let argv = elevated_delete_argv(&[PathBuf::from("/var/cache/x"), PathBuf::from("/opt/y")])
            .unwrap();
        assert_eq!(argv[0], "rm");
        assert_eq!(argv[1], "-rf");
        assert_eq!(
            argv[2], "--",
            "paths must follow `--` so none is read as a flag"
        );
        assert_eq!(argv[3], "/var/cache/x");
        assert_eq!(argv[4], "/opt/y");
    }

    #[test]
    fn elevated_argv_refuses_empty_relative_and_critical() {
        assert!(elevated_delete_argv(&[]).is_err());
        assert!(elevated_delete_argv(&[PathBuf::from("relative/path")]).is_err());
        assert!(elevated_delete_argv(&[PathBuf::from("/")]).is_err());
        assert!(elevated_delete_argv(&[PathBuf::from("/boot/grub")]).is_err());
    }

    #[test]
    fn sudo_command_quotes_each_path() {
        let cmd = sudo_delete_command(&[PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/c")]);
        assert_eq!(cmd, "sudo rm -rf -- '/tmp/a b' '/tmp/c'");
    }
}
