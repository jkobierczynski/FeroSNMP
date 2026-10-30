//! SNMPv3 passwords of saved agents and trap users, kept out of
//! `settings.toml`.
//!
//! On Windows they are generic credentials in the Credential Manager
//! (Control Panel > Credential Manager > Windows Credentials), one per
//! password, stored for the current user on this computer. An agent's are
//! named `FeroSNMP/<agent name>/auth` and `.../priv`; a trap user's
//! `FeroSNMP/trap-user/<user>/auth`, with `@<engine ID>` after the user when
//! the user belongs to one sender. The Win32 calls are in
//! `ui/credentials.rs`; this module decides what to write and delete, so
//! that part is tested on every platform.

use crate::targets::{SavedTarget, VersionSetting};
use crate::trap_users::SavedTrapUser;

/// Which password.
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
}

/// Something with SNMPv3 passwords that live in the Credential Manager.
pub trait WithSecrets {
    /// The start of its credential names, or `None` when it has no
    /// passwords (an agent that is not SNMPv3).
    fn secret_prefix(&self) -> Option<String>;
    /// The SNMPv3 user name, shown next to the password.
    fn secret_user(&self) -> &str;
    /// One of its passwords.
    fn secret(&self, kind: Kind) -> &str;
    /// One of its passwords, to fill in.
    fn secret_mut(&mut self, kind: Kind) -> &mut String;
}

impl WithSecrets for SavedTarget {
    fn secret_prefix(&self) -> Option<String> {
        (self.version == VersionSetting::V3).then(|| format!("FeroSNMP/{}", self.name.trim()))
    }

    fn secret_user(&self) -> &str {
        self.user.trim()
    }

    fn secret(&self, kind: Kind) -> &str {
        match kind {
            Kind::Auth => &self.auth_password,
            Kind::Priv => &self.priv_password,
        }
    }

    fn secret_mut(&mut self, kind: Kind) -> &mut String {
        match kind {
            Kind::Auth => &mut self.auth_password,
            Kind::Priv => &mut self.priv_password,
        }
    }
}

impl WithSecrets for SavedTrapUser {
    fn secret_prefix(&self) -> Option<String> {
        let engine = self.engine_id.trim();
        Some(if engine.is_empty() {
            format!("FeroSNMP/trap-user/{}", self.name.trim())
        } else {
            format!(
                "FeroSNMP/trap-user/{}@{}",
                self.name.trim(),
                engine.trim_start_matches("0x").to_lowercase()
            )
        })
    }

    fn secret_user(&self) -> &str {
        self.name.trim()
    }

    fn secret(&self, kind: Kind) -> &str {
        match kind {
            Kind::Auth => &self.auth_password,
            Kind::Priv => &self.priv_password,
        }
    }

    fn secret_mut(&mut self, kind: Kind) -> &mut String {
        match kind {
            Kind::Auth => &mut self.auth_password,
            Kind::Priv => &mut self.priv_password,
        }
    }
}

/// The credential name for one password of `item`.
pub fn key(item: &impl WithSecrets, kind: Kind) -> Option<String> {
    item.secret_prefix()
        .map(|prefix| format!("{prefix}/{}", kind.suffix()))
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

/// What to change when the saved items go from `old` to `new`: write the
/// passwords of every item in `new` that has them, and delete the ones that
/// are no longer needed (items deleted, renamed, no longer SNMPv3, or with
/// an emptied password).
pub fn changes<T: WithSecrets>(old: &[T], new: &[T]) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut kept = Vec::new();
    for item in new {
        for kind in Kind::ALL {
            let secret = item.secret(kind);
            let Some(key) = key(item, kind).filter(|_| !secret.is_empty()) else {
                continue;
            };
            kept.push(key.clone());
            let unchanged = old.iter().any(|o| {
                self::key(o, kind).as_ref() == Some(&key)
                    && o.secret(kind) == secret
                    && o.secret_user() == item.secret_user()
            });
            if !unchanged {
                changes.push(Change::Write {
                    key,
                    kind,
                    user: item.secret_user().to_string(),
                    secret: secret.to_string(),
                });
            }
        }
    }
    for item in old {
        for kind in Kind::ALL {
            if let Some(key) = key(item, kind)
                && !item.secret(kind).is_empty()
                && !kept.contains(&key)
            {
                changes.push(Change::Delete { key });
            }
        }
    }
    changes
}

/// Fill in the passwords with `read`, which returns the secret stored under
/// a key, if any.
pub fn fill<T: WithSecrets>(items: &mut [T], mut read: impl FnMut(&str) -> Option<String>) {
    for item in items {
        for kind in Kind::ALL {
            if let Some(secret) = key(item, kind).and_then(|key| read(&key)) {
                *item.secret_mut(kind) = secret;
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
    fn trap_users_have_their_own_names() {
        let user = |name: &str, engine: &str| SavedTrapUser {
            name: name.into(),
            engine_id: engine.into(),
            auth_password: "password1".into(),
            ..SavedTrapUser::default()
        };
        let old = [user("admin", ""), user("admin", "0x80001F8804")];
        assert_eq!(
            keys(&changes(&[], &old)),
            [
                "write FeroSNMP/trap-user/admin/auth",
                "write FeroSNMP/trap-user/admin@80001f8804/auth"
            ]
        );
        assert_eq!(
            keys(&changes(&old, &old[..1])),
            ["delete FeroSNMP/trap-user/admin@80001f8804/auth"]
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
