//! A small persisted library of disk images the user has added.
//!
//! Stored as JSON under the app config dir (inside the Flatpak that is
//! `~/.var/app/net.bresilla.BootDrive/config/bootdrive/library.json`).

use std::path::{Path, PathBuf};

use bootdrive_common::ImageMode;
use serde::{Deserialize, Serialize};

/// One image in the library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageEntry {
    /// Absolute host path.
    pub path: String,
    /// Display name (basename by default).
    pub display_name: String,
    /// Size in bytes, if known.
    pub size: Option<u64>,
    /// Whether to expose as CD-ROM (true) or disk (false).
    pub cdrom: bool,
    /// Whether the mode may be toggled (hybrid `.iso`).
    pub hybrid: bool,
}

impl ImageEntry {
    /// Build an entry from a chosen path (+ optional size).
    pub fn from_path(path: PathBuf, size: Option<u64>) -> ImageEntry {
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".to_string());
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let cdrom = ImageMode::default_for_extension(&ext) == ImageMode::Cdrom;
        ImageEntry {
            path: path.to_string_lossy().into_owned(),
            display_name,
            size,
            cdrom,
            hybrid: ext == "iso",
        }
    }

    /// The exposure mode for this entry.
    pub fn mode(&self) -> ImageMode {
        if self.cdrom {
            ImageMode::Cdrom
        } else {
            ImageMode::Disk
        }
    }
}

/// The whole library, loaded from / saved to disk.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Library {
    /// Images, most-recently-added last.
    pub entries: Vec<ImageEntry>,
}

fn library_path() -> PathBuf {
    let mut p: PathBuf = glib::user_config_dir();
    p.push("bootdrive");
    p.push("library.json");
    p
}

impl Library {
    /// Load the library, or an empty one if missing/corrupt.
    pub fn load() -> Library {
        match std::fs::read(library_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Library::default(),
        }
    }

    /// Persist the library (best-effort).
    pub fn save(&self) {
        let path = library_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Add an entry (de-duplicated by path; moves an existing one to the end).
    pub fn add(&mut self, entry: ImageEntry) {
        self.entries.retain(|e| e.path != entry.path);
        self.entries.push(entry);
        self.save();
    }

    /// Remove the entry at `index`.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
            self.save();
        }
    }

    /// Whether the entry's file still exists on disk.
    pub fn exists(entry: &ImageEntry) -> bool {
        Path::new(&entry.path).is_file()
    }
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
    fn entry_from_iso_is_cdrom_hybrid() {
        let e = ImageEntry::from_path(PathBuf::from("/x/ubuntu.iso"), Some(100));
        assert!(e.cdrom);
        assert!(e.hybrid);
        assert_eq!(e.display_name, "ubuntu.iso");
        assert_eq!(e.mode(), ImageMode::Cdrom);
    }

    #[test]
    fn entry_from_img_is_disk() {
        let e = ImageEntry::from_path(PathBuf::from("/x/disk.img"), None);
        assert!(!e.cdrom);
        assert!(!e.hybrid);
        assert_eq!(e.mode(), ImageMode::Disk);
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(4_000_000_000), "3.7 GB");
    }
}
