//! BootDrive's managed image library.
//!
//! Added images are **copied** into BootDrive's own data directory
//! (`…/data/bootdrive/images/`, i.e. inside the Flatpak
//! `~/.var/app/net.bresilla.BootDrive/data/bootdrive/images/`). That gives a
//! stable path the root usb-signaller can always read, and means the image
//! survives the original being moved or unplugged. The index is a JSON file
//! next to it.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use bootdrive_common::ImageMode;
use serde::{Deserialize, Serialize};

/// One image in the library (its path is BootDrive's local copy).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageEntry {
    /// Absolute path to BootDrive's copy.
    pub path: String,
    /// Display name.
    pub display_name: String,
    /// Size in bytes.
    pub size: Option<u64>,
    /// Expose as CD-ROM (true) or disk (false).
    pub cdrom: bool,
    /// Hybrid `.iso` (mode may be toggled).
    pub hybrid: bool,
}

impl ImageEntry {
    fn new(path: PathBuf, display_name: String, size: Option<u64>) -> ImageEntry {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        ImageEntry {
            cdrom: ImageMode::default_for_extension(&ext) == ImageMode::Cdrom,
            hybrid: ext == "iso",
            path: path.to_string_lossy().into_owned(),
            display_name,
            size,
        }
    }

    /// The exposure mode.
    pub fn mode(&self) -> ImageMode {
        if self.cdrom {
            ImageMode::Cdrom
        } else {
            ImageMode::Disk
        }
    }
}

/// The library index.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Library {
    /// Images, most-recently-added last.
    pub entries: Vec<ImageEntry>,
}

fn base_dir() -> PathBuf {
    let mut p = glib::user_data_dir();
    p.push("bootdrive");
    p
}

/// Where copied images live.
pub fn images_dir() -> PathBuf {
    base_dir().join("images")
}

fn index_path() -> PathBuf {
    base_dir().join("library.json")
}

/// Progress of an import: (copied bytes, total bytes).
pub type Progress = (u64, u64);

/// Copy `source` into the images dir, reporting progress and honouring
/// `cancel`. Returns the new entry (pointing at the copy). On cancel/error the
/// partial copy is removed.
pub fn import<F: FnMut(Progress)>(
    source: &Path,
    mut progress: F,
    cancel: &AtomicBool,
) -> std::io::Result<ImageEntry> {
    let dir = images_dir();
    std::fs::create_dir_all(&dir)?;

    let display_name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let dest = unique_dest(&dir, &display_name);

    let total = std::fs::metadata(source)?.len();
    let mut src = File::open(source)?;
    let mut dst = File::create(&dest)?;
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut copied = 0u64;
    progress((0, total));
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(dst);
            let _ = std::fs::remove_file(&dest);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Err(e) = dst.write_all(&buf[..n]) {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
        copied += n as u64;
        progress((copied, total));
    }
    dst.flush()?;
    Ok(ImageEntry::new(dest, display_name, Some(total)))
}

/// Pick a non-colliding destination path for `name` in `dir`.
fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

impl Library {
    /// Load the index (empty if missing/corrupt).
    pub fn load() -> Library {
        match std::fs::read(index_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Library::default(),
        }
    }

    /// Persist the index (best-effort).
    pub fn save(&self) {
        let path = index_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Add an entry (the caller has already copied the file in).
    pub fn add(&mut self, entry: ImageEntry) {
        self.entries.retain(|e| e.path != entry.path);
        self.entries.push(entry);
        self.save();
    }

    /// Remove the entry at `index` and delete its copied file.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            let entry = self.entries.remove(index);
            let _ = std::fs::remove_file(&entry.path);
            self.save();
        }
    }

    /// Whether the entry's file still exists.
    pub fn exists(entry: &ImageEntry) -> bool {
        Path::new(&entry.path).is_file()
    }

    /// Total bytes used by the library's images.
    pub fn total_size(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size).sum()
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
    fn new_entry_modes() {
        let iso = ImageEntry::new(PathBuf::from("/x/ubuntu.iso"), "ubuntu.iso".into(), Some(9));
        assert!(iso.cdrom && iso.hybrid);
        assert_eq!(iso.mode(), ImageMode::Cdrom);
        let img = ImageEntry::new(PathBuf::from("/x/d.img"), "d.img".into(), None);
        assert!(!img.cdrom && !img.hybrid);
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(4_000_000_000), "3.7 GB");
    }
}
