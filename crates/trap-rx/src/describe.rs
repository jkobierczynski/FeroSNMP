//! Notifications decoded against the loaded MIBs: the notification's name,
//! formatted objects, and hints for what no MIB defines.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use mib_core::{MibSet, NodeKind, Oid};
use snmp_core::{
    Formatter, NotificationKind, NotificationSecurity, SNMP_TRAP_ADDRESS_0, SNMP_TRAP_COMMUNITY_0,
    SNMP_TRAP_ENTERPRISE_0, SNMP_TRAP_OID_0, SNMP_TRAPS, SYS_UP_TIME_0, V1Trap, Varbind, Version,
    hex,
};

use crate::listener::Received;

/// A received notification, ready to show: everything as text, with the
/// MIBs' names and value formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    /// When it arrived, as UTC in RFC 3339 form: `2026-10-29T21:05:00.123Z`.
    pub time: String,
    /// The sender's address and port.
    pub source: SocketAddr,
    /// `"1"`, `"2c"` or `"3"`.
    pub version: &'static str,
    /// Trap or INFORM.
    pub kind: NotificationKind,
    /// For an INFORM: whether it was acknowledged.
    pub acknowledged: bool,
    /// The community (SNMPv1, SNMPv2c), as text.
    pub community: Option<String>,
    /// The SNMPv3 user and level, such as `admin` and `authPriv`, and the
    /// authoritative engine ID in hex.
    pub user: Option<(String, String, String)>,
    /// The notification's name, such as `IF-MIB::linkDown`, or its numeric
    /// OID when no loaded MIB defines it.
    pub name: String,
    /// The same as a name even when `name` is numeric (`-n`), for filters.
    pub symbol: String,
    /// The notification's OID.
    pub oid: Option<Oid>,
    /// Whether a loaded MIB (or the built-in list of standard
    /// notifications) knows the notification.
    pub known: bool,
    /// The sender's uptime, such as `1 day, 2:03:04.05`, and in hundredths
    /// of a second.
    pub uptime: Option<(String, u32)>,
    /// The fields of an SNMPv1 trap.
    pub v1: Option<DescribedV1>,
    /// The objects.
    pub objects: Vec<DescribedObject>,
    /// What to do about OIDs no loaded MIB defines.
    pub hints: Vec<String>,
}

/// The fields of an SNMPv1 trap, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedV1 {
    /// The enterprise, by name if known.
    pub enterprise: String,
    /// The agent address the trap names.
    pub agent_address: String,
    /// `generic-trap`, with its name: `linkDown(2)`.
    pub generic_trap: String,
    /// `specific-trap`.
    pub specific_trap: i64,
}

/// One object of a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribedObject {
    /// The instance, such as `IF-MIB::ifIndex.2`.
    pub name: String,
    /// The numeric OID.
    pub oid: Oid,
    /// The type, as net-snmp names it: `INTEGER`, `STRING`, ...
    pub type_name: String,
    /// The value, such as `down(2)`.
    pub value: String,
    /// Whether a loaded MIB defines the object.
    pub known: bool,
}

/// Names for the objects and notifications every SNMP tool knows, used when
/// SNMPv2-MIB and IF-MIB are not loaded.
fn built_in_name(oid: &Oid) -> Option<&'static str> {
    let arcs = oid.arcs();
    Some(match arcs {
        a if a == SYS_UP_TIME_0 => "SNMPv2-MIB::sysUpTime.0",
        a if a == SNMP_TRAP_OID_0 => "SNMPv2-MIB::snmpTrapOID.0",
        a if a == SNMP_TRAP_ENTERPRISE_0 => "SNMPv2-MIB::snmpTrapEnterprise.0",
        a if a == SNMP_TRAP_ADDRESS_0 => "SNMP-COMMUNITY-MIB::snmpTrapAddress.0",
        a if a == SNMP_TRAP_COMMUNITY_0 => "SNMP-COMMUNITY-MIB::snmpTrapCommunity.0",
        [prefix @ .., last] if prefix == SNMP_TRAPS => match last {
            1 => "SNMPv2-MIB::coldStart",
            2 => "SNMPv2-MIB::warmStart",
            3 => "IF-MIB::linkDown",
            4 => "IF-MIB::linkUp",
            5 => "SNMPv2-MIB::authenticationFailure",
            6 => "RFC1213-MIB::egpNeighborLoss",
            _ => return None,
        },
        _ => return None,
    })
}

/// An OID as `MODULE::name.suffix`, numeric with `numeric`. The standard
/// names stand in for what the loaded MIBs do not define.
fn display_name(
    mibs: Option<&MibSet>,
    formatter: &Formatter<'_>,
    oid: &Oid,
    numeric: bool,
) -> String {
    let defined = mibs.is_some_and(|m| {
        matches!(
            m.lookup(oid).node.kind(),
            NodeKind::Notification | NodeKind::Scalar | NodeKind::Column
        )
    });
    match built_in_name(oid) {
        Some(name) if !numeric && !defined => name.to_string(),
        _ => formatter.oid(oid),
    }
}

/// Decode `received` against `mibs` (or without MIBs: numeric OIDs, plain
/// values). `numeric` shows OIDs as numbers even when a MIB names them.
pub fn describe(received: &Received, mibs: Option<&MibSet>, numeric: bool) -> Described {
    let notification = &received.notification;
    let mut formatter = Formatter::new(mibs).with_numeric_oids(numeric);
    let mut hints = Vec::new();

    let name_of =
        |formatter: &Formatter<'_>, oid: &Oid| display_name(mibs, formatter, oid, numeric);

    let oid = notification.trap_oid().cloned();
    let (name, known) = match &oid {
        Some(oid) => {
            let known = mibs
                .and_then(|m| m.node_at(oid))
                .is_some_and(|node| node.kind() == NodeKind::Notification)
                || built_in_name(oid).is_some();
            if !known {
                hints.push(unknown_hint(mibs, oid, "notification"));
            }
            (name_of(&formatter, oid), known)
        }
        None => {
            hints.push(
                "the notification has no snmpTrapOID.0, so it cannot be identified; the \
                 sender does not follow RFC 3416"
                    .to_string(),
            );
            ("(no snmpTrapOID.0)".to_string(), false)
        }
    };

    // The name also when OIDs are shown as numbers, for filters.
    let symbol = oid
        .as_ref()
        .map(|oid| display_name(mibs, &Formatter::new(mibs), oid, false))
        .unwrap_or_else(|| name.clone());

    let mut objects = Vec::new();
    for varbind in notification.objects() {
        let known = built_in_name(&varbind.oid).is_some()
            || mibs.is_some_and(|m| {
                matches!(
                    m.lookup(&varbind.oid).node.kind(),
                    NodeKind::Scalar | NodeKind::Column
                )
            });
        if !known && mibs.is_some() {
            let hint = unknown_hint(mibs, &varbind.oid, "object");
            if !hints.contains(&hint) {
                hints.push(hint);
            }
        }
        objects.push(describe_object(&mut formatter, varbind, known, &name_of));
    }

    let (community, user) = match &notification.security {
        NotificationSecurity::Community(community) => {
            (Some(String::from_utf8_lossy(community).into_owned()), None)
        }
        NotificationSecurity::Usm(usm) => (
            None,
            Some((usm.user.clone(), usm.level.to_string(), hex(&usm.engine_id))),
        ),
    };

    Described {
        time: rfc3339(received.at),
        source: received.source,
        version: match notification.version {
            Version::V1 => "1",
            Version::V2c => "2c",
            Version::V3 => "3",
        },
        kind: notification.kind,
        acknowledged: received.acknowledged,
        community,
        user,
        name,
        symbol,
        oid,
        known,
        uptime: notification
            .uptime()
            .map(|ticks| (ticks_text(ticks), ticks)),
        v1: notification
            .v1
            .as_ref()
            .map(|v1| describe_v1(&formatter, v1, &name_of)),
        objects,
        hints,
    }
}

fn describe_object(
    formatter: &mut Formatter<'_>,
    varbind: &Varbind,
    known: bool,
    name_of: &impl Fn(&Formatter<'_>, &Oid) -> String,
) -> DescribedObject {
    let text = formatter.value_text(varbind);
    DescribedObject {
        name: name_of(formatter, &varbind.oid),
        oid: varbind.oid.clone(),
        type_name: text.type_name,
        value: text.text,
        known,
    }
}

fn describe_v1(
    formatter: &Formatter<'_>,
    v1: &V1Trap,
    name_of: &impl Fn(&Formatter<'_>, &Oid) -> String,
) -> DescribedV1 {
    let [a, b, c, d] = v1.agent_address;
    DescribedV1 {
        enterprise: name_of(formatter, &v1.enterprise),
        agent_address: format!("{a}.{b}.{c}.{d}"),
        generic_trap: match v1.generic_name() {
            Some(name) => format!("{name}({})", v1.generic_trap),
            None => v1.generic_trap.to_string(),
        },
        specific_trap: v1.specific_trap,
    }
}

/// "Load the vendor MIB", with what is known about where the OID is.
fn unknown_hint(mibs: Option<&MibSet>, oid: &Oid, what: &str) -> String {
    let Some(mibs) = mibs else {
        return format!("no MIBs are loaded, so the {what} {oid} is shown as a number");
    };
    let lookup = mibs.lookup(oid);
    let nearest = if lookup.node.oid().is_empty() {
        String::new()
    } else {
        format!(
            " (the nearest node it is below is {})",
            lookup.node.qualified_name()
        )
    };
    let enterprises = Oid::new(vec![1, 3, 6, 1, 4, 1]);
    let vendor = match oid.arcs().get(enterprises.len()) {
        Some(number) if oid.starts_with(&enterprises) => format!(
            "the MIB of enterprise number {number} (the vendor; see \
             https://www.iana.org/assignments/enterprise-numbers)"
        ),
        _ => "the MIB that defines it".to_string(),
    };
    format!("no loaded MIB defines the {what} {oid}{nearest}: add {vendor} to the MIB folders")
}

/// Hundredths of a second as net-snmp shows TimeTicks: `0:02:03.45`, or
/// `3 days, 4:05:06.07`.
fn ticks_text(ticks: u32) -> String {
    let (days, rest) = (ticks / 8_640_000, ticks % 8_640_000);
    let time = format!(
        "{}:{:02}:{:02}.{:02}",
        rest / 360_000,
        rest / 6000 % 60,
        rest / 100 % 60,
        rest % 100
    );
    match days {
        0 => time,
        1 => format!("1 day, {time}"),
        n => format!("{n} days, {time}"),
    }
}

/// RFC 3339 in UTC with milliseconds, without a date library.
pub fn rfc3339(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// Days since 1970-01-01 to year, month and day (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

impl Described {
    /// The notification as text: a header line, then one line per object.
    ///
    /// ```text
    /// 2026-10-29T21:05:00.123Z 192.0.2.1:41003 SNMPv2c trap, community "public"
    ///   FEROSNMP-TRAP-TEST-MIB::fttPortChange, uptime 0:00:42.42
    ///   FEROSNMP-TRAP-TEST-MIB::fttPortName = STRING: "eth0"
    /// ```
    pub fn text(&self) -> String {
        let mut text = format!(
            "{} {} SNMPv{} {}",
            self.time, self.source, self.version, self.kind
        );
        if self.kind == NotificationKind::Inform {
            text.push_str(if self.acknowledged {
                " (acknowledged)"
            } else {
                " (the acknowledgement could not be sent)"
            });
        }
        if let Some(community) = &self.community {
            text.push_str(&format!(", community {community:?}"));
        }
        if let Some((user, level, engine)) = &self.user {
            text.push_str(&format!(", user {user} {level}, engine {engine}"));
        }
        text.push_str(&format!("\n  {}", self.name));
        if let Some((uptime, _)) = &self.uptime {
            text.push_str(&format!(", uptime {uptime}"));
        }
        text.push('\n');
        if let Some(v1) = &self.v1 {
            text.push_str(&format!(
                "  SNMPv1 enterprise {}, agent {}, generic {}, specific {}\n",
                v1.enterprise, v1.agent_address, v1.generic_trap, v1.specific_trap
            ));
        }
        for object in &self.objects {
            let value = if object.value.is_empty() {
                object.type_name.clone()
            } else {
                format!("{}: {}", object.type_name, object.value)
            };
            text.push_str(&format!("  {} = {value}\n", object.name));
        }
        for hint in &self.hints {
            text.push_str(&format!("  hint: {hint}\n"));
        }
        text
    }

    /// The notification as one JSON object, for one line of JSON Lines
    /// output.
    pub fn json(&self) -> serde_json::Value {
        use serde_json::{Value, json};
        let mut object = json!({
            "time": self.time,
            "source": self.source.to_string(),
            "version": self.version,
            "type": self.kind.as_str(),
        });
        let map = object.as_object_mut().expect("an object");
        if self.kind == NotificationKind::Inform {
            map.insert("acknowledged".into(), Value::Bool(self.acknowledged));
        }
        if let Some(community) = &self.community {
            map.insert("community".into(), json!(community));
        }
        if let Some((user, level, engine)) = &self.user {
            map.insert("user".into(), json!(user));
            map.insert("level".into(), json!(level));
            map.insert("engine_id".into(), json!(engine));
        }
        map.insert("notification".into(), json!(self.name));
        map.insert(
            "oid".into(),
            self.oid
                .as_ref()
                .map_or(Value::Null, |oid| json!(oid.to_string())),
        );
        map.insert("known".into(), json!(self.known));
        if let Some((_, ticks)) = &self.uptime {
            map.insert("uptime".into(), json!(ticks));
        }
        if let Some(v1) = &self.v1 {
            map.insert(
                "v1".into(),
                json!({
                    "enterprise": v1.enterprise,
                    "agent_address": v1.agent_address,
                    "generic_trap": v1.generic_trap,
                    "specific_trap": v1.specific_trap,
                }),
            );
        }
        map.insert(
            "objects".into(),
            Value::Array(
                self.objects
                    .iter()
                    .map(|o| {
                        json!({
                            "name": o.name,
                            "oid": o.oid.to_string(),
                            "type": o.type_name,
                            "value": o.value,
                        })
                    })
                    .collect(),
            ),
        );
        if !self.hints.is_empty() {
            map.insert("hints".into(), json!(self.hints));
        }
        object
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn times_are_utc() {
        let at = |seconds: u64, millis: u64| {
            rfc3339(UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(millis))
        };
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(at(1_793_311_285, 123), "2026-10-29T22:01:25.123Z");
    }

    #[test]
    fn ticks_read_like_net_snmp() {
        assert_eq!(ticks_text(12345), "0:02:03.45");
        assert_eq!(ticks_text(8_640_000 + 100), "1 day, 0:00:01.00");
        assert_eq!(ticks_text(u32::MAX), "497 days, 2:27:52.95");
    }

    #[test]
    fn standard_names_are_built_in() {
        let name = |text: &str| built_in_name(&text.parse().unwrap());
        assert_eq!(name("1.3.6.1.6.3.1.1.5.3"), Some("IF-MIB::linkDown"));
        assert_eq!(name("1.3.6.1.2.1.1.3.0"), Some("SNMPv2-MIB::sysUpTime.0"));
        assert_eq!(name("1.3.6.1.6.3.1.1.5.7"), None);
        assert_eq!(name("1.3.6.1.4.1.9"), None);
    }
}
