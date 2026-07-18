//! Turning a user's file choice into something the daemon can open.
//!
//! Inside a Flatpak the chosen file is exported through the XDG Documents
//! portal and appears under `/run/flatpak/doc/<id>/<name>` (or
//! `/run/user/<uid>/doc/<id>/<name>`). Those *are* real paths the daemon —
//! running outside the sandbox — cannot see. [`resolve_host_path`] maps them to
//! a path on the host when possible, following the plan's preferred/fallback
//! flow (section 8).

use std::path::{Path, PathBuf};

use bootdrive_common::ImageMode;

/// A selected image ready to hand to the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Host path the daemon should open.
    pub host_path: PathBuf,
    /// Display name (basename).
    pub display_name: String,
    /// Size in bytes, if known at selection time.
    pub size: Option<u64>,
    /// Default mode inferred from the extension.
    pub default_mode: ImageMode,
    /// Whether the file looks like a hybrid ISO (may also be exposed as a disk).
    pub hybrid: bool,
}

impl Selection {
    /// Build a selection from a resolved host path.
    pub fn from_host_path(path: PathBuf, size: Option<u64>) -> Selection {
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".to_string());
        let ext = extension_of(&path);
        Selection {
            default_mode: ImageMode::default_for_extension(&ext),
            hybrid: ext == "iso",
            display_name,
            host_path: path,
            size,
        }
    }
}

/// Lower-cased extension without the dot.
fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether we appear to be running inside a Flatpak sandbox.
pub fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Resolve a possibly-sandboxed path to a host path.
///
/// - A normal absolute path outside the portal is returned as-is.
/// - A Documents-portal path (`/run/flatpak/doc/<id>/<name>` or
///   `/run/user/<uid>/doc/<id>/<name>`) is rewritten to the host location
///   `~/.local/share/flatpak/... ` is **not** attempted; instead the daemon is
///   given the portal path's document id + name so it can reconstruct the host
///   path under `/run/user/<uid>/doc/<id>/`, which is visible system-wide.
///
/// For version 1 we pass the portal path through unchanged when it is already a
/// `/run/user/<uid>/doc/...` path (system-visible), and only strip the
/// per-app `/run/flatpak/doc` prefix, which is *not* visible outside the
/// sandbox, down to the shared form.
pub fn resolve_host_path(selected: &Path) -> PathBuf {
    let s = selected.to_string_lossy();

    // Per-app fuse mount, only visible inside this sandbox. Rewrite to the
    // shared documents mount which the daemon can also see.
    if let Some(rest) = s.strip_prefix("/run/flatpak/doc/") {
        // rest = "<doc-id>/<name>"
        if let Ok(uid) = real_uid() {
            return PathBuf::from(format!("/run/user/{uid}/doc/{rest}"));
        }
    }

    selected.to_path_buf()
}

/// The real uid of the GUI process (used to build the shared portal path).
fn real_uid() -> Result<u32, ()> {
    // Avoid a libc dependency: read from the environment/proc.
    if let Ok(s) = std::env::var("BOOTDRIVE_UID") {
        if let Ok(n) = s.parse() {
            return Ok(n);
        }
    }
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .or_else(|| {
            // Fallback: parse /proc/self/status Uid line.
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("Uid:"))
                        .and_then(|l| l.split_whitespace().next())
                        .and_then(|v| v.parse::<u32>().ok())
                })
        })
        .ok_or(())
}

/// A human-friendly size string.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_defaults() {
        let s = Selection::from_host_path(PathBuf::from("/x/debian.iso"), Some(4_000_000_000));
        assert_eq!(s.default_mode, ImageMode::Cdrom);
        assert!(s.hybrid);
        assert_eq!(s.display_name, "debian.iso");

        let s = Selection::from_host_path(PathBuf::from("/x/disk.img"), None);
        assert_eq!(s.default_mode, ImageMode::Disk);
        assert!(!s.hybrid);
    }

    #[test]
    fn plain_path_passes_through() {
        assert_eq!(
            resolve_host_path(Path::new("/home/u/a.iso")),
            PathBuf::from("/home/u/a.iso")
        );
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(4_000_000_000), "3.7 GB");
    }
}
