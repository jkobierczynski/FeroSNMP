//! Saved agents: an address with the settings for talking to it, kept in the
//! settings file and picked in the Agent box of the toolbar.
//!
//! In `settings.toml` each one is a `[[targets]]` table:
//!
//! ```toml
//! [[targets]]
//! name = "lab switch"
//! host = "192.0.2.10"
//! port = 161
//! version = "2c"
//! community = "public"
//! timeout = 2.0
//! retries = 2
//! max-repetitions = 10
//! ```
//!
//! An SNMPv3 agent has `version = "3"` and the user settings; its passwords
//! are not in the file but in the Windows Credential Manager (see
//! `secrets.rs`):
//!
//! ```toml
//! [[targets]]
//! name = "core router"
//! host = "192.0.2.1"
//! version = "3"
//! user = "monitor"
//! security-level = "authPriv"
//! auth-protocol = "SHA-256"
//! priv-protocol = "AES"
//! ```

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use snmp_core::{
    AuthProtocol, DEFAULT_MAX_REPETITIONS, DEFAULT_PORT, PrivProtocol, SecurityLevel, Target, Usm,
    Version,
};

/// An agent saved in the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SavedTarget {
    /// What the Agent box shows, such as `lab switch`.
    pub name: String,
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    /// UDP port.
    pub port: u16,
    /// SNMP version.
    pub version: VersionSetting,
    /// Community string. Stored as plain text, like net-snmp's
    /// `snmp.conf`; SNMPv3 passwords go to the Windows Credential Manager
    /// instead.
    pub community: String,
    /// SNMPv3 user name.
    pub user: String,
    /// SNMPv3 security level.
    #[serde(with = "by_name")]
    pub security_level: SecurityLevel,
    /// SNMPv3 authentication protocol.
    #[serde(with = "by_name")]
    pub auth_protocol: AuthProtocol,
    /// SNMPv3 privacy protocol.
    #[serde(with = "by_name")]
    pub priv_protocol: PrivProtocol,
    /// SNMPv3 context name; usually empty.
    pub context: String,
    /// SNMPv3 authentication password: never written to the file.
    #[serde(skip)]
    pub auth_password: String,
    /// SNMPv3 privacy password: never written to the file.
    #[serde(skip)]
    pub priv_password: String,
    /// Seconds to wait for each response.
    pub timeout: f64,
    /// How often to send a request again when no response arrives.
    pub retries: u32,
    /// Successors per GETBULK request, for Bulk and for walks.
    pub max_repetitions: u32,
}

impl Default for SavedTarget {
    fn default() -> Self {
        SavedTarget {
            name: String::new(),
            host: String::new(),
            port: DEFAULT_PORT,
            version: VersionSetting::V2c,
            community: "public".to_string(),
            user: String::new(),
            security_level: SecurityLevel::AuthPriv,
            auth_protocol: AuthProtocol::Sha1,
            priv_protocol: PrivProtocol::Aes128,
            context: String::new(),
            auth_password: String::new(),
            priv_password: String::new(),
            timeout: 2.0,
            retries: 2,
            max_repetitions: DEFAULT_MAX_REPETITIONS,
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

/// [`Version`] as written in the settings file: `"1"`, `"2c"` or `"3"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionSetting {
    /// SNMPv1.
    #[serde(rename = "1")]
    V1,
    /// SNMPv2c.
    #[default]
    #[serde(rename = "2c")]
    V2c,
    /// SNMPv3.
    #[serde(rename = "3")]
    V3,
}

impl VersionSetting {
    /// Every value, in the order the Agents dialog lists them.
    pub const ALL: [VersionSetting; 3] =
        [VersionSetting::V2c, VersionSetting::V3, VersionSetting::V1];

    /// The name shown in the Agents dialog.
    pub fn label(self) -> &'static str {
        match self {
            VersionSetting::V1 => "SNMPv1",
            VersionSetting::V2c => "SNMPv2c",
            VersionSetting::V3 => "SNMPv3",
        }
    }
}

impl From<VersionSetting> for Version {
    fn from(setting: VersionSetting) -> Self {
        match setting {
            VersionSetting::V1 => Version::V1,
            VersionSetting::V2c => Version::V2c,
            VersionSetting::V3 => Version::V3,
        }
    }
}

/// The limits the Agents dialog enforces.
pub const MAX_TIMEOUT: f64 = 600.0;
pub const MAX_RETRIES: u32 = 20;
pub const MAX_REPETITIONS: u32 = 1000;

impl SavedTarget {
    /// What is wrong with this target, if anything, as one sentence for the
    /// Agents dialog. `others` are the names of the other saved targets.
    pub fn problem<'a>(&self, mut others: impl Iterator<Item = &'a str>) -> Option<String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Some("Give the agent a name.".to_string());
        }
        if others.any(|other| other.trim().eq_ignore_ascii_case(name)) {
            return Some(format!(
                "There is already an agent called \u{201c}{name}\u{201d}."
            ));
        }
        if self.host.trim().is_empty() {
            return Some("Enter the host name or IP address of the agent.".to_string());
        }
        if self.host.trim().contains(char::is_whitespace) {
            return Some("The host name cannot contain spaces.".to_string());
        }
        if self.port == 0 {
            return Some("The port must be a number from 1 to 65535.".to_string());
        }
        if !(self.timeout > 0.0 && self.timeout <= MAX_TIMEOUT) {
            return Some(format!(
                "The timeout must be more than 0 and at most {MAX_TIMEOUT} seconds."
            ));
        }
        if self.retries > MAX_RETRIES {
            return Some(format!("Retries must be at most {MAX_RETRIES}."));
        }
        if !(1..=MAX_REPETITIONS).contains(&self.max_repetitions) {
            return Some(format!(
                "Max-repetitions must be a number from 1 to {MAX_REPETITIONS}."
            ));
        }
        if self.version == VersionSetting::V3
            && let Err(problem) = self.usm().check()
        {
            let mut problem =
                problem.replace("SNMPv3 needs a user name", "Enter the SNMPv3 user name");
            if let Some(first) = problem.get(..1) {
                problem = first.to_uppercase() + &problem[1..];
            }
            return Some(format!("{problem}."));
        }
        None
    }

    /// The SNMPv3 user settings, with the passwords.
    pub fn usm(&self) -> Usm {
        Usm {
            user: self.user.trim().to_string(),
            level: self.security_level,
            auth_protocol: self.auth_protocol,
            auth_password: self.auth_password.clone(),
            priv_protocol: self.priv_protocol,
            priv_password: self.priv_password.clone(),
            context_name: self.context.trim().to_string(),
            context_engine_id: None,
        }
    }

    /// The engine's [`Target`] for this saved one.
    pub fn target(&self) -> Target {
        let host = self.host.trim();
        // `[2001:db8::1]` is accepted as the host too.
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let target = Target::new(host, self.port)
            .with_version(self.version.into())
            .with_community(self.community.as_bytes());
        let target = match self.version {
            VersionSetting::V3 => target.with_usm(self.usm()),
            _ => target,
        };
        target
            .with_timeout(Duration::from_secs_f64(
                self.timeout.clamp(0.001, MAX_TIMEOUT),
            ))
            .with_retries(self.retries.min(MAX_RETRIES))
    }
}

/// The agent to query, as picked in the Agent box.
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    /// What to send to.
    pub target: Target,
    /// Successors per GETBULK request.
    pub max_repetitions: u32,
    /// The saved target's name, or `None` for an address typed in the box,
    /// which uses the default settings.
    pub saved: Option<String>,
}

/// The agent for the text in the Agent box: the saved target with that name
/// (ignoring case), or else an address (`host`, `host:port`,
/// `[ipv6]:port`) with the default settings: SNMPv2c, community `public`,
/// 2 seconds, 2 retries.
pub fn pick(text: &str, saved: &[SavedTarget]) -> Result<Agent, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(
            "Type the address of an agent in the Agent box, or add one with Agents\u{2026}"
                .to_string(),
        );
    }
    if let Some(target) = saved
        .iter()
        .find(|target| target.name.trim().eq_ignore_ascii_case(text))
    {
        return Ok(Agent {
            target: target.target(),
            max_repetitions: target.max_repetitions.clamp(1, MAX_REPETITIONS),
            saved: Some(target.name.trim().to_string()),
        });
    }
    let defaults = SavedTarget::default();
    let parsed = Target::parse(text).map_err(|error| {
        format!("{error}. Pick a saved agent, or type an address such as 192.0.2.1 or router:1161.")
    })?;
    Ok(Agent {
        target: SavedTarget {
            host: parsed.host,
            port: parsed.port,
            ..defaults.clone()
        }
        .target(),
        max_repetitions: defaults.max_repetitions,
        saved: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab() -> SavedTarget {
        SavedTarget {
            name: "Lab switch".into(),
            host: "192.0.2.10".into(),
            port: 1161,
            version: VersionSetting::V1,
            community: "secret".into(),
            timeout: 0.5,
            retries: 0,
            max_repetitions: 25,
            ..SavedTarget::default()
        }
    }

    fn router() -> SavedTarget {
        SavedTarget {
            name: "Router".into(),
            host: "192.0.2.1".into(),
            version: VersionSetting::V3,
            user: "monitor".into(),
            security_level: SecurityLevel::AuthPriv,
            auth_protocol: AuthProtocol::Sha256,
            priv_protocol: PrivProtocol::Aes256,
            auth_password: "auth secret".into(),
            priv_password: "priv secret".into(),
            ..SavedTarget::default()
        }
    }

    #[test]
    fn snmpv3_targets() {
        let agent = pick("router", &[router()]).unwrap();
        assert_eq!(agent.target.version, Version::V3);
        assert_eq!(agent.target.usm.user, "monitor");
        assert_eq!(agent.target.usm.auth_password, "auth secret");
        assert_eq!(agent.target.usm.priv_protocol, PrivProtocol::Aes256);
        assert_eq!(router().problem(std::iter::empty()), None);

        // The passwords never reach the settings file.
        let text = toml::to_string(&router()).unwrap();
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("version = \"3\""), "{text}");
        assert!(text.contains("auth-protocol = \"SHA-256\""), "{text}");
        assert!(text.contains("security-level = \"authPriv\""), "{text}");
        let back: SavedTarget = toml::from_str(&text).unwrap();
        assert_eq!(back.auth_protocol, AuthProtocol::Sha256);
        assert_eq!(back.auth_password, "");

        let mut no_user = router();
        no_user.user = " ".into();
        let problem = no_user.problem(std::iter::empty()).unwrap();
        assert!(
            problem.starts_with("Enter the SNMPv3 user name"),
            "{problem}"
        );
        let mut short = router();
        short.auth_password = "short".into();
        let problem = short.problem(std::iter::empty()).unwrap();
        assert!(
            problem.starts_with("The authentication password"),
            "{problem}"
        );
        assert!(toml::from_str::<SavedTarget>("auth-protocol = \"ROT13\"").is_err());
    }

    #[test]
    fn saved_names_win_over_addresses() {
        let agent = pick(" lab SWITCH ", &[lab()]).unwrap();
        assert_eq!(agent.saved.as_deref(), Some("Lab switch"));
        assert_eq!(agent.target.to_string(), "192.0.2.10:1161");
        assert_eq!(agent.target.version, Version::V1);
        assert_eq!(agent.target.community, b"secret");
        assert_eq!(agent.target.timeout, Duration::from_millis(500));
        assert_eq!(agent.target.retries, 0);
        assert_eq!(agent.max_repetitions, 25);
    }

    #[test]
    fn typed_addresses_use_the_defaults() {
        let agent = pick("[::1]:1161", &[lab()]).unwrap();
        assert_eq!(agent.saved, None);
        assert_eq!(agent.target.to_string(), "[::1]:1161");
        assert_eq!(agent.target.version, Version::V2c);
        assert_eq!(agent.target.community, b"public");
        assert_eq!(agent.max_repetitions, DEFAULT_MAX_REPETITIONS);
        assert!(pick("", &[]).is_err());
        let error = pick("host:99999", &[]).unwrap_err();
        assert!(error.contains("port"), "{error}");
    }

    #[test]
    fn problems_are_explained() {
        let names = ["Other"];
        assert_eq!(lab().problem(names.into_iter()), None);
        let problem = |change: fn(&mut SavedTarget)| {
            let mut target = lab();
            change(&mut target);
            target.problem(names.into_iter())
        };
        assert!(problem(|t| t.name = " ".into()).is_some());
        assert!(
            problem(|t| t.name = "other".into())
                .unwrap()
                .contains("already")
        );
        assert!(problem(|t| t.host = String::new()).is_some());
        assert!(problem(|t| t.port = 0).is_some());
        assert!(problem(|t| t.timeout = 0.0).is_some());
        assert!(problem(|t| t.timeout = f64::NAN).is_some());
        assert!(problem(|t| t.max_repetitions = 0).is_some());
    }

    #[test]
    fn bracketed_ipv6_hosts_are_accepted() {
        let target = SavedTarget {
            host: "[2001:db8::1]".into(),
            ..SavedTarget::default()
        };
        assert_eq!(target.target().host, "2001:db8::1");
        assert_eq!(target.target().to_string(), "[2001:db8::1]:161");
    }

    #[test]
    fn toml_round_trip() {
        let text = toml::to_string(&lab()).unwrap();
        assert!(text.contains("version = \"1\""), "{text}");
        assert!(text.contains("max-repetitions = 25"), "{text}");
        let back: SavedTarget = toml::from_str(&text).unwrap();
        assert_eq!(back, lab());
        let sparse: SavedTarget = toml::from_str("name = \"x\"\nhost = \"h\"").unwrap();
        assert_eq!(sparse.community, "public");
        assert_eq!(sparse.port, 161);
    }
}
