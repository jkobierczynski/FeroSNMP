//! Receiving notifications: SNMPv1 traps, SNMPv2c traps and INFORMs, and
//! SNMPv3 traps and INFORMs (RFC 3416, RFC 3412, RFC 3414).
//!
//! [`NotificationReceiver`] reads one datagram at a time and returns the
//! [`Notification`] in it and, when the sender expects one, the reply to
//! send back: a Response that acknowledges an INFORM, or a Report for an
//! SNMPv3 sender that is discovering this receiver's engine ID. It does no
//! networking itself; the `trap-rx` crate puts it behind a UDP socket.
//!
//! An SNMPv1 trap is translated to the SNMPv2 form (RFC 3584, 3.1), so every
//! notification looks the same afterwards: `sysUpTime.0`, `snmpTrapOID.0`,
//! then the objects.
//!
//! For SNMPv3 the sender of a trap is the authoritative engine, so a trap
//! user's keys are localized with the sender's engine ID; for an INFORM the
//! receiver is authoritative, so the keys are localized with this
//! receiver's own engine ID, which the sender discovers first.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::time::Instant;

use mib_core::Oid;
use snmp2::AsnReader;

use crate::codec::{self, SEQUENCE, integer, read_integer, tlv};
use crate::error::SecurityProblem;
use crate::target::Version;
use crate::usm::{Keys, SecurityLevel, Usm, hex};
use crate::v3::{self, EngineTime, FLAG_AUTH, FLAG_PRIV, FLAG_REPORTABLE};
use crate::value::{Value, Varbind, tag};

const TRAP_V1: u8 = 0xa4;
const INFORM_REQUEST: u8 = 0xa6;
const TRAP_V2: u8 = 0xa7;

/// `sysUpTime.0`, the first variable of every SNMPv2 notification.
pub const SYS_UP_TIME_0: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 3, 0];
/// `snmpTrapOID.0`, the second: which notification this is.
pub const SNMP_TRAP_OID_0: [u32; 11] = [1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0];
/// `snmpTrapEnterprise.0`: the enterprise of a translated SNMPv1 trap.
pub const SNMP_TRAP_ENTERPRISE_0: [u32; 11] = [1, 3, 6, 1, 6, 3, 1, 1, 4, 3, 0];
/// `snmpTrapAddress.0`: the agent address of a translated SNMPv1 trap.
pub const SNMP_TRAP_ADDRESS_0: [u32; 10] = [1, 3, 6, 1, 6, 3, 18, 1, 3, 0];
/// `snmpTrapCommunity.0`: the community of a translated SNMPv1 trap.
pub const SNMP_TRAP_COMMUNITY_0: [u32; 10] = [1, 3, 6, 1, 6, 3, 18, 1, 4, 0];
/// `snmpTraps`: the generic SNMPv1 traps are `snmpTraps.1` (coldStart) to
/// `snmpTraps.6` (egpNeighborLoss).
pub const SNMP_TRAPS: [u32; 9] = [1, 3, 6, 1, 6, 3, 1, 1, 5];

/// Whether the sender waits for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    /// A trap (SNMPv1 Trap-PDU or SNMPv2-Trap-PDU): not answered.
    Trap,
    /// An InformRequest: answered with a Response, or sent again.
    Inform,
}

impl NotificationKind {
    /// `"trap"` or `"inform"`.
    pub fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Trap => "trap",
            NotificationKind::Inform => "inform",
        }
    }
}

impl fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The fields of an SNMPv1 Trap-PDU (RFC 1157, 4.1.6), as received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Trap {
    /// The `enterprise` field: the kind of device that sent the trap.
    pub enterprise: Oid,
    /// The `agent-addr` field: the address the agent says it has, which can
    /// differ from the packet's source (NAT, proxies, several interfaces).
    pub agent_address: [u8; 4],
    /// `generic-trap`: 0 coldStart, 1 warmStart, 2 linkDown, 3 linkUp,
    /// 4 authenticationFailure, 5 egpNeighborLoss, 6 enterpriseSpecific.
    pub generic_trap: i64,
    /// `specific-trap`: the enterprise's own trap number, when generic-trap
    /// is 6.
    pub specific_trap: i64,
    /// `time-stamp`: the agent's sysUpTime, in hundredths of a second.
    pub time_stamp: u32,
    /// How many variable bindings the trap itself carried. They are
    /// [`Notification::varbinds`]`[2..2 + varbind_count]`.
    pub varbind_count: usize,
}

impl V1Trap {
    /// The name of the generic-trap value, such as `"linkDown"`.
    pub fn generic_name(&self) -> Option<&'static str> {
        Some(match self.generic_trap {
            0 => "coldStart",
            1 => "warmStart",
            2 => "linkDown",
            3 => "linkUp",
            4 => "authenticationFailure",
            5 => "egpNeighborLoss",
            6 => "enterpriseSpecific",
            _ => return None,
        })
    }

    /// The SNMPv2 notification OID for this trap (RFC 3584, 3.1):
    /// `snmpTraps.(generic + 1)` for the generic traps, or
    /// `enterprise.0.specific` for an enterpriseSpecific one.
    pub fn notification_oid(&self) -> Oid {
        match self.generic_trap {
            0..=5 => Oid::new(SNMP_TRAPS.to_vec()).child(self.generic_trap as u32 + 1),
            _ => self
                .enterprise
                .child(0)
                .child(u32::try_from(self.specific_trap).unwrap_or(0)),
        }
    }
}

/// Who sent a notification, and how it was protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationSecurity {
    /// SNMPv1 or SNMPv2c: the community string. Communities are not
    /// checked; every one is accepted and shown.
    Community(Vec<u8>),
    /// SNMPv3: the user, already checked.
    Usm(UsmSecurity),
}

/// The SNMPv3 security of a received notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsmSecurity {
    /// The user name.
    pub user: String,
    /// The level the message was sent with.
    pub level: SecurityLevel,
    /// The authoritative engine: the sender's for a trap, this receiver's
    /// for an INFORM.
    pub engine_id: Vec<u8>,
    /// Engine boots of the authoritative engine, from the message.
    pub engine_boots: u32,
    /// Engine time of the authoritative engine, from the message.
    pub engine_time: u32,
    /// `contextEngineID`: the engine that produced the notification, which
    /// for a trap is normally the sender's engine ID.
    pub context_engine_id: Vec<u8>,
    /// `contextName`, usually empty.
    pub context_name: Vec<u8>,
}

/// A received trap or INFORM, in SNMPv2 form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The SNMP version of the message.
    pub version: Version,
    /// Trap or INFORM.
    pub kind: NotificationKind,
    /// Community or SNMPv3 user.
    pub security: NotificationSecurity,
    /// The request-id of the PDU (0 in an SNMPv1 trap, which has none).
    pub request_id: i32,
    /// The original fields of an SNMPv1 trap.
    pub v1: Option<V1Trap>,
    /// The variable bindings: `sysUpTime.0`, `snmpTrapOID.0`, then the
    /// objects. For an SNMPv1 trap these are built by the translation of
    /// RFC 3584, 3.1, which also appends `snmpTrapAddress.0`,
    /// `snmpTrapCommunity.0` and `snmpTrapEnterprise.0` unless the trap
    /// already carried them.
    pub varbinds: Vec<Varbind>,
}

impl Notification {
    /// Which notification this is: the value of `snmpTrapOID.0`, or `None`
    /// if a broken sender left it out.
    pub fn trap_oid(&self) -> Option<&Oid> {
        self.varbinds
            .iter()
            .find(|v| v.oid.arcs() == SNMP_TRAP_OID_0)
            .and_then(|v| match &v.value {
                Value::ObjectIdentifier(oid) => Some(oid),
                _ => None,
            })
    }

    /// The sender's sysUpTime in hundredths of a second, from `sysUpTime.0`.
    pub fn uptime(&self) -> Option<u32> {
        self.varbinds
            .iter()
            .find(|v| v.oid.arcs() == SYS_UP_TIME_0)
            .and_then(|v| match v.value {
                Value::TimeTicks(ticks) => Some(ticks),
                _ => None,
            })
    }

    /// The objects the notification carries: every variable binding except
    /// `sysUpTime.0` and `snmpTrapOID.0`, and for an SNMPv1 trap only the
    /// ones the trap itself carried (its other fields are in
    /// [`v1`](Self::v1)).
    pub fn objects(&self) -> Vec<&Varbind> {
        if let Some(v1) = &self.v1 {
            return self
                .varbinds
                .iter()
                .skip(2)
                .take(v1.varbind_count)
                .collect();
        }
        self.varbinds
            .iter()
            .filter(|v| v.oid.arcs() != SYS_UP_TIME_0 && v.oid.arcs() != SNMP_TRAP_OID_0)
            .collect()
    }
}

/// An SNMPv3 user that may send notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapUser {
    /// User name, protocols and passwords. The context fields are not used.
    pub usm: Usm,
    /// The engine ID of the sender this user belongs to, as net-snmp's
    /// `createUser -e`. `None` accepts traps from any engine that knows the
    /// passwords. INFORMs always use this receiver's engine ID.
    pub engine_id: Option<Vec<u8>>,
}

impl TrapUser {
    /// A user for traps from any engine.
    pub fn new(usm: Usm) -> TrapUser {
        TrapUser {
            usm,
            engine_id: None,
        }
    }

    /// Accept traps from this user only from the engine with this ID.
    pub fn with_engine_id(mut self, engine_id: impl Into<Vec<u8>>) -> TrapUser {
        self.engine_id = Some(engine_id.into());
        self
    }
}

/// Why a datagram did not yield a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveError {
    /// Not a valid SNMP message.
    Malformed(String),
    /// A valid message, but not a notification, such as a GetRequest.
    NotNotification {
        /// The SNMP version of the message.
        version: Version,
        /// The PDU type, such as `"GetRequest"`.
        pdu: &'static str,
    },
    /// An SNMPv3 message that failed the security checks.
    Security {
        /// The user name in the message.
        user: String,
        /// The authoritative engine ID in the message.
        engine_id: Vec<u8>,
        /// What failed.
        problem: SecurityProblem,
    },
}

impl fmt::Display for ReceiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReceiveError::Malformed(reason) => write!(f, "not a valid SNMP message: {reason}"),
            ReceiveError::NotNotification { version, pdu } => {
                write!(f, "an {version} {pdu}, not a trap or INFORM; ignored")
            }
            ReceiveError::Security {
                user,
                engine_id,
                problem,
            } => {
                let engine = if engine_id.is_empty() {
                    "no engine ID".to_string()
                } else {
                    format!("engine ID {}", hex(engine_id))
                };
                let what = match problem {
                    SecurityProblem::UnknownUser => "no trap user has this name".to_string(),
                    SecurityProblem::UnknownEngineId => {
                        "the trap user with this name is bound to another engine ID".to_string()
                    }
                    SecurityProblem::WrongDigest => {
                        "wrong authentication code: the authentication password or protocol \
                         differs"
                            .to_string()
                    }
                    SecurityProblem::DecryptionError => {
                        "cannot decrypt: the privacy password or protocol differs".to_string()
                    }
                    SecurityProblem::UnsupportedSecurityLevel => {
                        "the security level does not match the trap user's".to_string()
                    }
                    SecurityProblem::NotInTimeWindow => {
                        "the INFORM's engine time is not in the time window; a Report with the \
                         right time was sent, and the sender normally sends it again"
                            .to_string()
                    }
                    other => format!("{other:?}"),
                };
                write!(f, "SNMPv3 user {user:?} ({engine}): {what}")
            }
        }
    }
}

impl std::error::Error for ReceiveError {}

impl ReceiveError {
    /// What to change, for the security problems.
    pub fn hint(&self) -> Option<String> {
        let ReceiveError::Security {
            user,
            engine_id,
            problem,
        } = self
        else {
            return None;
        };
        Some(match problem {
            SecurityProblem::UnknownUser | SecurityProblem::UnknownEngineId => format!(
                "add the user as the sender has it, such as `createUser -e {} {user} SHA \
                 <auth password> AES <priv password>` in the users file; without -e the \
                 user is accepted from any engine",
                if engine_id.is_empty() {
                    "<engine ID>".to_string()
                } else {
                    hex(engine_id)
                }
            ),
            SecurityProblem::WrongDigest | SecurityProblem::DecryptionError => {
                "use the same protocols and passwords as the sender (net-snmp: the \
                 trapsess or snmptrap -a/-A/-x/-X options)"
                    .to_string()
            }
            SecurityProblem::UnsupportedSecurityLevel => {
                "a user with an authentication password only accepts authenticated \
                 notifications (authNoPriv or authPriv), a user without one only \
                 noAuthNoPriv, and authPriv needs a privacy password"
                    .to_string()
            }
            _ => return None,
        })
    }
}

/// What came of one datagram.
#[derive(Debug)]
pub struct Handled {
    /// The notification, an error, or `Ok(None)` for a message that only
    /// needed a reply (SNMPv3 discovery).
    pub result: Result<Option<Notification>, ReceiveError>,
    /// The datagram to send back to the sender, if any.
    pub reply: Option<Vec<u8>>,
}

impl Handled {
    fn error(error: ReceiveError) -> Handled {
        Handled {
            result: Err(error),
            reply: None,
        }
    }
}

/// Counters of the Reports this receiver can send (RFC 3414, `usmStats`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Stats {
    unsupported_levels: u32,
    not_in_time_windows: u32,
    unknown_users: u32,
    unknown_engine_ids: u32,
    wrong_digests: u32,
    decryption_errors: u32,
}

/// The engine time window of RFC 3414, 3.2 step 7: 150 seconds.
const TIME_WINDOW: u32 = 150;

/// Turns datagrams into notifications and replies; see the module
/// documentation.
pub struct NotificationReceiver {
    engine_id: Vec<u8>,
    boots: u32,
    started: Instant,
    /// Added to the seconds since `started`; tests move the clock with it.
    time_offset: u32,
    users: Vec<TrapUser>,
    /// Localized keys per user (index into `users`) and engine ID. Making
    /// them hashes a megabyte per password, so they are kept.
    keys: HashMap<(usize, Vec<u8>), Keys>,
    stats: Stats,
    next_salt: u64,
}

impl fmt::Debug for NotificationReceiver {
    // The users' passwords and keys are left out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificationReceiver")
            .field("engine_id", &hex(&self.engine_id))
            .field("boots", &self.boots)
            .field("users", &self.users.len())
            .finish_non_exhaustive()
    }
}

impl NotificationReceiver {
    /// A receiver with the given SNMPv3 users. Its engine ID is new and
    /// random (see [`new_engine_id`]), with engine boots 1.
    pub fn new(users: Vec<TrapUser>) -> NotificationReceiver {
        NotificationReceiver::with_engine_id(new_engine_id(), users)
    }

    /// A receiver with a fixed engine ID, so SNMPv3 senders of INFORMs can
    /// be configured with it. Engine boots is 1: a sender that remembers
    /// the time from an earlier run gets a `notInTimeWindow` Report and
    /// synchronizes again.
    pub fn with_engine_id(engine_id: Vec<u8>, users: Vec<TrapUser>) -> NotificationReceiver {
        let seed = RandomState::new().build_hasher().finish();
        NotificationReceiver {
            engine_id,
            boots: 1,
            started: Instant::now(),
            time_offset: 0,
            users,
            keys: HashMap::new(),
            stats: Stats::default(),
            next_salt: seed,
        }
    }

    /// This receiver's engine ID (`snmpEngineID`), which SNMPv3 INFORMs
    /// are sent to.
    pub fn engine_id(&self) -> &[u8] {
        &self.engine_id
    }

    /// The SNMPv3 users.
    pub fn users(&self) -> &[TrapUser] {
        &self.users
    }

    fn engine_time(&self) -> u32 {
        let elapsed = u32::try_from(self.started.elapsed().as_secs()).unwrap_or(u32::MAX);
        self.time_offset.saturating_add(elapsed)
    }

    fn own_engine(&self) -> EngineTime {
        EngineTime {
            engine_id: self.engine_id.clone(),
            boots: self.boots,
            time: self.engine_time(),
        }
    }

    fn salt(&mut self) -> u64 {
        self.next_salt = self.next_salt.wrapping_add(1);
        self.next_salt
    }

    /// Read one datagram.
    pub fn handle(&mut self, bytes: &[u8]) -> Handled {
        let version = match peek_version(bytes) {
            Ok(version) => version,
            Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
        };
        match version {
            0 | 1 => self.handle_community(bytes),
            3 => self.handle_v3(bytes),
            other => Handled::error(ReceiveError::Malformed(format!(
                "SNMP version field {other}; expected 0 (SNMPv1), 1 (SNMPv2c) or 3 (SNMPv3)"
            ))),
        }
    }

    fn handle_community(&mut self, bytes: &[u8]) -> Handled {
        let decoded = (|| -> Result<_, String> {
            let asn = |e: snmp2::Error| e.to_string();
            let content = AsnReader::from_bytes(bytes)
                .read_raw(SEQUENCE)
                .map_err(asn)?;
            let mut reader = AsnReader::from_bytes(content);
            let version = if read_integer(&mut reader)? == 0 {
                Version::V1
            } else {
                Version::V2c
            };
            let community = reader.read_raw(tag::OCTET_STRING).map_err(asn)?.to_vec();
            let pdu_tag = reader.peek_byte().map_err(asn)?;
            let pdu = reader.read_raw(pdu_tag).map_err(asn)?;
            Ok((version, community, pdu_tag, pdu))
        })();
        let (version, community, pdu_tag, pdu) = match decoded {
            Ok(decoded) => decoded,
            Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
        };
        match (version, pdu_tag) {
            (Version::V1, TRAP_V1) => match decode_v1_trap(pdu, &community) {
                Ok(notification) => Handled {
                    result: Ok(Some(notification)),
                    reply: None,
                },
                Err(reason) => Handled::error(ReceiveError::Malformed(reason)),
            },
            (Version::V2c, TRAP_V2 | INFORM_REQUEST) => {
                let (request_id, varbinds) = match decode_v2_pdu(pdu) {
                    Ok(decoded) => decoded,
                    Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
                };
                let kind = kind_of(pdu_tag);
                let reply = (kind == NotificationKind::Inform)
                    .then(|| inform_response(request_id, &varbinds))
                    .map(|pdu| {
                        let mut message = integer(1);
                        message.extend(tlv(tag::OCTET_STRING, &community));
                        message.extend(pdu);
                        tlv(SEQUENCE, &message)
                    });
                Handled {
                    result: Ok(Some(Notification {
                        version,
                        kind,
                        security: NotificationSecurity::Community(community),
                        request_id,
                        v1: None,
                        varbinds,
                    })),
                    reply,
                }
            }
            (version, pdu_tag) => Handled::error(ReceiveError::NotNotification {
                version,
                pdu: pdu_name(pdu_tag),
            }),
        }
    }

    fn handle_v3(&mut self, bytes: &[u8]) -> Handled {
        let message = match v3::decode_message(bytes) {
            Ok(message) => message,
            Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
        };
        let user_name = String::from_utf8_lossy(message.user).into_owned();
        let security_error = |problem: SecurityProblem| ReceiveError::Security {
            user: user_name.clone(),
            engine_id: message.engine.engine_id.clone(),
            problem,
        };
        let reportable = message.flags & FLAG_REPORTABLE != 0;
        let level = match (
            message.flags & FLAG_AUTH != 0,
            message.flags & FLAG_PRIV != 0,
        ) {
            (false, false) => SecurityLevel::NoAuthNoPriv,
            (true, false) => SecurityLevel::AuthNoPriv,
            (true, true) => SecurityLevel::AuthPriv,
            (false, true) => {
                return Handled::error(ReceiveError::Malformed(
                    "msgFlags ask for privacy without authentication".to_string(),
                ));
            }
        };
        let own = message.engine.engine_id == self.engine_id;

        // A confirmed message (an INFORM, or discovery before one) must be
        // sent to this receiver's engine (RFC 3414, 3.2 step 3). Discovery
        // sends an empty engine ID and gets a Report with ours.
        if reportable && !own {
            self.stats.unknown_engine_ids += 1;
            let reply = self.report(&message, usm_stats(4), self.stats.unknown_engine_ids, None);
            return Handled {
                result: if message.engine.engine_id.is_empty() {
                    Ok(None)
                } else {
                    Err(security_error(SecurityProblem::UnknownEngineId))
                },
                reply: Some(reply),
            };
        }

        // The user, for the authoritative engine of the message.
        let candidates: Vec<usize> = (0..self.users.len())
            .filter(|&i| self.users[i].usm.user.as_bytes() == message.user)
            .collect();
        let bound = |i: &usize| match &self.users[*i].engine_id {
            Some(id) => !own && *id == message.engine.engine_id,
            None => false,
        };
        let Some(index) = candidates
            .iter()
            .copied()
            .find(bound)
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .find(|&i| self.users[i].engine_id.is_none())
            })
            .or_else(|| {
                // A user bound to the sender's engine also sends INFORMs,
                // which use this receiver's engine.
                own.then(|| candidates.first().copied()).flatten()
            })
        else {
            let problem = if candidates.is_empty() {
                SecurityProblem::UnknownUser
            } else {
                SecurityProblem::UnknownEngineId
            };
            self.stats.unknown_users += 1;
            let reply = reportable
                .then(|| self.report(&message, usm_stats(3), self.stats.unknown_users, None));
            return Handled {
                result: Err(security_error(problem)),
                reply,
            };
        };

        let user_level = self.users[index].usm.level;
        let level_ok = match user_level {
            SecurityLevel::NoAuthNoPriv => level == SecurityLevel::NoAuthNoPriv,
            SecurityLevel::AuthNoPriv => level == SecurityLevel::AuthNoPriv,
            SecurityLevel::AuthPriv => level.has_auth(),
        };
        if !level_ok {
            self.stats.unsupported_levels += 1;
            let reply = reportable
                .then(|| self.report(&message, usm_stats(1), self.stats.unsupported_levels, None));
            return Handled {
                result: Err(security_error(SecurityProblem::UnsupportedSecurityLevel)),
                reply,
            };
        }

        let keys = self
            .keys
            .entry((index, message.engine.engine_id.clone()))
            .or_insert_with(|| Keys::derive(&self.users[index].usm, &message.engine.engine_id))
            .clone();
        let usm = self.users[index].usm.clone();

        if level.has_auth() {
            let mut zeroed = bytes.to_vec();
            zeroed[message.auth_range.clone()].fill(0);
            let expected = usm.auth_protocol.mac(&keys.auth, &zeroed);
            if !v3::same(&expected, message.auth) {
                self.stats.wrong_digests += 1;
                let reply = reportable
                    .then(|| self.report(&message, usm_stats(5), self.stats.wrong_digests, None));
                return Handled {
                    result: Err(security_error(SecurityProblem::WrongDigest)),
                    reply,
                };
            }
            // The time window applies where this receiver is the
            // authoritative engine: INFORMs (RFC 3414, 3.2 step 7a). The
            // sender's clock is not checked for traps.
            if own {
                let now = self.engine_time();
                if message.engine.boots != self.boots
                    || message.engine.time.abs_diff(now) > TIME_WINDOW
                {
                    self.stats.not_in_time_windows += 1;
                    // Authenticated, so the sender believes the time in it.
                    let reply = reportable.then(|| {
                        self.report(
                            &message,
                            usm_stats(2),
                            self.stats.not_in_time_windows,
                            Some((&usm, &keys)),
                        )
                    });
                    return Handled {
                        result: Err(security_error(SecurityProblem::NotInTimeWindow)),
                        reply,
                    };
                }
            }
        }

        let plaintext;
        let scoped = if level.has_priv() {
            let decrypted = usm.priv_protocol.decrypt(
                &keys.privacy,
                message.engine.boots,
                message.engine.time,
                message.privacy,
                message.data,
            );
            match decrypted {
                Ok(data) => {
                    plaintext = data;
                    plaintext.as_slice()
                }
                Err(_) => return self.decryption_error(&message, reportable, security_error),
            }
        } else {
            message.data
        };
        let (context_engine_id, context_name, pdu_tag, pdu) = match read_scoped_pdu(scoped) {
            Ok(parts) => parts,
            Err(reason) if level.has_priv() => {
                // Garbage after decryption: the keys differ.
                let _ = reason;
                return self.decryption_error(&message, reportable, security_error);
            }
            Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
        };
        if pdu_tag != TRAP_V2 && pdu_tag != INFORM_REQUEST {
            return Handled::error(ReceiveError::NotNotification {
                version: Version::V3,
                pdu: pdu_name(pdu_tag),
            });
        }
        let (request_id, varbinds) = match decode_v2_pdu(pdu) {
            Ok(decoded) => decoded,
            Err(reason) => return Handled::error(ReceiveError::Malformed(reason)),
        };
        let kind = kind_of(pdu_tag);
        let reply = (kind == NotificationKind::Inform).then(|| {
            let scoped = v3::scoped_pdu(
                &context_engine_id,
                &context_name,
                &inform_response(request_id, &varbinds),
            );
            let engine = self.own_engine();
            let salt = self.salt();
            v3::seal(
                &usm,
                level,
                &keys,
                message.msg_id,
                0,
                &engine,
                &scoped,
                salt,
            )
        });
        Handled {
            result: Ok(Some(Notification {
                version: Version::V3,
                kind,
                security: NotificationSecurity::Usm(UsmSecurity {
                    user: user_name.clone(),
                    level,
                    engine_id: message.engine.engine_id.clone(),
                    engine_boots: message.engine.boots,
                    engine_time: message.engine.time,
                    context_engine_id,
                    context_name,
                }),
                request_id,
                v1: None,
                varbinds,
            })),
            reply,
        }
    }

    fn decryption_error(
        &mut self,
        message: &v3::Message<'_>,
        reportable: bool,
        security_error: impl Fn(SecurityProblem) -> ReceiveError,
    ) -> Handled {
        self.stats.decryption_errors += 1;
        let reply = reportable
            .then(|| self.report(message, usm_stats(6), self.stats.decryption_errors, None));
        Handled {
            result: Err(security_error(SecurityProblem::DecryptionError)),
            reply,
        }
    }

    /// A Report PDU with one usmStats counter (RFC 3412, 7.1 step 3; RFC
    /// 3414, 3.2), from this receiver's engine. Unauthenticated, except
    /// `notInTimeWindow`, which is sent with the user's authentication so
    /// the sender can trust the time in it.
    fn report(
        &mut self,
        message: &v3::Message<'_>,
        counter: Oid,
        count: u32,
        auth: Option<(&Usm, &Keys)>,
    ) -> Vec<u8> {
        // The request-id is readable unless the request was encrypted.
        let request_id = if message.flags & FLAG_PRIV == 0 {
            read_scoped_pdu(message.data)
                .ok()
                .and_then(|(_, _, _, pdu)| {
                    read_integer(&mut AsnReader::from_bytes(pdu))
                        .ok()
                        .and_then(|id| i32::try_from(id).ok())
                })
                .unwrap_or(0)
        } else {
            0
        };
        let varbinds = [Varbind {
            oid: counter,
            value: Value::Counter32(count),
        }];
        let mut pdu = integer(i64::from(request_id));
        pdu.extend(integer(0));
        pdu.extend(integer(0));
        pdu.extend(codec::encode_varbinds(&varbinds).expect("valid OIDs"));
        let pdu = tlv(codec::REPORT, &pdu);
        let scoped = v3::scoped_pdu(&self.engine_id, b"", &pdu);
        let engine = self.own_engine();
        match auth {
            Some((usm, keys)) => {
                let salt = self.salt();
                v3::seal(
                    usm,
                    SecurityLevel::AuthNoPriv,
                    keys,
                    message.msg_id,
                    0,
                    &engine,
                    &scoped,
                    salt,
                )
            }
            None => v3::message(
                message.msg_id,
                0,
                &v3::security_parameters(&engine, message.user, &[], &[]),
                &scoped,
            ),
        }
    }
}

/// A new engine ID for a receiver: RFC 3411 format 5 (octets) with the
/// enterprise number 0 (none), the text `ferosnmp` and 8 random bytes, so
/// two receivers never share one.
pub fn new_engine_id() -> Vec<u8> {
    let mut id = vec![0x80, 0x00, 0x00, 0x00, 0x05];
    id.extend_from_slice(b"ferosnmp");
    let random = RandomState::new().build_hasher().finish();
    id.extend_from_slice(&random.to_be_bytes());
    id
}

fn usm_stats(counter: u32) -> Oid {
    Oid::new(vec![1, 3, 6, 1, 6, 3, 15, 1, 1, counter, 0])
}

fn kind_of(pdu_tag: u8) -> NotificationKind {
    if pdu_tag == INFORM_REQUEST {
        NotificationKind::Inform
    } else {
        NotificationKind::Trap
    }
}

fn pdu_name(pdu_tag: u8) -> &'static str {
    match pdu_tag {
        0xa0 => "GetRequest",
        0xa1 => "GetNextRequest",
        0xa2 => "Response",
        0xa3 => "SetRequest",
        TRAP_V1 => "SNMPv1 Trap",
        0xa5 => "GetBulkRequest",
        INFORM_REQUEST => "InformRequest",
        TRAP_V2 => "SNMPv2-Trap",
        0xa8 => "Report",
        _ => "unknown PDU",
    }
}

/// The version field of a message.
fn peek_version(bytes: &[u8]) -> Result<i64, String> {
    let content = AsnReader::from_bytes(bytes)
        .read_raw(SEQUENCE)
        .map_err(|e| e.to_string())?;
    read_integer(&mut AsnReader::from_bytes(content))
}

/// The Response that acknowledges an INFORM: the same request-id and
/// variable bindings (RFC 3416, 4.2.7).
fn inform_response(request_id: i32, varbinds: &[Varbind]) -> Vec<u8> {
    let mut pdu = integer(i64::from(request_id));
    pdu.extend(integer(0));
    pdu.extend(integer(0));
    // The variables came in as valid OIDs, so they go out as well; an OID
    // value that cannot be encoded again (a first arc above 2) is answered
    // with an empty list rather than not at all.
    pdu.extend(codec::encode_varbinds(varbinds).unwrap_or_else(|_| tlv(SEQUENCE, &[])));
    tlv(codec::RESPONSE, &pdu)
}

/// Request-id and variable bindings of an SNMPv2-Trap or InformRequest PDU
/// (contents, without tag and length).
fn decode_v2_pdu(pdu: &[u8]) -> Result<(i32, Vec<Varbind>), String> {
    let mut reader = AsnReader::from_bytes(pdu);
    let request_id = read_integer(&mut reader)?;
    let request_id = i32::try_from(request_id).map_err(|_| "request-id out of range")?;
    read_integer(&mut reader)?;
    read_integer(&mut reader)?;
    let varbinds = codec::decode_varbinds(&mut reader)?;
    Ok((request_id, varbinds))
}

/// Read an SNMPv1 Trap-PDU and translate it (RFC 3584, 3.1).
fn decode_v1_trap(pdu: &[u8], community: &[u8]) -> Result<Notification, String> {
    let asn = |e: snmp2::Error| e.to_string();
    let mut reader = AsnReader::from_bytes(pdu);
    let enterprise = codec::decode_oid(reader.read_raw(tag::OBJECT_IDENTIFIER).map_err(asn)?)?;
    let address = reader.read_raw(tag::IP_ADDRESS).map_err(asn)?;
    let agent_address = <[u8; 4]>::try_from(address)
        .map_err(|_| format!("agent-addr has {} bytes instead of 4", address.len()))?;
    let generic_trap = read_integer(&mut reader)?;
    let specific_trap = read_integer(&mut reader)?;
    let time_stamp = match codec::decode_value(
        tag::TIME_TICKS,
        reader.read_raw(tag::TIME_TICKS).map_err(asn)?,
    ) {
        Value::TimeTicks(ticks) => ticks,
        _ => return Err("time-stamp is not a valid TimeTicks value".to_string()),
    };
    let original = codec::decode_varbinds(&mut reader)?;
    let v1 = V1Trap {
        enterprise,
        agent_address,
        generic_trap,
        specific_trap,
        time_stamp,
        varbind_count: original.len(),
    };

    let mut varbinds = vec![
        Varbind {
            oid: Oid::new(SYS_UP_TIME_0.to_vec()),
            value: Value::TimeTicks(time_stamp),
        },
        Varbind {
            oid: Oid::new(SNMP_TRAP_OID_0.to_vec()),
            value: Value::ObjectIdentifier(v1.notification_oid()),
        },
    ];
    varbinds.extend(original);
    for (oid, value) in [
        (&SNMP_TRAP_ADDRESS_0[..], Value::IpAddress(v1.agent_address)),
        (
            &SNMP_TRAP_COMMUNITY_0[..],
            Value::OctetString(community.to_vec()),
        ),
        (
            &SNMP_TRAP_ENTERPRISE_0[..],
            Value::ObjectIdentifier(v1.enterprise.clone()),
        ),
    ] {
        if !varbinds.iter().any(|v| v.oid.arcs() == oid) {
            varbinds.push(Varbind {
                oid: Oid::new(oid.to_vec()),
                value,
            });
        }
    }
    Ok(Notification {
        version: Version::V1,
        kind: NotificationKind::Trap,
        security: NotificationSecurity::Community(community.to_vec()),
        request_id: 0,
        v1: Some(v1),
        varbinds,
    })
}

/// The parts of a ScopedPDU: context engine ID, context name, PDU tag and
/// PDU contents.
type ScopedPdu<'a> = (Vec<u8>, Vec<u8>, u8, &'a [u8]);

/// Read a ScopedPDU. Anything after it is DES padding.
fn read_scoped_pdu(bytes: &[u8]) -> Result<ScopedPdu<'_>, String> {
    let asn = |e: snmp2::Error| e.to_string();
    let content = AsnReader::from_bytes(bytes)
        .read_raw(SEQUENCE)
        .map_err(asn)?;
    let mut reader = AsnReader::from_bytes(content);
    let context_engine_id = reader.read_raw(tag::OCTET_STRING).map_err(asn)?.to_vec();
    let context_name = reader.read_raw(tag::OCTET_STRING).map_err(asn)?.to_vec();
    let pdu_tag = reader.peek_byte().map_err(asn)?;
    let pdu = reader.read_raw(pdu_tag).map_err(asn)?;
    Ok((context_engine_id, context_name, pdu_tag, pdu))
}

#[cfg(test)]
impl NotificationReceiver {
    /// Pretend the receiver has been running for `seconds`.
    pub(crate) fn set_engine_time(&mut self, seconds: u32) {
        self.started = Instant::now();
        self.time_offset = seconds;
    }
}

#[cfg(test)]
#[path = "tests_notify.rs"]
mod tests;
