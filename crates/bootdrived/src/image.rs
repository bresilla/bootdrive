//! Selected-image validation.
//!
//! `bootdrived` runs as root, so it must not blindly open a path handed to it by
//! an unprivileged GUI. [`validate`] enforces the rules from the plan
//! (section 9): the path must resolve to a non-empty regular file that the
//! *calling* user can read, and while it is exposed it must not be writable by
//! untrusted users.

use std::path::{Path, PathBuf};

use bootdrive_common::{BootDriveError, ErrorCode};

/// A path that has passed every validation rule and is safe to expose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedImage {
    /// Canonicalized, absolute host path to the backing file.
    pub path: PathBuf,
    /// GUI-safe display name (never logged as a full path).
    pub display_name: String,
    /// Size of the file in bytes.
    pub size: u64,
}

/// File metadata needed for validation, abstracted so the rules can be
/// unit-tested without touching the filesystem.
#[derive(Debug, Clone, Copy)]
pub struct FileFacts {
    /// `true` if this is a regular file (not a dir, device, socket or FIFO).
    pub is_regular: bool,
    /// Size in bytes.
    pub size: u64,
    /// Owning uid.
    pub uid: u32,
    /// Owning gid.
    pub gid: u32,
    /// Permission bits (the low 12 bits of the mode).
    pub mode: u32,
}

/// The uid/gids of the D-Bus caller, resolved from bus credentials (never from
/// a client-supplied parameter).
#[derive(Debug, Clone)]
pub struct Caller {
    /// Caller uid.
    pub uid: u32,
    /// Supplementary + primary gids of the caller, best-effort.
    pub gids: Vec<u32>,
}

impl Caller {
    /// Whether the caller is a member of the given gid.
    fn in_group(&self, gid: u32) -> bool {
        self.gids.contains(&gid)
    }
}

/// Decide whether `caller` may read a file with `facts`, using standard POSIX
/// permission semantics (root always may).
pub fn caller_can_read(caller: &Caller, facts: &FileFacts) -> bool {
    if caller.uid == 0 {
        return true;
    }
    if caller.uid == facts.uid {
        return facts.mode & 0o400 != 0;
    }
    if caller.in_group(facts.gid) {
        return facts.mode & 0o040 != 0;
    }
    facts.mode & 0o004 != 0
}

/// Whether the file is writable by users we do not trust while it is exposed.
///
/// World-writable is always rejected. Group-writable is rejected unless the
/// group is `root` (gid 0), which we treat as an administrative group.
pub fn writable_by_untrusted(facts: &FileFacts) -> bool {
    if facts.mode & 0o002 != 0 {
        return true;
    }
    if facts.mode & 0o020 != 0 && facts.gid != 0 {
        return true;
    }
    false
}

/// Apply every content rule to already-gathered facts. Pure, hence testable.
pub fn check_facts(facts: &FileFacts, caller: &Caller) -> Result<(), BootDriveError> {
    if !facts.is_regular {
        return Err(BootDriveError::new(
            ErrorCode::InvalidImage,
            "the selected item is not a regular file",
        ));
    }
    if facts.size == 0 {
        return Err(BootDriveError::new(
            ErrorCode::InvalidImage,
            "the selected file is empty",
        ));
    }
    if !caller_can_read(caller, facts) {
        return Err(BootDriveError::new(
            ErrorCode::InvalidImage,
            "you do not have permission to read the selected file",
        ));
    }
    if writable_by_untrusted(facts) {
        return Err(BootDriveError::new(
            ErrorCode::InvalidImage,
            "the selected file is writable by other users and cannot be exposed safely",
        ));
    }
    Ok(())
}

/// A derived, GUI-safe display name: prefer the caller-supplied one, else the
/// file's basename. Never contains directory components.
pub fn safe_display_name(path: &Path, requested: &str) -> String {
    let requested = requested.trim();
    if !requested.is_empty() && !requested.contains('/') {
        return requested.to_string();
    }
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string())
}

/// Validate a selected path on a real filesystem.
///
/// The path is canonicalized first, then classified. All error messages are
/// GUI-safe and never echo the full path.
#[cfg(target_os = "linux")]
pub fn validate(
    raw_path: &Path,
    requested_name: &str,
    caller: &Caller,
) -> Result<ValidatedImage, BootDriveError> {
    use std::os::unix::fs::MetadataExt;

    let path = std::fs::canonicalize(raw_path).map_err(|_| {
        BootDriveError::new(
            ErrorCode::InvalidImage,
            "the selected file could not be found or is no longer accessible",
        )
    })?;

    let meta = std::fs::symlink_metadata(&path).map_err(|_| {
        BootDriveError::new(
            ErrorCode::InvalidImage,
            "the selected file could not be inspected",
        )
    })?;

    let facts = FileFacts {
        is_regular: meta.file_type().is_file(),
        size: meta.len(),
        uid: meta.uid(),
        gid: meta.gid(),
        mode: meta.mode() & 0o7777,
    };

    check_facts(&facts, caller)?;

    Ok(ValidatedImage {
        display_name: safe_display_name(&path, requested_name),
        path,
        size: facts.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(mode: u32, uid: u32, gid: u32) -> FileFacts {
        FileFacts {
            is_regular: true,
            size: 4096,
            uid,
            gid,
            mode,
        }
    }

    #[test]
    fn owner_read_bit_governs_owner() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        assert!(caller_can_read(&caller, &facts(0o600, 1000, 1000)));
        assert!(!caller_can_read(&caller, &facts(0o066, 1000, 1000)));
    }

    #[test]
    fn group_membership_grants_read() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000, 27],
        };
        assert!(caller_can_read(&caller, &facts(0o640, 0, 27)));
        let stranger = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        assert!(!caller_can_read(&stranger, &facts(0o640, 0, 27)));
    }

    #[test]
    fn other_read_is_last_resort() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        assert!(caller_can_read(&caller, &facts(0o604, 0, 0)));
        assert!(!caller_can_read(&caller, &facts(0o600, 0, 0)));
    }

    #[test]
    fn root_reads_anything() {
        let root = Caller {
            uid: 0,
            gids: vec![0],
        };
        assert!(caller_can_read(&root, &facts(0o000, 1000, 1000)));
    }

    #[test]
    fn world_writable_is_rejected() {
        assert!(writable_by_untrusted(&facts(0o666, 1000, 1000)));
        assert!(!writable_by_untrusted(&facts(0o644, 1000, 1000)));
    }

    #[test]
    fn nonroot_group_writable_is_rejected() {
        assert!(writable_by_untrusted(&facts(0o664, 1000, 1000)));
        assert!(!writable_by_untrusted(&facts(0o664, 1000, 0)));
    }

    #[test]
    fn empty_file_rejected() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        let mut f = facts(0o644, 1000, 1000);
        f.size = 0;
        let err = check_facts(&f, &caller).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidImage);
    }

    #[test]
    fn non_regular_rejected() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        let mut f = facts(0o644, 1000, 1000);
        f.is_regular = false;
        assert!(check_facts(&f, &caller).is_err());
    }

    #[test]
    fn good_file_passes() {
        let caller = Caller {
            uid: 1000,
            gids: vec![1000],
        };
        assert!(check_facts(&facts(0o644, 1000, 1000), &caller).is_ok());
    }

    #[test]
    fn display_name_strips_paths() {
        assert_eq!(
            safe_display_name(Path::new("/x/debian.iso"), "../../etc/passwd"),
            "debian.iso"
        );
        assert_eq!(
            safe_display_name(Path::new("/x/debian.iso"), "Debian 13"),
            "Debian 13"
        );
    }
}
