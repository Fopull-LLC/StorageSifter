//! File operations: move to trash (reversible, the default) or delete
//! permanently, plus "reveal in file manager". Path-based and UI-agnostic.

use std::path::{Path, PathBuf};

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
        freed: 0,
    };
    for (path, size) in targets {
        let result = match mode {
            Mode::Trash => trash::delete(path).map_err(|e| e.to_string()),
            Mode::Delete => remove(path),
        };
        match result {
            Ok(()) => {
                report.succeeded.push(path.clone());
                report.freed += size;
            }
            Err(error) => report.failed.push((path.clone(), error)),
        }
    }
    report
}

/// Permanently remove a path. A symlink is removed as a link (never followed).
fn remove(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .map_err(|e| e.to_string())
}

/// Open the system file manager at `path` (or its parent, for a file).
pub fn reveal(path: &Path) {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
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
    }
}
