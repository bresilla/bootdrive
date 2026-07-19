//! Browsable list of downloadable install images, read from osinfo-db.
//!
//! osinfo-db is the OS database maintained by the libosinfo project (and used
//! by GNOME Boxes, virt-manager, Impression, ...). It ships as a tree of XML
//! files under `.../osinfo/os/`, one per OS release, each listing the install
//! media and their download URLs. We read those files directly instead of
//! linking libosinfo, group them by distro, and hand the result to the
//! Download tab. The Flatpak bundles the database at `/app/share/osinfo`; on a
//! normal system it lives at `/usr/share/osinfo`.

use std::path::{Path, PathBuf};

/// One distro with all of its downloadable images.
pub struct Distro {
    /// Name shown in the list (e.g. `Ubuntu`).
    pub name: String,
    /// Images, newest release first.
    pub images: Vec<Image>,
}

/// A single downloadable image (one arch of one media of one release).
#[derive(Clone)]
pub struct Image {
    /// Release name, e.g. `Ubuntu 24.04 LTS`.
    pub os_name: String,
    /// Media variant if the file names one, e.g. `Workstation`.
    pub variant: Option<String>,
    /// CPU architecture, e.g. `x86_64`.
    pub arch: String,
    /// True for a live/desktop image, false for an installer.
    pub live: bool,
    /// Direct download URL of the ISO.
    pub url: String,
    /// Parsed version, used only to sort newest first.
    version: f64,
}

/// Architectures worth offering for a PC-bootable USB stick. osinfo-db carries
/// plenty of s390x/ppc64/... media that would only clutter the list here.
const ARCHES: [&str; 2] = ["x86_64", "aarch64"];

/// Locate the osinfo-db `os/` directory, or `None` if the database is missing.
pub fn os_dir() -> Option<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("OSINFO_SYSTEM_DIR") {
        bases.push(PathBuf::from(dir));
    }
    bases.push(PathBuf::from("/app/share/osinfo"));
    bases.push(PathBuf::from("/usr/share/osinfo"));
    let mut local = glib::user_data_dir();
    local.push("osinfo");
    bases.push(local);

    bases.into_iter().map(|b| b.join("os")).find(|p| p.is_dir())
}

/// Load and group every downloadable image. Returns an empty list if osinfo-db
/// is not installed.
pub fn load() -> Vec<Distro> {
    let Some(dir) = os_dir() else {
        return Vec::new();
    };

    let mut files = Vec::new();
    collect_xml(&dir, &mut files);

    let mut by_key: std::collections::HashMap<String, Distro> = std::collections::HashMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(doc) = roxmltree::Document::parse(&text) else {
            continue;
        };
        for os in doc.descendants().filter(|n| n.has_tag_name("os")) {
            parse_os(os, &mut by_key);
        }
    }

    let mut distros: Vec<Distro> = by_key.into_values().collect();
    for d in &mut distros {
        d.images.sort_by(|a, b| b.version.total_cmp(&a.version));
    }
    distros.retain(|d| !d.images.is_empty());
    distros.sort_by_key(|a| a.name.to_lowercase());
    distros
}

/// Read one `<os>` element and append its media to the matching distro.
fn parse_os(os: roxmltree::Node, by_key: &mut std::collections::HashMap<String, Distro>) {
    let name = child_text(os, "name").unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let key = child_text(os, "distro")
        .or_else(|| child_text(os, "family"))
        .unwrap_or_else(|| name.clone());
    let version = child_text(os, "version")
        .and_then(|v| leading_number(&v))
        .unwrap_or(0.0);

    // Variant id -> display name, so a media's `<variant id=".."/>` becomes a
    // readable label.
    let variants: std::collections::HashMap<String, String> = os
        .children()
        .filter(|n| n.has_tag_name("variant"))
        .filter_map(|v| {
            let id = v.attribute("id")?;
            let vname = child_text(v, "name")?;
            Some((id.to_string(), vname))
        })
        .collect();

    for media in os.children().filter(|n| n.has_tag_name("media")) {
        let Some(arch) = media.attribute("arch") else {
            continue;
        };
        if !ARCHES.contains(&arch) {
            continue;
        }
        let Some(url) = child_text(media, "url") else {
            continue;
        };
        let live = media.attribute("live") == Some("true");
        let variant = media
            .children()
            .find(|n| n.has_tag_name("variant"))
            .and_then(|v| v.attribute("id"))
            .and_then(|id| variants.get(id).cloned())
            // A variant that just repeats the release name adds no information.
            .filter(|v| v != &name);

        let entry = by_key.entry(key.clone()).or_insert_with(|| Distro {
            name: pretty_distro(&key),
            images: Vec::new(),
        });
        entry.images.push(Image {
            os_name: name.clone(),
            variant,
            arch: arch.to_string(),
            live,
            url,
            version,
        });
    }
}

/// Walk `dir` and push every `.xml` file into `out`.
fn collect_xml(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_xml(&path, out);
        } else if path.extension().is_some_and(|e| e == "xml") {
            out.push(path);
        }
    }
}

fn child_text(node: roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Parse the numeric prefix of a version string (`24.04 LTS` -> 24.04).
fn leading_number(v: &str) -> Option<f64> {
    let end = v
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(v.len());
    v[..end].parse().ok()
}

/// Turn an osinfo distro key into something nicer to read.
fn pretty_distro(key: &str) -> String {
    match key {
        "rhel" => "Red Hat Enterprise Linux".to_string(),
        "opensuse" => "openSUSE".to_string(),
        "sled" | "sles" => key.to_uppercase(),
        "macosx" => "macOS".to_string(),
        _ => {
            let mut c = key.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => key.to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs osinfo-db installed; run with --ignored"]
    fn loads_local_db() {
        let distros = load();
        assert!(!distros.is_empty(), "no distros parsed");
        let total: usize = distros.iter().map(|d| d.images.len()).sum();
        println!("{} distros, {total} images", distros.len());
        for d in distros.iter().take(8) {
            let img = &d.images[0];
            println!(
                "  {} ({} images), e.g. {} {}",
                d.name,
                d.images.len(),
                img.os_name,
                img.arch
            );
        }
    }
}
