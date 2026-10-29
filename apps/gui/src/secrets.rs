//! SNMPv3 passwords of saved agents, kept out of `settings.toml`.
//!
//! On Windows they are generic credentials in the Credential Manager
//! (Control Panel > Credential Manager > Windows Credentials), one per
//! password, named `FeroSNMP/<agent name>/auth` and `.../priv` and stored
//! for the current user on this computer. The Win32 calls are in
//! `ui/credentials.rs`; this module decides what to write and delete, so
//! that part is tested on every platform.

use crate::targets::{SavedTarget, VersionSetting};

/// Which password of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The authentication password.
    Auth,
    /// The privacy password.
    Priv,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Auth, Kind::Priv];

    fn suffix(self) -> &'static str {
        match self {
            Kind::Auth => "auth",
            Kind::Priv => "priv",
        }
    }

    /// The comment shown in the Credential Manager.
    pub fn comment(self) -> &'static str {
        match self {
            Kind::Auth => "FeroSNMP: SNMPv3 authentication password",
            Kind::Priv => "FeroSNMP: SNMPv3 privacy password",
        }
    }

    /// This password of a saved agent.
    pub fn of(self, target: &SavedTarget) -> &str {
        match self {
            Kind::Auth => &target.auth_password,
            Kind::Priv => &target.priv_password,
        }
    }

    fn of_mut(self, target: &mut SavedTarget) -> &mut String {
        match self {
            Kind::Auth => &mut target.auth_password,
            Kind::Priv => &mut target.priv_password,
        }
    }
}

/// The credential name for one password of an agent.
pub fn key(agent: &str, kind: Kind) -> String {
    format!("FeroSNMP/{}/{}", agent.trim(), kind.suffix())
}

/// A change to the stored passwords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Store `secret` under `key`, with the SNMPv3 user name shown next to it.
    Write {
        key: String,
        kind: Kind,
        user: String,
        secret: String,
    },
    /// Remove `key`.
    Delete { key: String },
}

/// What to change when the saved agents go from `old` to `new`: write the
/// passwords of every SNMPv3 agent in `new`, and delete the ones that are
/// no longer needed (agents deleted, renamed, no longer SNMPv3, or with an
/// emptied password).
pub fn changes(old: &[SavedTarget], new: &[SavedTarget]) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut kept = Vec::new();
    for target in new.iter().filter(|t| t.version == VersionSetting::V3) {
        for kind in Kind::ALL {
            let secret = kind.of(target);
            if secret.is_empty() {
                continue;
            }
            let key = key(&target.name, kind);
            kept.push(key.clone());
            let unchanged = old.iter().any(|o| {
                o.version == VersionSetting::V3
                    && key == self::key(&o.name, kind)
                    && kind.of(o) == secret
                    && o.user == target.user
            });
            if !unchanged {
                changes.push(Change::Write {
                    key,
                    kind,
                    user: target.user.trim().to_string(),
                    secret: secret.to_string(),
                });
            }
        }
    }
    for target in old {
        for kind in Kind::ALL {
            let key = key(&target.name, kind);
            if !kind.of(target).is_empty() && !kept.contains(&key) {
                changes.push(Change::Delete { key });
            }
        }
    }
    changes
}

/// Fill in the passwords of the SNMPv3 agents with `read`, which returns
/// the secret stored under a key, if any.
pub fn fill(targets: &mut [SavedTarget], mut read: impl FnMut(&str) -> Option<String>) {
    for target in targets
        .iter_mut()
        .filter(|t| t.version == VersionSetting::V3)
    {
        for kind in Kind::ALL {
            if let Some(secret) = read(&key(&target.name, kind)) {
                *kind.of_mut(target) = secret;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3(name: &str, auth: &str, privacy: &str) -> SavedTarget {
        SavedTarget {
            name: name.into(),
            host: "h".into(),
            version: VersionSetting::V3,
            user: "u".into(),
            auth_password: auth.into(),
            priv_password: privacy.into(),
            ..SavedTarget::default()
        }
    }

    fn keys(changes: &[Change]) -> Vec<String> {
        changes
            .iter()
            .map(|c| match c {
                Change::Write { key, .. } => format!("write {key}"),
                Change::Delete { key } => format!("delete {key}"),
            })
            .collect()
    }

    #[test]
    fn new_and_changed_passwords_are_written() {
        let old = [v3("a", "password1", "password2")];
        assert!(changes(&old, &old).is_empty());
        let new = [v3("a", "password1", "changed!!"), v3("b", "password3", "")];
        assert_eq!(
            keys(&changes(&old, &new)),
            ["write FeroSNMP/a/priv", "write FeroSNMP/b/auth"]
        );
    }

    #[test]
    fn unused_passwords_are_deleted() {
        let old = [v3("a", "password1", "password2"), v3("b", "password3", "")];
        let mut renamed = v3("c", "password1", "password2");
        renamed.name = "c".into();
        let mut v2 = v3("b", "password3", "");
        v2.version = VersionSetting::V2c;
        let new = [renamed, v2];
        assert_eq!(
            keys(&changes(&old, &new)),
            [
                "write FeroSNMP/c/auth",
                "write FeroSNMP/c/priv",
                "delete FeroSNMP/a/auth",
                "delete FeroSNMP/a/priv",
                "delete FeroSNMP/b/auth",
            ]
        );
    }

    #[test]
    fn passwords_are_filled_in() {
        let mut targets = [v3("a", "", ""), SavedTarget::default()];
        fill(&mut targets, |key| {
            (key == "FeroSNMP/a/auth").then(|| "from the store".to_string())
        });
        assert_eq!(targets[0].auth_password, "from the store");
        assert_eq!(targets[0].priv_password, "");
    }
}
