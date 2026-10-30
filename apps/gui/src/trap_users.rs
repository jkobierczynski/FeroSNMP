//! SNMPv3 trap users: who may send SNMPv3 traps and INFORMs to the Traps
//! tab, kept in the settings file like the saved agents.
//!
//! In `settings.toml` each one is a `[[trap-users]]` table. The passwords
//! are not in the file but in the Windows Credential Manager (see
//! `secrets.rs`):
//!
//! ```toml
//! [[trap-users]]
//! name = "trapper"
//! engine-id = "80001f88040102030405"
//! security-level = "authPriv"
//! auth-protocol = "SHA-256"
//! priv-protocol = "AES"
//! ```
//!
//! Without an `engine-id` the user is accepted from any sender, as
//! `ferosnmp traps listen -u` does.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use snmp_core::{AuthProtocol, PrivProtocol, SecurityLevel, TrapUser, Usm, hex, parse_hex};

/// An SNMPv3 trap user saved in the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SavedTrapUser {
    /// The SNMPv3 user name.
    pub name: String,
    /// The sender's engine ID in hex, or empty for any sender.
    pub engine_id: String,
    /// The lowest security level accepted from this user.
    #[serde(with = "by_name")]
    pub security_level: SecurityLevel,
    /// Authentication protocol.
    #[serde(with = "by_name")]
    pub auth_protocol: AuthProtocol,
    /// Privacy protocol.
    #[serde(with = "by_name")]
    pub priv_protocol: PrivProtocol,
    /// Authentication password: never written to the file.
    #[serde(skip)]
    pub auth_password: String,
    /// Privacy password: never written to the file.
    #[serde(skip)]
    pub priv_password: String,
}

impl Default for SavedTrapUser {
    fn default() -> Self {
        SavedTrapUser {
            name: String::new(),
            engine_id: String::new(),
            security_level: SecurityLevel::AuthPriv,
            auth_protocol: AuthProtocol::Sha1,
            priv_protocol: PrivProtocol::Aes128,
            auth_password: String::new(),
            priv_password: String::new(),
        }
    }
}

/// Protocols and levels are written by their names, such as `"SHA-256"`.
mod by_name {
    use super::*;

    pub fn serialize<T: std::fmt::Display, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: std::str::FromStr<Err = String>,
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl SavedTrapUser {
    /// A saved user for one read from a `createUser` line.
    pub fn from_trap_user(user: &TrapUser) -> SavedTrapUser {
        let usm = &user.usm;
        let mut saved = SavedTrapUser {
            name: usm.user.clone(),
            engine_id: user.engine_id.as_deref().map(hex).unwrap_or_default(),
            security_level: usm.level,
            auth_protocol: usm.auth_protocol,
            priv_protocol: usm.priv_protocol,
            auth_password: String::new(),
            priv_password: String::new(),
        };
        if usm.level != SecurityLevel::NoAuthNoPriv {
            saved.auth_password = usm.auth_password.clone();
        }
        if usm.level == SecurityLevel::AuthPriv {
            saved.priv_password = usm.priv_password.clone();
        }
        saved
    }

    /// The engine ID as bytes, if one is given.
    fn engine_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        let text = self.engine_id.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let digits = text.trim_start_matches("0x").trim_start_matches("0X");
        let id = parse_hex(digits).map_err(|error| format!("The engine ID {error}."))?;
        if !(5..=32).contains(&id.len()) {
            return Err(
                "An engine ID has 5 to 32 bytes (10 to 64 hex digits), such as \
                 80001f88040102030405."
                    .to_string(),
            );
        }
        Ok(Some(id))
    }

    /// The receiver's user, with the passwords.
    pub fn trap_user(&self) -> Result<TrapUser, String> {
        let mut usm = Usm::new(self.name.trim());
        if self.security_level != SecurityLevel::NoAuthNoPriv {
            usm = usm.with_auth(self.auth_protocol, &self.auth_password);
            if self.security_level == SecurityLevel::AuthPriv {
                usm = usm.with_privacy(self.priv_protocol, &self.priv_password);
            }
        }
        usm.check().map_err(|problem| {
            let problem = problem.replace("SNMPv3 needs a user name", "Enter the user name");
            match problem.get(..1) {
                Some(first) => format!("{}{}.", first.to_uppercase(), &problem[1..]),
                None => problem,
            }
        })?;
        let user = TrapUser::new(usm);
        Ok(match self.engine_bytes()? {
            Some(id) => user.with_engine_id(id),
            None => user,
        })
    }

    /// What is wrong with this user, if anything, as one sentence for the
    /// Trap user dialog. `others` are the other saved users.
    pub fn problem<'a>(
        &self,
        mut others: impl Iterator<Item = &'a SavedTrapUser>,
    ) -> Option<String> {
        if let Err(problem) = self.trap_user() {
            return Some(problem);
        }
        let same = |other: &SavedTrapUser| {
            other.name.trim() == self.name.trim()
                && other.engine_bytes().ok() == self.engine_bytes().ok()
        };
        others.any(same).then(|| {
            format!(
                "There is already a trap user \u{201c}{}\u{201d} for {}.",
                self.name.trim(),
                self.sender_text()
            )
        })
    }

    /// `any sender`, or `engine 80001f8804...`.
    fn sender_text(&self) -> String {
        match self.engine_id.trim() {
            "" => "any sender".to_string(),
            engine => format!("engine {}", engine.trim_start_matches("0x")),
        }
    }

    /// One line for the list on the Traps settings page:
    /// `trapper: authPriv, SHA-256 and AES, from any sender`.
    pub fn label(&self) -> String {
        let protocols = match self.security_level {
            SecurityLevel::NoAuthNoPriv => String::new(),
            SecurityLevel::AuthNoPriv => format!(", {}", self.auth_protocol),
            SecurityLevel::AuthPriv => {
                format!(", {} and {}", self.auth_protocol, self.priv_protocol)
            }
        };
        format!(
            "{}: {}{protocols}, from {}",
            self.name.trim(),
            self.security_level,
            self.sender_text()
        )
    }
}

/// The receiver's users for the saved ones. Users that are not complete
/// (a password missing from the Credential Manager, say) are left out and
/// reported.
pub fn trap_users(saved: &[SavedTrapUser]) -> (Vec<TrapUser>, Vec<String>) {
    let mut users = Vec::new();
    let mut problems = Vec::new();
    for user in saved {
        if user.security_level != SecurityLevel::NoAuthNoPriv && user.auth_password.is_empty() {
            problems.push(format!(
                "trap user {} is not used: its password is not in the Windows Credential \
                 Manager; enter it in Traps > Trap settings",
                user.name.trim()
            ));
            continue;
        }
        match user.trap_user() {
            Ok(user) => users.push(user),
            Err(problem) => problems.push(format!(
                "trap user {} is not used: {problem}",
                user.name.trim()
            )),
        }
    }
    (users, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> SavedTrapUser {
        SavedTrapUser {
            name: "admin".into(),
            security_level: SecurityLevel::AuthPriv,
            auth_protocol: AuthProtocol::Sha256,
            auth_password: "ferosnmp-auth".into(),
            priv_password: "ferosnmp-priv".into(),
            ..SavedTrapUser::default()
        }
    }

    #[test]
    fn saved_users_become_receiver_users() {
        let user = admin().trap_user().unwrap();
        assert_eq!(user.usm.user, "admin");
        assert_eq!(user.usm.level, SecurityLevel::AuthPriv);
        assert_eq!(user.engine_id, None);
        let bound = SavedTrapUser {
            engine_id: "0x80001F88040102030405".into(),
            ..admin()
        };
        assert_eq!(
            bound.trap_user().unwrap().engine_id,
            Some(parse_hex("80001f88040102030405").unwrap())
        );
        assert_eq!(
            admin().label(),
            "admin: authPriv, SHA-256 and AES, from any sender"
        );
    }

    #[test]
    fn problems_are_sentences() {
        let problem = |user: SavedTrapUser| user.problem([admin()].iter()).unwrap();
        assert_eq!(
            problem(SavedTrapUser {
                name: " ".into(),
                ..admin()
            }),
            "Enter the user name."
        );
        assert!(
            problem(SavedTrapUser {
                auth_password: "short".into(),
                ..admin()
            })
            .contains("at least 8"),
        );
        assert!(
            problem(SavedTrapUser {
                engine_id: "12".into(),
                ..admin()
            })
            .contains("5 to 32 bytes")
        );
        assert!(problem(admin()).contains("already a trap user"));
        let other_engine = SavedTrapUser {
            engine_id: "80001f88040102030405".into(),
            ..admin()
        };
        assert_eq!(other_engine.problem([admin()].iter()), None);
        let noauth = SavedTrapUser {
            name: "watcher".into(),
            security_level: SecurityLevel::NoAuthNoPriv,
            ..SavedTrapUser::default()
        };
        assert_eq!(noauth.problem(std::iter::empty()), None);
        assert_eq!(noauth.label(), "watcher: noAuthNoPriv, from any sender");
    }

    #[test]
    fn imported_users_keep_their_passwords() {
        let users = trap_rx::parse_users(
            "createUser -e 0x80001f88040102030405 trapper SHA-512 \"auth pass\" AES-256 \"priv pass\"\n\
             createUser watcher\n",
        )
        .unwrap();
        let saved: Vec<SavedTrapUser> = users.iter().map(SavedTrapUser::from_trap_user).collect();
        assert_eq!(saved[0].name, "trapper");
        assert_eq!(saved[0].engine_id, "80001f88040102030405");
        assert_eq!(saved[0].auth_password, "auth pass");
        assert_eq!(saved[0].priv_password, "priv pass");
        assert_eq!(saved[0].security_level, SecurityLevel::AuthPriv);
        assert_eq!(saved[1].security_level, SecurityLevel::NoAuthNoPriv);
        let (back, problems) = trap_users(&saved);
        assert_eq!(back.len(), 2, "{problems:?}");
        let (_, problems) = trap_users(&[SavedTrapUser {
            auth_password: String::new(),
            ..admin()
        }]);
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].starts_with("trap user admin is not used"),
            "{problems:?}"
        );
    }

    #[test]
    fn written_to_settings_without_passwords() {
        let text = toml::to_string(&admin()).unwrap();
        assert!(text.contains("auth-protocol = \"SHA-256\""), "{text}");
        assert!(!text.contains("ferosnmp-auth"), "{text}");
        let back: SavedTrapUser = toml::from_str(&text).unwrap();
        assert_eq!(back.security_level, SecurityLevel::AuthPriv);
    }
}
