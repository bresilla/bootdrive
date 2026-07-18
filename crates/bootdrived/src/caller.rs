//! Resolving the invoking user.
//!
//! The helper runs as root (launched through `pkexec`), so it must not trust a
//! path blindly. `pkexec` exports the original user's uid as `PKEXEC_UID`; we
//! use it only to double-check that the user who launched BootDrive can
//! actually read the file they selected. Authorization itself is handled by
//! polkit at `pkexec` time — there is no in-process access control.

use crate::image::Caller;

/// Resolve the invoking [`Caller`] from `pkexec`'s `PKEXEC_UID`, falling back to
/// root (uid 0) when it is absent (e.g. run directly under sudo or in tests).
pub fn from_pkexec_env() -> Caller {
    match std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Some(uid) => from_uid(uid),
        None => Caller {
            uid: 0,
            gids: vec![0],
        },
    }
}

/// Resolve a caller's primary and supplementary gids from the system databases
/// given only their uid.
pub fn from_uid(uid: u32) -> Caller {
    let mut gids = Vec::new();
    let mut username = None;

    if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 4 {
                continue;
            }
            if f[2].trim().parse::<u32>().ok() == Some(uid) {
                username = Some(f[0].to_string());
                if let Ok(gid) = f[3].trim().parse::<u32>() {
                    gids.push(gid);
                }
                break;
            }
        }
    }

    if let (Some(name), Ok(group)) = (&username, std::fs::read_to_string("/etc/group")) {
        for line in group.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 4 {
                continue;
            }
            if f[3].split(',').any(|m| m == name) {
                if let Ok(gid) = f[2].trim().parse::<u32>() {
                    if !gids.contains(&gid) {
                        gids.push(gid);
                    }
                }
            }
        }
    }

    Caller { uid, gids }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_falls_back_to_root() {
        // In the test process PKEXEC_UID is normally unset.
        std::env::remove_var("PKEXEC_UID");
        let c = from_pkexec_env();
        assert_eq!(c.uid, 0);
    }
}
