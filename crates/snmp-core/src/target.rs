//! The agent to talk to and how.

use std::fmt;
use std::time::Duration;

/// The SNMP versions the engine speaks. SNMPv3 follows in a later milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Version {
    /// SNMPv1 (RFC 1157): no GETBULK, no Counter64, errors instead of
    /// exceptions.
    V1,
    /// SNMPv2c (RFC 1901, RFC 3416).
    V2c,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Version::V1 => "SNMPv1",
            Version::V2c => "SNMPv2c",
        })
    }
}

/// The default SNMP port.
pub const DEFAULT_PORT: u16 = 161;

/// An agent and the settings for talking to it.
///
/// ```
/// use snmp_core::{Target, Version};
///
/// let target = Target::parse("192.0.2.1:1161").unwrap().with_community("private");
/// assert_eq!(target.port, 1161);
/// assert_eq!(target.version, Version::V2c);
/// assert_eq!(target.to_string(), "192.0.2.1:1161");
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Target {
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    /// UDP port.
    pub port: u16,
    /// Protocol version.
    pub version: Version,
    /// Community string.
    pub community: Vec<u8>,
    /// How long to wait for each response.
    pub timeout: Duration,
    /// How many times to send a request again after a timeout.
    pub retries: u32,
}

impl Target {
    /// A target with the defaults: SNMPv2c, community `public`, a timeout of
    /// 2 seconds and 2 retries.
    pub fn new(host: impl Into<String>, port: u16) -> Target {
        Target {
            host: host.into(),
            port,
            version: Version::V2c,
            community: b"public".to_vec(),
            timeout: Duration::from_secs(2),
            retries: 2,
        }
    }

    /// Parse `host`, `host:port`, `[ipv6]:port` or a bare IPv6 address.
    pub fn parse(text: &str) -> Result<Target, ParseTargetError> {
        let error = |reason: &str| ParseTargetError {
            text: text.to_string(),
            reason: reason.to_string(),
        };
        let text = text.trim();
        let (host, port) = if let Some(rest) = text.strip_prefix('[') {
            let (host, after) = rest
                .split_once(']')
                .ok_or_else(|| error("missing `]` after the IPv6 address"))?;
            match after {
                "" => (host, None),
                _ => match after.strip_prefix(':') {
                    Some(port) => (host, Some(port)),
                    None => return Err(error("expected `:port` after `]`")),
                },
            }
        } else if text.matches(':').count() > 1 {
            // A bare IPv6 address; it cannot carry a port without brackets.
            (text, None)
        } else {
            match text.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (text, None),
            }
        };
        if host.is_empty() {
            return Err(error("no host name or address"));
        }
        let port = match port {
            None => DEFAULT_PORT,
            Some(port) => match port.parse::<u16>() {
                Ok(port) if port > 0 => port,
                _ => return Err(error("the port must be a number from 1 to 65535")),
            },
        };
        Ok(Target::new(host, port))
    }

    /// Set the protocol version.
    pub fn with_version(mut self, version: Version) -> Target {
        self.version = version;
        self
    }

    /// Set the community string.
    pub fn with_community(mut self, community: impl Into<Vec<u8>>) -> Target {
        self.community = community.into();
        self
    }

    /// Set how long to wait for each response.
    pub fn with_timeout(mut self, timeout: Duration) -> Target {
        self.timeout = timeout;
        self
    }

    /// Set how many times to retry after a timeout.
    pub fn with_retries(mut self, retries: u32) -> Target {
        self.retries = retries;
        self
    }
}

impl fmt::Display for Target {
    /// `host:port`, with brackets around an IPv6 address.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

impl fmt::Debug for Target {
    // The community is a password, so it is left out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("version", &self.version)
            .field("timeout", &self.timeout)
            .field("retries", &self.retries)
            .finish_non_exhaustive()
    }
}

/// Error returned by [`Target::parse`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a valid agent address {text:?}: {reason}")]
pub struct ParseTargetError {
    text: String,
    reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_port(text: &str) -> (String, u16) {
        let target = Target::parse(text).unwrap();
        (target.host, target.port)
    }

    #[test]
    fn parses_addresses() {
        assert_eq!(host_port("router"), ("router".into(), 161));
        assert_eq!(host_port("10.0.0.1:1161"), ("10.0.0.1".into(), 1161));
        assert_eq!(host_port("[::1]:1161"), ("::1".into(), 1161));
        assert_eq!(host_port("[::1]"), ("::1".into(), 161));
        assert_eq!(host_port("fe80::1"), ("fe80::1".into(), 161));
    }

    #[test]
    fn rejects_bad_addresses() {
        for text in [
            "",
            ":161",
            "host:",
            "host:0",
            "host:70000",
            "[::1",
            "[::1]x",
        ] {
            assert!(Target::parse(text).is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn display_and_debug() {
        assert_eq!(Target::parse("[::1]:162").unwrap().to_string(), "[::1]:162");
        let debug = format!("{:?}", Target::new("h", 161).with_community("secret"));
        assert!(!debug.contains("secret"), "{debug}");
    }
}
