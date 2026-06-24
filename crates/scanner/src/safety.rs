//! Path safety classification.
//!
//! A first pass at the guardrails the UI will lean on in later phases: how much
//! caution a path warrants before a destructive operation. Phase 1 only needs
//! the classification primitive itself; the confirmation flows that consume it
//! arrive with file operations in Phase 5.

use std::path::{Path, PathBuf};

/// How much caution a path warrants before any destructive operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Inside the user's home directory: ordinary, trash by default.
    Normal,
    /// Outside home but not a known system location: warrants extra confirmation.
    OutsideHome,
    /// A system location (e.g. `/usr`, `/etc`): deletion is heavily guarded.
    System,
    /// Critical to a bootable system (e.g. `/`, `/boot`): never deleted by the app.
    Critical,
}

/// Roots the app must refuse to delete outright.
const CRITICAL: &[&str] = &["/", "/boot", "/efi", "/proc", "/sys", "/dev", "/run"];

/// System roots that require heavy confirmation.
const SYSTEM: &[&str] = &[
    "/usr", "/etc", "/bin", "/sbin", "/lib", "/lib64", "/var", "/opt", "/srv", "/root",
];

/// Classify `path` relative to `home` (the user's home directory). This is a
/// purely lexical decision and never touches the filesystem.
pub fn classify(path: &Path, home: &Path) -> Class {
    if CRITICAL.iter().any(|c| path == Path::new(c)) {
        return Class::Critical;
    }
    if path.starts_with(home) {
        return Class::Normal;
    }
    if SYSTEM.iter().any(|s| path.starts_with(s)) {
        return Class::System;
    }
    if CRITICAL.iter().any(|c| *c != "/" && path.starts_with(c)) {
        return Class::Critical;
    }
    Class::OutsideHome
}

/// Convenience wrapper that reads `$HOME` from the environment.
pub fn classify_with_env(path: &Path) -> Class {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    classify(path, &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_paths() {
        let home = Path::new("/home/me");
        assert_eq!(
            classify(Path::new("/home/me/Downloads"), home),
            Class::Normal
        );
        assert_eq!(classify(Path::new("/"), home), Class::Critical);
        assert_eq!(classify(Path::new("/boot/grub"), home), Class::Critical);
        assert_eq!(classify(Path::new("/usr/lib"), home), Class::System);
        assert_eq!(classify(Path::new("/mnt/data"), home), Class::OutsideHome);
    }
}
