//! Errors from SNMP operations.

use std::fmt;
use std::time::Duration;

use mib_core::Oid;

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
         (check the address, the port, the community and any firewall)",
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

    /// GETBULK was requested with SNMPv1, which does not have it.
    #[error("GETBULK needs SNMPv2c; SNMPv1 only has GET and GETNEXT")]
    BulkNeedsV2,

    /// The operation was cancelled before it finished.
    #[error("the operation was cancelled")]
    Cancelled,

    /// The engine has shut down and accepts no more operations.
    #[error("the SNMP engine has stopped")]
    EngineStopped,
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
