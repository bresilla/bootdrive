//! Caller authorization.
//!
//! For the development version, access is gated on membership of a `bootdrive`
//! Unix group (mirrored by the D-Bus policy). Root is always allowed. The caller
//! identity is derived from bus credentials by the service layer and passed in
//! as a [`Caller`]; it is never accepted as a method parameter.
//!
//! A PolicyKit action (`net.bresilla.bootdrive.activate`) is the intended
//! long-term mechanism; the trait boundary lets it drop in later.

use bootdrive_common::{BootDriveError, ErrorCode};

use crate::image::Caller;

/// Decides whether a caller may perform privileged actions.
pub trait Authorizer: Send + Sync {
    /// Authorize an `Activate` request, or return [`ErrorCode::NotAuthorized`].
    fn authorize_activate(&self, caller: &Caller) -> Result<(), BootDriveError>;
}

/// Group-membership authorizer: root, or a member of the configured gid.
#[derive(Debug, Clone)]
pub struct GroupAuthorizer {
    required_gid: Option<u32>,
    group_name: String,
}

impl GroupAuthorizer {
    /// Build an authorizer for the named group, resolving its gid now.
    pub fn for_group(name: &str) -> Self {
        GroupAuthorizer {
            required_gid: group_gid(name),
            group_name: name.to_string(),
        }
    }
}

impl Authorizer for GroupAuthorizer {
    fn authorize_activate(&self, caller: &Caller) -> Result<(), BootDriveError> {
        if caller.uid == 0 {
            return Ok(());
        }
        match self.required_gid {
            Some(gid) if caller.gids.contains(&gid) => Ok(()),
            _ => Err(BootDriveError::new(
                ErrorCode::NotAuthorized,
                format!(
                    "you must be a member of the '{}' group to expose an image",
                    self.group_name
                ),
            )),
        }
    }
}

/// Authorizer that permits everyone. Test/development use only.
#[derive(Debug, Clone, Copy)]
pub struct AllowAllAuthorizer;

impl Authorizer for AllowAllAuthorizer {
    fn authorize_activate(&self, _caller: &Caller) -> Result<(), BootDriveError> {
        Ok(())
    }
}

/// Look up the gid of a group by parsing `/etc/group`. Returns `None` if the
/// group does not exist (in which case only root is authorized).
pub fn group_gid(name: &str) -> Option<u32> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let mut fields = line.split(':');
        let gname = fields.next()?;
        if gname != name {
            continue;
        }
        let _passwd = fields.next();
        let gid = fields.next()?;
        return gid.trim().parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_always_authorized() {
        let auth = GroupAuthorizer {
            required_gid: Some(999),
            group_name: "bootdrive".into(),
        };
        assert!(auth
            .authorize_activate(&Caller {
                uid: 0,
                gids: vec![0]
            })
            .is_ok());
    }

    #[test]
    fn member_is_authorized() {
        let auth = GroupAuthorizer {
            required_gid: Some(42),
            group_name: "bootdrive".into(),
        };
        assert!(auth
            .authorize_activate(&Caller {
                uid: 1000,
                gids: vec![1000, 42]
            })
            .is_ok());
    }

    #[test]
    fn non_member_is_rejected() {
        let auth = GroupAuthorizer {
            required_gid: Some(42),
            group_name: "bootdrive".into(),
        };
        let err = auth
            .authorize_activate(&Caller {
                uid: 1000,
                gids: vec![1000],
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotAuthorized);
    }

    #[test]
    fn missing_group_rejects_non_root() {
        let auth = GroupAuthorizer {
            required_gid: None,
            group_name: "bootdrive".into(),
        };
        assert!(auth
            .authorize_activate(&Caller {
                uid: 1000,
                gids: vec![1000]
            })
            .is_err());
    }

    #[test]
    fn allow_all_permits_everyone() {
        assert!(AllowAllAuthorizer
            .authorize_activate(&Caller {
                uid: 1000,
                gids: vec![1000]
            })
            .is_ok());
    }
}
