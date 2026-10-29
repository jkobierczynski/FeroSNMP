//! Errors from SNMP operations.

use std::fmt;
use std::time::Duration;

use mib_core::Oid;

use crate::usm::SecurityLevel;

/// Why an SNMP operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The agent's host name could not be resolved to an address.
    #[error("cannot find the address of {host}: {message}")]
    Resolve {
        /// The host name as given.
        host: String,
        /// What the resolver said.
        message: String,
    },

    /// Sending or receiving failed at the socket level.
    #[error("network error with {target}: {message}")]
    Network {
        /// The agent, as `host:port`.
        target: String,
        /// What went wrong.
        message: String,
    },

    /// No response arrived, not even after retrying.
    #[error(
        "no response from {target} after {attempts} attempt{s} of {timeout} each \
         (check the address, the port, the community or SNMPv3 user, and any firewall)",
        s = if *attempts == 1 { "" } else { "s" },
        timeout = SecondsText(*timeout)
    )]
    Timeout {
        /// The agent, as `host:port`.
        target: String,
        /// Requests sent, including retries.
        attempts: u32,
        /// How long each attempt waited.
        timeout: Duration,
    },

    /// The response could not be decoded.
    #[error("cannot decode the response from {target}: {message}")]
    Decode {
        /// The agent, as `host:port`.
        target: String,
        /// What was wrong with it.
        message: String,
    },

    /// The agent answered with an error status, such as `noSuchName`.
    #[error("{target} answered {status}{}", oid.as_ref().map(|o| format!(" for {o}")).unwrap_or_default())]
    Agent {
        /// The agent, as `host:port`.
        target: String,
        /// The error status from the response.
        status: ErrorStatus,
        /// The error index from the response (1 is the first variable).
        index: u32,
        /// The requested OID the error index points at, if it is valid.
        oid: Option<Oid>,
    },

    /// A walk received an OID that is not greater than the one before it.
    /// Following it could loop forever, so the walk stops.
    #[error(
        "the agent returned {next} after {previous}; OIDs must increase, so the walk \
         stopped here to avoid an endless loop (the agent is broken)"
    )]
    NotIncreasing {
        /// The last OID received before the problem.
        previous: Oid,
        /// The OID that is not greater than `previous`.
        next: Oid,
    },

    /// An OID that cannot be sent, such as one with fewer than two arcs.
    #[error("cannot send OID {oid:?}: {reason}")]
    InvalidOid {
        /// The OID as text.
        oid: String,
        /// Why it cannot be sent.
        reason: String,
    },

    /// An SNMPv3 security problem: the agent refused the request with a
    /// Report, or its answer did not pass our checks, or the settings are
    /// unusable.
    #[error("{target}: {}", problem.explain(user, *level))]
    Security {
        /// The agent, as `host:port`.
        target: String,
        /// The SNMPv3 user name.
        user: String,
        /// The security level asked for.
        level: SecurityLevel,
        /// What went wrong.
        problem: SecurityProblem,
    },

    /// GETBULK was requested with SNMPv1, which does not have it.
    #[error("GETBULK needs SNMPv2c; SNMPv1 only has GET and GETNEXT")]
    BulkNeedsV2,

    /// Engine discovery was requested with SNMPv1 or SNMPv2c.
    #[error("engine discovery needs SNMPv3; SNMPv1 and SNMPv2c have no engine ID")]
    DiscoveryNeedsV3,

    /// The operation was cancelled before it finished.
    #[error("the operation was cancelled")]
    Cancelled,

    /// The engine has shut down and accepts no more operations.
    #[error("the SNMP engine has stopped")]
    EngineStopped,
}

/// What went wrong with SNMPv3 security. Most of these come from the
/// counter in a Report PDU (RFC 3414, section 3.2; RFC 3412, section 7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityProblem {
    /// `usmStatsUnsupportedSecLevels`: the user exists, but not at this
    /// security level.
    UnsupportedSecurityLevel,
    /// `usmStatsNotInTimeWindows`, even after synchronizing the clock with
    /// the agent.
    NotInTimeWindow,
    /// `usmStatsUnknownUserNames`: the agent does not know the user.
    UnknownUser,
    /// `usmStatsUnknownEngineIDs`, even after discovering the engine ID
    /// again.
    UnknownEngineId,
    /// `usmStatsWrongDigests`: the authentication code did not match, so
    /// the authentication password or protocol differs.
    WrongDigest,
    /// `usmStatsDecryptionErrors`: the agent could not decrypt the request,
    /// so the privacy password or protocol differs.
    DecryptionError,
    /// The agent ignored an encrypted request but answered the same user
    /// without privacy: the privacy password or protocol differs.
    PrivacyMismatch,
    /// `snmpUnknownContexts`: no such context name.
    UnknownContext,
    /// `snmpUnavailableContexts`.
    UnavailableContext,
    /// `snmpUnknownSecurityModels`.
    UnknownSecurityModel,
    /// `snmpInvalidMsgs`.
    InvalidMessage,
    /// `snmpUnknownPDUHandlers`.
    UnknownPduHandler,
    /// A Report with another (or no) counter.
    OtherReport(Option<Oid>),
    /// The response had no authentication code, although one was asked for.
    ResponseNotAuthenticated,
    /// The response's authentication code was wrong.
    ResponseWrongDigest,
    /// The response could not be decrypted.
    ResponseDecryption(String),
    /// Discovery returned no engine ID.
    NoEngineId,
    /// The settings cannot be used, such as a password that is too short.
    InvalidSettings(String),
}

impl SecurityProblem {
    /// The counter name from the Report, such as `usmStatsWrongDigests`.
    pub fn counter(&self) -> Option<&'static str> {
        Some(match self {
            SecurityProblem::UnsupportedSecurityLevel => "usmStatsUnsupportedSecLevels",
            SecurityProblem::NotInTimeWindow => "usmStatsNotInTimeWindows",
            SecurityProblem::UnknownUser => "usmStatsUnknownUserNames",
            SecurityProblem::UnknownEngineId => "usmStatsUnknownEngineIDs",
            SecurityProblem::WrongDigest => "usmStatsWrongDigests",
            SecurityProblem::DecryptionError => "usmStatsDecryptionErrors",
            SecurityProblem::UnknownContext => "snmpUnknownContexts",
            SecurityProblem::UnavailableContext => "snmpUnavailableContexts",
            SecurityProblem::UnknownSecurityModel => "snmpUnknownSecurityModels",
            SecurityProblem::InvalidMessage => "snmpInvalidMsgs",
            SecurityProblem::UnknownPduHandler => "snmpUnknownPDUHandlers",
            _ => return None,
        })
    }

    /// One or two sentences: what happened, and what to change.
    pub fn explain(&self, user: &str, level: SecurityLevel) -> String {
        let counter = self
            .counter()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        match self {
            SecurityProblem::UnsupportedSecurityLevel => format!(
                "the agent knows user \"{user}\", but not at security level {level}{counter}. \
                 Pick the level the user was set up with: authPriv needs a privacy \
                 password on the agent, authNoPriv needs an authentication password"
            ),
            SecurityProblem::NotInTimeWindow => format!(
                "the agent rejected the request as outside its time window{counter}, even \
                 after synchronizing with its clock. Its engine boots or time may be \
                 broken, or several devices share one engine ID"
            ),
            SecurityProblem::UnknownUser => format!(
                "the agent does not know SNMPv3 user \"{user}\"{counter}. Check the user \
                 name (it is case-sensitive); on net-snmp, users are added with createUser"
            ),
            SecurityProblem::UnknownEngineId => format!(
                "the agent did not accept its own engine ID{counter}, even after \
                 discovering it again"
            ),
            SecurityProblem::WrongDigest => format!(
                "authentication failed for user \"{user}\"{counter}: the authentication \
                 password or protocol differs from the agent's. Check both; SHA means \
                 SHA-1, which is not the same as SHA-256"
            ),
            SecurityProblem::DecryptionError => format!(
                "the agent could not decrypt the request{counter}: the privacy password or \
                 protocol differs from the agent's. For AES-192 and AES-256, also try the \
                 Cisco key extension (AES-192C, AES-256C), or the other way round"
            ),
            SecurityProblem::PrivacyMismatch => format!(
                "the agent did not answer the encrypted request, but does answer user \
                 \"{user}\" without privacy: the privacy password or protocol differs from \
                 the agent's (agents such as net-snmp drop what they cannot decrypt instead \
                 of reporting it). For AES-192 and AES-256, also try the Cisco key \
                 extension (AES-192C, AES-256C), or the other way round"
            ),
            SecurityProblem::UnknownContext => format!(
                "the agent does not know the context name{counter}; leave it empty unless \
                 the device's documentation names one"
            ),
            SecurityProblem::UnavailableContext => {
                format!("the context is not available on the agent{counter}")
            }
            SecurityProblem::UnknownSecurityModel => {
                format!("the agent does not support the User-based Security Model{counter}")
            }
            SecurityProblem::InvalidMessage => format!(
                "the agent says the message was invalid{counter}; this is likely a FeroSNMP \
                 bug, please report it"
            ),
            SecurityProblem::UnknownPduHandler => {
                format!("the agent cannot handle this kind of request{counter}")
            }
            SecurityProblem::OtherReport(Some(oid)) => {
                format!("the agent refused the request with a Report for {oid}")
            }
            SecurityProblem::OtherReport(None) => {
                "the agent refused the request with an empty Report".to_string()
            }
            SecurityProblem::ResponseNotAuthenticated => format!(
                "the response was not authenticated although {level} was asked for, so it \
                 was not accepted"
            ),
            SecurityProblem::ResponseWrongDigest => format!(
                "the response's authentication code is wrong: the agent's key for user \
                 \"{user}\" differs from ours (check the authentication password and \
                 protocol), or the response was changed on the way"
            ),
            SecurityProblem::ResponseDecryption(reason) => format!(
                "cannot decrypt the response: {reason}. The privacy password or protocol \
                 probably differs from the agent's"
            ),
            SecurityProblem::NoEngineId => "the agent did not report an engine ID when asked; \
                it may not support SNMPv3"
                .to_string(),
            SecurityProblem::InvalidSettings(reason) => reason.clone(),
        }
    }
}

/// Formats a duration as "1 s" or "0.5 s".
struct SecondsText(Duration);

impl fmt::Display for SecondsText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} s", self.0.as_secs_f64())
    }
}

/// The error-status field of an SNMP response (RFC 3416, section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorStatus {
    /// 0: no error.
    NoError,
    /// 1: the response would not fit in one message.
    TooBig,
    /// 2: SNMPv1 only: the variable does not exist (or, for GETNEXT, there
    /// is nothing after it).
    NoSuchName,
    /// 3: SNMPv1 only: wrong value in a SET.
    BadValue,
    /// 4: SNMPv1 only: the variable cannot be written.
    ReadOnly,
    /// 5: any other error.
    GenErr,
    /// 6 and up: SNMPv2 SET errors and `authorizationError`, by number.
    Other(u32),
}

impl Error {
    /// Advice that goes beyond the message, for a few errors whose cause
    /// is often not where one looks first.
    pub fn hint(&self) -> Option<&'static str> {
        let Error::Agent { status, .. } = self else {
            return None;
        };
        Some(match status.code() {
            16 => {
                "authorizationError: the agent's access rules do not allow this user, \
                 community or security level to read this. On net-snmp, `rouser NAME priv` \
                 needs authPriv and `rouser NAME auth` needs authNoPriv or more"
            }
            4 | 6 | 17 => {
                "the agent does not let this community or user write here. A read-only \
                 community cannot SET (net-snmp needs `rwcommunity` or `rwuser`, not \
                 `rocommunity`), and many agents keep objects read-only that the MIB \
                 calls read-write"
            }
            3 | 10 => {
                "the agent does not accept this value, although its type is right: it may \
                 support fewer values or a smaller range than the MIB allows"
            }
            7 => {
                "the agent expects another type for this object; check that the loaded MIB \
                 is the one the agent implements, or give the type"
            }
            8 => "the value is longer or shorter than the agent accepts for this object",
            11 => {
                "this instance does not exist and cannot be created: check the index, or \
                 create the row through its RowStatus column (createAndGo or createAndWait)"
            }
            12 => {
                "the value does not fit the agent's current state, for example a row that \
                 must be set to notInService before it can be changed"
            }
            18 => "the agent cannot create an instance with this index",
            13 => "the agent ran out of a resource it needs to make this change",
            14 | 15 => {
                "the agent failed while making the change; with undoFailed, some of the \
                 values may have been set and others not"
            }
            _ => return None,
        })
    }
}

impl ErrorStatus {
    /// The status for a number as it appears in a response.
    pub fn from_code(code: u32) -> ErrorStatus {
        match code {
            0 => ErrorStatus::NoError,
            1 => ErrorStatus::TooBig,
            2 => ErrorStatus::NoSuchName,
            3 => ErrorStatus::BadValue,
            4 => ErrorStatus::ReadOnly,
            5 => ErrorStatus::GenErr,
            other => ErrorStatus::Other(other),
        }
    }

    /// The number as it appears in a response.
    pub fn code(self) -> u32 {
        match self {
            ErrorStatus::NoError => 0,
            ErrorStatus::TooBig => 1,
            ErrorStatus::NoSuchName => 2,
            ErrorStatus::BadValue => 3,
            ErrorStatus::ReadOnly => 4,
            ErrorStatus::GenErr => 5,
            ErrorStatus::Other(code) => code,
        }
    }

    /// The name used in RFC 3416, such as `noSuchName`.
    pub fn name(self) -> &'static str {
        match self.code() {
            0 => "noError",
            1 => "tooBig",
            2 => "noSuchName",
            3 => "badValue",
            4 => "readOnly",
            5 => "genErr",
            6 => "noAccess",
            7 => "wrongType",
            8 => "wrongLength",
            9 => "wrongEncoding",
            10 => "wrongValue",
            11 => "noCreation",
            12 => "inconsistentValue",
            13 => "resourceUnavailable",
            14 => "commitFailed",
            15 => "undoFailed",
            16 => "authorizationError",
            17 => "notWritable",
            18 => "inconsistentName",
            _ => "unknown error",
        }
    }
}

impl fmt::Display for ErrorStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.name(), self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names() {
        assert_eq!(ErrorStatus::from_code(2).to_string(), "noSuchName(2)");
        assert_eq!(
            ErrorStatus::from_code(16).to_string(),
            "authorizationError(16)"
        );
        assert_eq!(ErrorStatus::from_code(99).to_string(), "unknown error(99)");
        for code in 0..20 {
            assert_eq!(ErrorStatus::from_code(code).code(), code);
        }
    }

    #[test]
    fn timeout_message() {
        let error = Error::Timeout {
            target: "192.0.2.1:161".into(),
            attempts: 3,
            timeout: Duration::from_millis(1500),
        };
        assert!(
            error
                .to_string()
                .starts_with("no response from 192.0.2.1:161 after 3 attempts of 1.5 s each")
        );
    }
}
