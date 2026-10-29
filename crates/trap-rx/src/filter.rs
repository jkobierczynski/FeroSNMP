//! Choosing which notifications to show.

use std::net::IpAddr;

use mib_core::{Oid, Pattern};

use crate::describe::Described;

/// Which notifications to keep. An empty filter keeps everything; each
/// condition that is set must match.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Only from this sender address.
    pub source: Option<IpAddr>,
    /// Only notifications whose name matches, as `ferosnmp mib search`
    /// matches (`link*`, `IF-MIB::*`), or whose OID is at or below this
    /// numeric OID.
    pub notification: Option<String>,
}

impl Filter {
    /// Whether `described` passes the filter.
    pub fn matches(&self, described: &Described) -> bool {
        if let Some(source) = self.source
            && source != described.source.ip()
        {
            return false;
        }
        if let Some(pattern) = &self.notification
            && !name_matches(pattern, described)
        {
            return false;
        }
        true
    }
}

fn name_matches(pattern: &str, described: &Described) -> bool {
    if let Ok(prefix) = pattern.trim_start_matches('.').parse::<Oid>()
        && pattern.contains('.')
    {
        return described
            .oid
            .as_ref()
            .is_some_and(|oid| oid.starts_with(&prefix));
    }
    let compiled = Pattern::new(pattern);
    if compiled.is_qualified() {
        compiled.matches(&described.symbol)
    } else {
        let bare = described
            .symbol
            .rsplit_once("::")
            .map_or(described.symbol.as_str(), |(_, name)| name);
        compiled.matches(bare)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snmp_core::NotificationKind;

    fn described(name: &str, oid: &str, source: &str) -> Described {
        Described {
            time: String::new(),
            source: source.parse().unwrap(),
            version: "2c",
            kind: NotificationKind::Trap,
            acknowledged: false,
            community: None,
            user: None,
            name: name.to_string(),
            symbol: name.to_string(),
            oid: Some(oid.parse().unwrap()),
            known: true,
            uptime: None,
            v1: None,
            objects: vec![],
            hints: vec![],
        }
    }

    #[test]
    fn filters_by_name_oid_and_source() {
        let link = described("IF-MIB::linkDown", "1.3.6.1.6.3.1.1.5.3", "192.0.2.1:5000");
        let filter = |pattern: &str| Filter {
            notification: Some(pattern.to_string()),
            ..Filter::default()
        };
        assert!(Filter::default().matches(&link));
        assert!(filter("link*").matches(&link));
        assert!(filter("down").matches(&link));
        assert!(filter("IF-MIB::*").matches(&link));
        assert!(!filter("SNMPv2-MIB::*").matches(&link));
        assert!(!filter("coldStart").matches(&link));
        assert!(filter("1.3.6.1.6.3.1.1.5").matches(&link));
        assert!(filter(".1.3.6.1.6.3.1.1.5.3").matches(&link));
        assert!(!filter("1.3.6.1.4.1").matches(&link));

        let from = |ip: &str| Filter {
            source: Some(ip.parse().unwrap()),
            ..Filter::default()
        };
        assert!(from("192.0.2.1").matches(&link));
        assert!(!from("192.0.2.2").matches(&link));
    }
}
