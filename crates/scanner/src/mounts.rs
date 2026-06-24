//! Resolving the set of devices that make up one physical filesystem.
//!
//! On btrfs a single filesystem is usually mounted as several subvolumes
//! (`/`, `/home`, `/var`, …), each reporting a *different* `st_dev`. A plain
//! single-device walk of `/` therefore misses everything in the other
//! subvolumes. [`same_filesystem_devices`] reads `/proc/self/mountinfo`, finds
//! the source device backing `root`, and returns every `st_dev` whose mount
//! shares that source — so the walk can cover the whole physical filesystem
//! (but still stop at genuinely different drives).

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use rustc_hash::FxHashSet;

/// Every `st_dev` reachable from `root` that belongs to the same source device
/// (the root's filesystem plus its sibling subvolumes / bind mounts). Falls back
/// to just the root's own device if the mount table can't be read.
pub fn same_filesystem_devices(root: &Path) -> FxHashSet<u64> {
    let mut allowed = FxHashSet::default();
    if let Ok(m) = fs::symlink_metadata(root) {
        allowed.insert(m.dev());
    }

    let Ok(content) = fs::read_to_string("/proc/self/mountinfo") else {
        return allowed;
    };
    let canon = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

    // (mount point, source device) for every mount.
    let mounts: Vec<(PathBuf, String)> = content.lines().filter_map(parse_line).collect();

    // The filesystem `root` lives on is the mount with the longest mount point
    // that is still a prefix of `root`.
    let root_source = mounts
        .iter()
        .filter(|(mp, _)| canon.starts_with(mp))
        .max_by_key(|(mp, _)| mp.as_os_str().len())
        .map(|(_, src)| src.clone());

    if let Some(source) = root_source {
        for (mount_point, src) in &mounts {
            if *src == source {
                if let Ok(m) = fs::symlink_metadata(mount_point) {
                    allowed.insert(m.dev());
                }
            }
        }
    }

    allowed
}

/// Parse one `/proc/self/mountinfo` line into `(mount_point, source_device)`.
///
/// Format: `id pid major:minor root mount_point opts… - fstype source superopts`.
fn parse_line(line: &str) -> Option<(PathBuf, String)> {
    let (before, after) = line.split_once(" - ")?;
    let mount_point = before.split_whitespace().nth(4)?; // 5th field
    let source = after.split_whitespace().nth(1)?; // fstype, then source
    Some((unescape(mount_point), source.to_owned()))
}

/// Undo the octal escaping `mountinfo` applies to space/tab/newline/backslash.
///
/// Works entirely on bytes — slicing the `&str` by byte offset would panic when
/// a backslash sits next to a multibyte UTF-8 sequence.
fn unescape(s: &str) -> PathBuf {
    if !s.contains('\\') {
        return PathBuf::from(s);
    }
    let bytes = s.as_bytes();
    let is_octal = |b: u8| (b'0'..=b'7').contains(&b);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && is_octal(bytes[i + 1])
            && is_octal(bytes[i + 2])
            && is_octal(bytes[i + 3])
        {
            let value = (bytes[i + 1] - b'0') as u16 * 64
                + (bytes[i + 2] - b'0') as u16 * 8
                + (bytes[i + 3] - b'0') as u16;
            if value <= 255 {
                out.push(value as u8);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(OsString::from_vec(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_octal_and_utf8_safe() {
        assert_eq!(unescape("/mnt/My\\040Drive"), PathBuf::from("/mnt/My Drive"));
        assert_eq!(unescape("/plain/path"), PathBuf::from("/plain/path"));
        // A backslash adjacent to multibyte UTF-8 must not panic.
        let _ = unescape("/mnt/\\é");
        let _ = unescape("/a\\");
    }
}
